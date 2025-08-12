pub mod types;
mod database;
mod encryption;

use std::collections::HashMap;
use std::sync::Arc;
use anyhow::{bail, Result};
use tracing::log::{debug, warn};
use pdu_rs::gsm_encoding::GsmMessageData;
use pdu_rs::pdu::{DataCodingScheme, MessageType, PduFirstOctet, SubmitPdu, VpFieldValidity};
use tokio::sync::Mutex;
use crate::config::DatabaseConfig;
use crate::webhooks::{WebhookEvent, WebhookSender};
use crate::modem::sender::ModemSender;
use crate::modem::types::{ModemRequest, ModemResponse};
use crate::sms::database::SMSDatabase;
use crate::sms::types::{SMSIncomingDeliveryReport, SMSIncomingMessage, SMSMessage, SMSMultipartMessages, SMSOutgoingMessage, SMSStatus};

#[derive(Clone)]
pub struct SMSManager {
    modem: ModemSender,
    database: Arc<SMSDatabase>,
    webhooks: Option<WebhookSender>,
}
impl SMSManager {
    pub async fn connect(
        config: DatabaseConfig,
        modem: ModemSender,
        webhooks: Option<WebhookSender>
    ) -> Result<Self> {
        let database = Arc::new(SMSDatabase::connect(config).await?);
        Ok(Self { modem, database, webhooks })
    }

    fn create_requests(message: &SMSOutgoingMessage) -> Result<Vec<ModemRequest>> {
        let requests = GsmMessageData::encode_message(&*message.content)
            .into_iter()
            .map(|data| {
                let pdu = SubmitPdu {
                    sca: None,
                    first_octet: PduFirstOctet {
                        mti: MessageType::SmsSubmit,
                        rd: false,
                        vpf: VpFieldValidity::Relative,
                        srr: true,
                        udhi: data.udh,
                        rp: false
                    },
                    message_id: 0,
                    destination: message.phone_number.clone(),
                    dcs: DataCodingScheme::Standard {
                        compressed: false,
                        class: None,
                        encoding: data.encoding
                    },
                    validity_period: 167,
                    user_data: data.bytes,
                    user_data_len: data.user_data_len,
                };

                let (bytes, size) = pdu.as_bytes();
                ModemRequest::SendSMS {
                    pdu: hex::encode(bytes),
                    len: size
                }
            })
            .collect::<Vec<ModemRequest>>();

        Ok(requests)
    }

    /// Returns the database row ID and final modem response.
    pub async fn send_sms(&self, message: SMSOutgoingMessage) -> Result<(i64, ModemResponse)> {

        // Send each send request for message, returning the last message.
        let mut last_response_opt = None;
        for request in Self::create_requests(&message)? {
            let response = self.modem.send_command(request).await?;
            last_response_opt.replace(response);
        }

        // Ensure there was at least one response back, otherwise nothing was actually sent somehow?
        let last_response = match last_response_opt {
            Some(response) => response,
            None => bail!("Missing any valid SendSMS response!")
        };
        debug!("SMSManager last_response: {:?}", last_response);

        let mut new_message = SMSMessage::from(message);
        let send_failure = match &last_response {
            ModemResponse::SendResult { reference_id } => {
                new_message.message_reference.replace(*reference_id);
                None
            },
            ModemResponse::Error { message: error_message } => {
                new_message.status = SMSStatus::PermanentFailure;
                Some(error_message)
            },
            _ => bail!("Got invalid ModemResponse back from sending SMS message!")
        };

        // Store sent message + send failure in database.
        let message_id_result = match self.database.insert_message(&new_message, send_failure.is_some()).await {
            Ok(row_id) => {
                if let Some(failure) = send_failure {
                    let _ = self.database.insert_send_failure(row_id, failure);
                }
                Ok(row_id)
            },
            Err(e) => Err(e)
        };

        // Send outgoing message webhook event.
        if let Some(webhooks) = &self.webhooks {
            webhooks.send(WebhookEvent::OutgoingMessage(
                new_message.with_message_id(message_id_result.as_ref().ok().copied())
            ));
        }

        match message_id_result {
            Ok(message_id) => Ok((message_id, last_response)),
            Err(e) => Err(e)
        }
    }

