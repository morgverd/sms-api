use anyhow::{anyhow, Error};
use pdu_rs::pdu::MessageStatus;
use serde::Serialize;

pub type SMSEncryptionKey = [u8; 32];

#[derive(Serialize, Debug, sqlx::FromRow)]
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

#[derive(Serialize, Debug)]
pub enum SMSStatus {
    Sent,
    Delivered,
    Received,
    TemporaryFailure,
    PermanentFailure
}
impl From<SMSStatus> for u8 {
    fn from(status: SMSStatus) -> Self {
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

pub struct SMSIncomingDeliveryReport {
    pub status: MessageStatus,
    pub phone_number: String,
    pub reference_id: u8
}