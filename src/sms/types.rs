use anyhow::{anyhow, Result};
use sms_pdu::pdu::MessageStatus;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use num_traits::cast::FromPrimitive;
use sms_pdu::gsm_encoding::udh::UserDataHeader;
use crate::sms::multipart::SMSMultipartHeader;
use crate::types::{SMSMessage, SMSStatus};

pub type SMSEncryptionKey = [u8; 32];

#[derive(Debug, Clone)]
pub struct SMSIncomingMessage {
    pub phone_number: String,
    pub user_data_header: Option<UserDataHeader>,
    pub content: String
}
impl SMSIncomingMessage {
    pub fn decode_multipart_data(&self) -> Option<Result<SMSMultipartHeader>> {

        // Find header component with multipart ID.
        let component = self.user_data_header.as_ref()?
            .components
            .iter()
            .find(|c| c.id == 0x00)?;

        if component.data.len() != 3 {
            return Some(Err(anyhow!("Invalid multipart header length!")));
        }

        Some(Ok(SMSMultipartHeader {
            message_reference: component.data[0],
            total: component.data[1],
            index: component.data[2]
        }))
    }
}
impl From<&SMSIncomingMessage> for SMSMessage {
    fn from(incoming: &SMSIncomingMessage) -> Self {
        SMSMessage {
            message_id: None,
            phone_number: incoming.phone_number.clone(),
            message_content: incoming.content.clone(),
            message_reference: None,
            is_outgoing: false,
            status: SMSStatus::Received,
            created_at: None,
            completed_at: None,
        }
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SMSIncomingDeliveryReport {
    pub phone_number: String,
    pub reference_id: u8,

    #[serde(serialize_with = "serialize_message_status")]
    #[serde(deserialize_with = "deserialize_message_status")]
    pub status: MessageStatus
}

fn serialize_message_status<S>(status: &MessageStatus, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer
{
    serializer.serialize_u8(*status as u8)
}

fn deserialize_message_status<'de, D>(deserializer: D) -> Result<MessageStatus, D::Error>
where
    D: Deserializer<'de>
{
    let value = u8::deserialize(deserializer)?;
    MessageStatus::from_u8(value)
        .ok_or_else(|| serde::de::Error::custom(format!("Invalid MessageStatus value: 0x{:02x}", value)))
}