    pub async fn send_command(&self, request: ModemRequest) -> Result<ModemResponse> {
        self.modem.send_command(request).await
    }

    pub fn borrow_database(&self) -> &Arc<SMSDatabase> {
        &self.database
    }
}

#[derive(Clone)]
pub struct SMSReceiver {
    manager: SMSManager,
    multipart: Arc<Mutex<HashMap<u8, SMSMultipartMessages>>>
}
impl SMSReceiver {
    pub fn new(manager: SMSManager) -> Self {
        Self { manager, multipart: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub async fn handle_incoming_sms(&mut self, incoming_message: SMSIncomingMessage) -> Option<Result<i64>> {

        // Handle incoming message, discarding if it's a multipart message and not final.
        let message = match self.get_incoming_sms_message(incoming_message).await {
            Some(Ok(message)) => message,
            Some(Err(e)) => return Some(Err(e)),
            None => return None
        };

        let row_id = self.manager.database.insert_message(&message, false).await;

        // Send incoming message webhook event.
        if let Some(webhooks) = &self.manager.webhooks {
            webhooks.send(WebhookEvent::IncomingMessage(
                message.with_message_id(row_id.as_ref().ok().copied())
            ));
        }

        Some(row_id)
    }

    pub async fn handle_delivery_report(&self, report: SMSIncomingDeliveryReport) -> Result<i64> {

        // Find the target message from phone number and message reference. This will be fine unless we send 255
        // messages to the client before they reply with delivery reports as then there's no way to properly track.
        let message_id = match self.manager.database.get_delivery_report_target_message(&report.phone_number, report.reference_id).await? {
            Some(message_id) => message_id,
            None => bail!("Could not find target message for delivery report!")
        };

        let is_final = report.status.is_success() || report.status.is_permanent_error();
        let status = u8::from(&SMSStatus::from(report.status));

        // Send delivery report webhook event.
        let sms_status = SMSStatus::from(report.status);
        if let Some(webhooks) = &self.manager.webhooks {
            webhooks.send(WebhookEvent::DeliveryReport {
                message_id,
                report
            })
        }

        self.manager.database.insert_delivery_report(message_id, status, is_final).await?;
        self.manager.database.update_message_status(message_id, &sms_status, is_final).await?;

        Ok(message_id)
    }

    pub async fn cleanup_stalled_multipart(&mut self) {
        debug!("Cleaning up stalled multipart messages.");
        let mut guard = self.multipart.lock().await;
        guard.retain(|message_reference, messages| {

            // Show a warning whenever a message group has stalled.
            let stalled = messages.is_stalled();
            if stalled {
                warn!("Removing received multipart message #{} has stalled!", message_reference);
            }
            stalled
        });
    }

    async fn get_incoming_sms_message(&mut self, incoming_message: SMSIncomingMessage) -> Option<Result<SMSMessage>> {

        // Decode the message data header to get multipart header.
        let header = match incoming_message.decode_multipart_data() {
            Some(Ok(header)) => header,
            Some(Err(e)) => return Some(Err(e)),
            None => return Some(Ok(SMSMessage::from(incoming_message)))
        };

        // Get multipart messages set for message reference.
        let mut guard = self.multipart.lock().await;
        let multipart = guard.entry(header.message_reference)
            .or_insert_with(|| SMSMultipartMessages::with_capacity(header.total as usize));

        // Add partial message, if it's full then return the compiled message.
        // Otherwise, nothing is returned as there is no message to store.
        match multipart.add_message(incoming_message, header.index) {
            true => Some(multipart.compile()),
            false => None
        }
    }
}