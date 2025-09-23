use anyhow::{anyhow, Error};
use serde::{Deserialize, Serialize};
use sms_pdu::pdu::{MessageStatus, PduAddress};
use sqlx::FromRow;

#[derive(Serialize, Deserialize, Clone, Debug, FromRow)]
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
    pub content: String,
    pub flash: bool,
    pub validity_period: Option<u8>,
    pub timeout: Option<u32>
}
impl SMSOutgoingMessage {
    pub fn get_validity_period(&self) -> u8 {
        self.validity_period.unwrap_or(167) // 24hr
    }
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
            completed_at: None
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

    fn try_from(value: u8) -> anyhow::Result<Self, Self::Error> {
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

#[derive(Serialize, Deserialize, FromRow)]
pub struct SMSDeliveryReport {
    pub report_id: Option<i64>,
    pub status: u8,
    pub is_final: bool,
    pub created_at: Option<u64>
}