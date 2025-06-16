pub mod types;
mod database;
mod encryption;

use std::str::FromStr;
use std::sync::Arc;
use anyhow::{anyhow, bail, Result};
use log::debug;
use pdu_rs::gsm_encoding::GsmMessageData;
use pdu_rs::pdu::{DataCodingScheme, MessageType, PduAddress, PduFirstOctet, SubmitPdu, TypeOfNumber, VpFieldValidity};
use crate::config::SMSConfig;
use crate::modem::sender::ModemSender;
use crate::modem::types::{ModemRequest, ModemResponse};
use crate::sms::database::SMSDatabase;
use crate::sms::types::{SMSIncomingDeliveryReport, SMSIncomingMessage, SMSMessage, SMSOutgoingMessage, SMSStatus};

#[derive(Clone)]
pub struct SMSManager {
    modem: ModemSender,
    database: Arc<SMSDatabase>
}
impl SMSManager {
    pub async fn new(config: SMSConfig, modem: ModemSender) -> Result<Self> {
        let database = Arc::new(SMSDatabase::connect(config).await?);
        Ok(Self { modem, database })
    }

    fn create_requests(message: &SMSOutgoingMessage) -> Result<Vec<ModemRequest>> {
        let address = PduAddress::from_str(&*message.phone_number)?;
        if !matches!(address.type_addr.type_of_number, TypeOfNumber::International) {
            return Err(anyhow!("Sending phone number must be in international format!"));
        }

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

        let (storage_message, send_failure) = match &last_response {
            ModemResponse::SendResult { reference_id } => {
                let mut new_message = SMSMessage::from(message);
                new_message.message_reference.replace(*reference_id);
                (new_message, None)
            },
            ModemResponse::Error { message: error_message } => {
                let mut new_message = SMSMessage::from(message);
                new_message.status = SMSStatus::PermanentFailure;
                (new_message, Some(error_message))
            },
            _ => bail!("Got invalid ModemResponse back from sending SMS message!")
        };

        // Store sent message + send failure in database.
        let message_id = self.database.insert_message(storage_message, send_failure.is_some()).await?;
        if let Some(error_message) = send_failure {
            let _ = self.database.insert_send_failure(message_id, error_message.to_owned());
        }

        Ok((message_id, last_response))
    }

    pub async fn handle_incoming_sms(&self, message: SMSIncomingMessage) -> Result<i64> {
        self.database.insert_message(SMSMessage::from(message), false).await
    }

    pub async fn handle_delivery_report(&self, report: SMSIncomingDeliveryReport) -> Result<i64> {

        // Find the target message from phone number and message reference. This will be fine unless we send 255
        // messages to the client before they reply with delivery reports as then there's no way to properly track.
        let message_id = match self.database.get_delivery_report_target_message(report.phone_number, report.reference_id).await? {
            Some(message_id) => message_id,
            None => bail!("Could not find target message for delivery report!")
        };

        let is_final = report.status.is_success() || report.status.is_permanent_error();
        let status = u8::from(SMSStatus::from(report.status));

        self.database.insert_delivery_report(message_id, status, is_final).await?;
        self.database.update_message_status(message_id, report.status.into(), is_final).await?;

        Ok(message_id)
    }

    pub async fn send_command(&self, request: ModemRequest) -> Result<ModemResponse> {
        self.modem.send_command(request).await
    }

    pub fn borrow_database(&self) -> &Arc<SMSDatabase> {
        &self.database
    }
}