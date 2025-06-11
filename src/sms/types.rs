use anyhow::{anyhow, Error};

pub type SMSEncryptionKey = [u8; 32];

#[derive(Debug, sqlx::FromRow)]
pub struct SMSMessage {
    pub id: Option<i64>,
    pub phone_number: String,
    pub message_content: String,
    pub message_reference: Option<u8>,
    pub is_outgoing: bool,
    pub status: SMSStatus,
    pub created_at: Option<String>, // TODO: Use chrono DateTime
    pub updated_at: Option<String>
}

#[derive(Debug)]
pub struct SMSOutgoingMessage {
    pub phone_number: String,
    pub content: String
}
impl From<SMSOutgoingMessage> for SMSMessage {
    fn from(outgoing: SMSOutgoingMessage) -> Self {
        SMSMessage {
            id: None,
            phone_number: outgoing.phone_number,
            message_content: outgoing.content,
            message_reference: None,
            is_outgoing: true,
            status: SMSStatus::Sent,
            created_at: None,
            updated_at: None,
        }
    }
}

#[derive(Debug)]
pub struct SMSIncomingMessage {
    pub phone_number: String,
    pub content: String
}
impl From<SMSIncomingMessage> for SMSMessage {
    fn from(incoming: SMSIncomingMessage) -> Self {
        SMSMessage {
            id: None,
            phone_number: incoming.phone_number,
            message_content: incoming.content,
            message_reference: None,
            is_outgoing: false,
            status: SMSStatus::Received,
            created_at: None,
            updated_at: None,
        }
    }
}

#[derive(Debug)]
pub enum SMSStatus {
    Sent,
    Delivered,
    Received,
    Failed
}
impl From<SMSStatus> for u8 {
    fn from(status: SMSStatus) -> Self {
        match status {
            SMSStatus::Sent => 0,
            SMSStatus::Delivered => 1,
            SMSStatus::Received => 2,
            SMSStatus::Failed => 3
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
            3 => Ok(SMSStatus::Failed),
            _ => Err(anyhow!("Invalid SMS status value: {}", value))
        }
    }
}