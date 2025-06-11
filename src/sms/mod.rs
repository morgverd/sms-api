pub mod types;
mod database;
mod encryption;

use std::str::FromStr;
use std::sync::Arc;
use anyhow::{bail, Result};
use huawei_modem::gsm_encoding::GsmMessageData;
use huawei_modem::pdu::{Pdu, PduAddress};
use log::info;
use crate::modem::sender::ModemSender;
use crate::modem::types::{ModemRequest, ModemResponse};
use crate::sms::database::SMSDatabase;
use crate::sms::types::{SMSEncryptionKey, SMSIncomingMessage, SMSMessage, SMSOutgoingMessage, SMSStatus};

#[derive(Clone)]
pub struct SMSManager {
    modem: ModemSender,
    database: Arc<SMSDatabase>
}
impl SMSManager {
    pub async fn new(modem: ModemSender, database_url: &str, encryption_key: SMSEncryptionKey) -> Result<Self> {
        let database = Arc::new(SMSDatabase::connect(database_url, encryption_key).await?);
        Ok(Self { modem, database })
    }

    /// Returns the database row ID and final modem response.
    /// https://github.com/eeeeeta/huawei-modem/issues/24
    pub async fn send_sms(&self, message: SMSOutgoingMessage) -> Result<(i64, ModemResponse)> {
        let mut last_response_opt = None;
        for part in GsmMessageData::encode_message(&*message.content) {

            // FIXME: This is horrendous, the address is being re-parsed for each split message
            //  because the PDU lib doesn't allow a PduFirstOctet to be directly initialized.
            let address = PduAddress::from_str(&*message.phone_number)?;
            let pdu = Pdu::make_simple_message(address, part);

            let (bytes, size) = pdu.as_bytes();
            let request = ModemRequest::SendSMS {
                pdu: hex::encode(bytes),
                len: size,
            };
            last_response_opt = Some(self.modem.send_command(request).await?);
        }

        // Ensure there was at least one response back, otherwise nothing was actually sent somehow?
        let last_response = match last_response_opt {
            Some(response) => response,
            None => bail!("Missing any valid SendSMS response!")
        };
        info!("SMSManager last_response: {:?}", last_response);

        let (storage_message, error_message) = match &last_response {
            ModemResponse::SendResult { reference_id } => {
                let mut new_message = SMSMessage::from(message);
                new_message.message_reference.replace(*reference_id);
                (new_message, None)
            },
            ModemResponse::Error { message: error_message } => {
                let mut new_message = SMSMessage::from(message);
                new_message.status = SMSStatus::Failed;
                (new_message, Some(error_message))
            },
            _ => bail!("Got invalid ModemResponse back from sending SMS message!")
        };

        // Store sent message + send failure in database.
        let message_id = self.database.insert_message(storage_message).await?;
        if let Some(error_message) = error_message {
            let _ = self.database.insert_send_failure(message_id, error_message.to_owned());
        }

        Ok((message_id, last_response))
    }

    pub async fn accept_incoming(&self, message: SMSIncomingMessage) -> Result<i64> {
        self.database.insert_message(SMSMessage::from(message)).await
    }

    pub async fn send_command(&self, request: ModemRequest) -> Result<ModemResponse> {
        self.modem.send_command(request).await
    }
}