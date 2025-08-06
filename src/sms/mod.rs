pub mod types;
pub mod webhooks;
mod database;
mod encryption;

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use anyhow::{anyhow, bail, Result};
use log::debug;
use pdu_rs::gsm_encoding::GsmMessageData;
use pdu_rs::pdu::{DataCodingScheme, MessageType, PduAddress, PduFirstOctet, SubmitPdu, TypeOfNumber, VpFieldValidity};
use crate::config::DatabaseConfig;
use crate::modem::sender::ModemSender;
use crate::modem::types::{ModemRequest, ModemResponse};
use crate::sms::database::SMSDatabase;
use crate::sms::types::{SMSIncomingDeliveryReport, SMSIncomingMessage, SMSMessage, SMSMultipartMessages, SMSOutgoingMessage, SMSStatus, WebhookEvent};
use crate::sms::webhooks::SMSWebhookManager;

#[derive(Clone)]
pub struct SMSManager {
    modem: ModemSender,
    database: Arc<SMSDatabase>,
    webhooks: Option<SMSWebhookManager>,
}
impl SMSManager {
    pub async fn connect(
        config: DatabaseConfig,
        modem: ModemSender,
        webhooks: Option<SMSWebhookManager>
    ) -> Result<Self> {
        let database = Arc::new(SMSDatabase::connect(config).await?);
        Ok(Self { modem, database, webhooks })
    }

    fn create_requests(message: &SMSOutgoingMessage) -> Result<Vec<ModemRequest>> {
        let address = PduAddress::from_str(&*message.phone_number)?;

        /// TODO: Re-add this.
        // if !matches!(address.type_addr.type_of_number, TypeOfNumber::International) {
        //     return Err(anyhow!("Sending phone number must be in international format!"));
        // }

        let requests = GsmMessageData::encode_message(&*message.content)
            .into_iter()
            .map(|message| {
                let pdu = SubmitPdu {
                    sca: None,
                    first_octet: PduFirstOctet {
                        mti: MessageType::SmsSubmit,
                        rd: false,
                        vpf: VpFieldValidity::Relative,
                        srr: true,
                        udhi: message.udh,
                        rp: false,
                    },
                    message_id: 0,
                    destination: address.clone(),
                    dcs: DataCodingScheme::Standard {
                        compressed: false,
                        class: None,
                        encoding: message.encoding
                    },
                    validity_period: 167,
                    user_data: message.bytes,
                    user_data_len: message.user_data_len,
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
    /// https://github.com/eeeeeta/huawei-modem/issues/24
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
    manager: Arc<SMSManager>,
    multipart: HashMap<u8, SMSMultipartMessages>
}
impl SMSReceiver {
    pub fn new(manager: Arc<SMSManager>) -> Self {
        Self { manager, multipart: HashMap::new() }
    }

    pub async fn handle_incoming_sms(&mut self, incoming_message: SMSIncomingMessage) -> Option<Result<i64>> {

        // Handle incoming message, discarding if it's a multipart message and not final.
        let message = match self.get_incoming_sms_message(incoming_message) {
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

    fn get_incoming_sms_message(&mut self, incoming_message: SMSIncomingMessage) -> Option<Result<SMSMessage>> {

        // Decode the message data header to get multipart header.
        let header = match incoming_message.decode_multipart_data() {
            Some(Ok(header)) => header,
            Some(Err(e)) => return Some(Err(e)),
            None => return Some(Ok(SMSMessage::from(incoming_message)))
        };

        // Get multipart messages set for message reference.
        let multipart = self.multipart.entry(header.message_reference)
            .or_insert_with(|| SMSMultipartMessages::with_capacity(header.total as usize));

        let is_full = multipart.add_message(incoming_message, header.index);
        (is_full || header.index >= header.total)
            .then(|| multipart.compile())
            .map(|result| result.map_err(|_| anyhow!("Failed to convert final multipart SMS message!")))
    }
}