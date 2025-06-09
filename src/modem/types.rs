use std::fmt::{Display, Formatter};
use serde::{Deserialize, Serialize};
use crate::modem::commands::CommandContext;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModemRequest {
    SendSMS { len: u64, pdu: String },
    GetNetworkStatus,
    GetSignalStrength
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ModemResponse {
    SendResult { message_id: String, status: String },
    NetworkStatus {
        operator: String
    },
    SignalStrength {
        rssi: i32,
        ber: i32,
        quality: String
    },
    Error { message: String }
}
impl Display for ModemResponse {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SendResult { message_id, status } =>
                write!(f, "SMSResult: {} -> {}", message_id, status),
            Self::NetworkStatus { operator } =>
                write!(f, "NetworkStatus: {}", operator),
            Self::SignalStrength { rssi, quality, .. } =>
                write!(f, "SignalStrength: {} dBm ({})", rssi, quality),
            Self::Error { message } =>
                write!(f, "Error: {}", message)
        }
    }
}

#[derive(Debug)]
pub enum ModemEvent {
    UnsolicitedNotification(String),
    CommandResponse(String),
    Data(String),
    Prompt(String),
}

#[derive(Debug, Clone)]
pub struct ModemConfig {
    pub device: &'static str,
    pub baud: u32
}

#[derive(Debug)]
pub enum ModemReadState {
    Idle,
    Command(CommandContext),
    UnsolicitedCmt {
        header: String,
        active_command: Option<CommandContext>
    }
}

#[derive(Debug)]
pub enum SMSStatus {
    Pending,
    Sent,
    Failed,
    Received
}

#[derive(Debug)]
pub struct ReceivedSMSMessage {
    pub id: String,
    pub from: String,
    pub to: String,
    pub content: String,
    pub timestamp: u64,
    pub status: SMSStatus
}