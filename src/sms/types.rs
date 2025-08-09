use std::time::Duration;
use anyhow::{anyhow, Result, Error};
use log::debug;
use pdu_rs::pdu::{MessageStatus, PduAddress};
use pdu_rs::gsm_encoding::udh::UserDataHeader;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sqlx::FromRow;
use num_traits::cast::FromPrimitive;
use tokio::time::Instant;

pub type SMSEncryptionKey = [u8; 32];
const MULTIPART_MESSAGES_STALLED_DURATION: Duration = Duration::from_secs(30 * 60); // 30 minutes

#[derive(Serialize, Deserialize, Clone, Debug, sqlx::FromRow)]
pub struct SMSMessage {
    pub message_id: Option<i64>,
    pub phone_number: String,
    pub message_content: String,
    pub message_reference: Option<u8>,
    pub is_outgoing: bool,
    pub status: SMSStatus,
    pub created_at: Option<u64>,
    pub completed_at: Option<u64>
}
impl SMSMessage {

    /// Returns a clone of the message with the message_id option replaced.
    pub fn with_message_id(&self, id: Option<i64>) -> Self {
        SMSMessage {
            message_id: id,
            ..self.clone()
        }
    }
}

#[derive(Debug)]
pub struct SMSOutgoingMessage {
    pub phone_number: PduAddress,
    pub content: String
}
impl From<SMSOutgoingMessage> for SMSMessage {
    fn from(outgoing: SMSOutgoingMessage) -> Self {
        SMSMessage {
            message_id: None,
            phone_number: outgoing.phone_number.to_string(),
            message_content: outgoing.content,
            message_reference: None,
            is_outgoing: true,
            status: SMSStatus::Sent,
            created_at: None,
            completed_at: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SMSMultipartMessages {
    pub total_size: usize,
    pub last_updated: Instant,
    pub first_message: Option<SMSIncomingMessage>,
    pub text_len: usize,
    pub text_parts: Vec<Option<String>>,
    pub received_count: usize,
}
impl SMSMultipartMessages {
    pub fn with_capacity(total_size: usize) -> Self {
        Self {
            total_size,
            last_updated: Instant::now(),
            first_message: None,
            text_len: 0,
            text_parts: vec![None; total_size],
            received_count: 0
        }
    }

    pub fn add_message(&mut self, message: SMSIncomingMessage, index: u8) -> bool {
        self.last_updated = Instant::now();
        if self.first_message.is_none() {
            self.first_message = Some(message.clone());
        }

        // Make multipart index 0-based.
        let idx = (index as usize).saturating_sub(1);
        if idx < self.text_parts.len() && self.text_parts[idx].is_none() {
            self.text_len += message.content.len();
            self.text_parts[idx] = Some(message.content);
            self.received_count += 1;
        }

        debug!("Received Multipart SMS Count: {:?} | Max: {:?}", self.received_count, self.total_size);
        self.received_count >= self.total_size
    }

    pub fn compile(&self) -> Result<SMSMessage> {
        let first_message = match &self.first_message {
            Some(first_message) => first_message,
            None => return Err(anyhow!("Missing required first message to convert into SMSMessage!"))
        };

        let mut content = String::with_capacity(self.text_len);
        for msg_opt in &self.text_parts {
            if let Some(text) = msg_opt {
                content.push_str(&text);
            }
        }

        let mut message = SMSMessage::from(first_message.clone());
        message.message_content = content;

        Ok(message)
    }

    pub fn is_stalled(&self) -> bool {
        self.last_updated.elapsed() > MULTIPART_MESSAGES_STALLED_DURATION
    }
}

#[derive(Debug, Clone)]
pub struct SMSMultipartHeader {
    pub message_reference: u8,
    pub total: u8,
    pub index: u8
}

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
impl From<SMSIncomingMessage> for SMSMessage {
    fn from(incoming: SMSIncomingMessage) -> Self {
        SMSMessage {
            message_id: None,
            phone_number: incoming.phone_number,
            message_content: incoming.content,
            message_reference: None,
            is_outgoing: false,
            status: SMSStatus::Received,
            created_at: None,
            completed_at: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum SMSStatus {
    Sent,
    Delivered,
    Received,
    TemporaryFailure,
    PermanentFailure
}
impl From<&SMSStatus> for u8 {
    fn from(status: &SMSStatus) -> Self {
        match status {
            SMSStatus::Sent => 0,
            SMSStatus::Delivered => 1,
            SMSStatus::Received => 2,
            SMSStatus::TemporaryFailure => 3,
            SMSStatus::PermanentFailure => 4
        }
    }
}
impl From<MessageStatus> for SMSStatus {
    fn from(status: MessageStatus) -> Self {
        if status.is_success() {
            SMSStatus::Received
        } else if status.is_temporary_error() {
            SMSStatus::TemporaryFailure
        } else {
            SMSStatus::PermanentFailure
        }
    }
}
impl TryFrom<u8> for SMSStatus {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(SMSStatus::Sent),
            1 => Ok(SMSStatus::Delivered),
            2 => Ok(SMSStatus::Received),
            3 => Ok(SMSStatus::TemporaryFailure),
            4 => Ok(SMSStatus::PermanentFailure),
            _ => Err(anyhow!("Invalid SMS status value: {}", value))
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
    S: Serializer,
{
    serializer.serialize_u8(*status as u8)
}

fn deserialize_message_status<'de, D>(deserializer: D) -> Result<MessageStatus, D::Error>
where
    D: Deserializer<'de>,
{
    let value = u8::deserialize(deserializer)?;
    MessageStatus::from_u8(value)
        .ok_or_else(|| serde::de::Error::custom(format!("Invalid MessageStatus value: 0x{:02x}", value)))
}

#[derive(Serialize, Deserialize, FromRow)]
pub struct SMSDeliveryReport {
    pub report_id: Option<i64>,
    pub status: u8,
    pub is_final: bool,
    pub created_at: Option<u64>
}
