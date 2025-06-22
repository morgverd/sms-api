use anyhow::{anyhow, Error};
use pdu_rs::pdu::MessageStatus;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sqlx::FromRow;
use num_traits::cast::FromPrimitive;
use crate::config::ConfiguredWebhookEvent;

pub type SMSEncryptionKey = [u8; 32];

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
    pub phone_number: String,
    pub content: String
}
impl From<SMSOutgoingMessage> for SMSMessage {
    fn from(outgoing: SMSOutgoingMessage) -> Self {
        SMSMessage {
            message_id: None,
            phone_number: outgoing.phone_number,
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
pub struct SMSIncomingMessage {
    pub phone_number: String,
    pub content: String
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WebhookEvent {
    IncomingMessage(SMSMessage),
    OutgoingMessage(SMSMessage),
    DeliveryReport {
        message_id: i64,
        report: SMSIncomingDeliveryReport
    }
}
impl WebhookEvent {

    #[inline]
    pub fn to_configured_event(&self) -> ConfiguredWebhookEvent {
        match self {
            WebhookEvent::IncomingMessage(_) => ConfiguredWebhookEvent::IncomingMessage,
            WebhookEvent::OutgoingMessage(_) => ConfiguredWebhookEvent::OutgoingMessage,
            WebhookEvent::DeliveryReport { .. } => ConfiguredWebhookEvent::DeliveryReport
        }
    }
}
