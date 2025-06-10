use std::fmt::{Display, Formatter};
use serde::{Deserialize, Serialize};
use crate::modem::commands::{CommandContext, CommandState};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModemRequest {
    SendSMS { len: u64, pdu: String },
    GetNetworkStatus,
    GetSignalStrength,
    GetNetworkOperator,
    GetServiceProvider,
    GetBatteryLevel
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
    NetworkOperator {
        operator: String
    },
    ServiceProvider {
        operator: String
    },
    BatteryLevel {
        status: u8,
        charge: u8,
        voltage: f32
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
            Self::NetworkOperator { operator, .. } =>
                write!(f, "NetworkOperator: {}", operator),
            Self::ServiceProvider { operator, .. } =>
                write!(f, "ServiceProvider: {}", operator),
            Self::BatteryLevel { status, charge, voltage } =>
                write!(f, "BatteryLevel. Status: {}, Charge: {}, Voltage: {}", status, charge, voltage),
            Self::Error { message } =>
                write!(f, "Error: {}", message)
        }
    }
}

#[derive(Debug)]
pub enum ModemEvent {
    UnsolicitedMessage {
        message_type: UnsolicitedMessageType,
        header: String
    },
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
pub enum UnsolicitedMessageType {
    IncomingSMS,
    IncomingCall,
    DeliveryReport,
    NetworkStatusChange
}
impl UnsolicitedMessageType {
    pub fn from_header(header: &str) -> Option<Self> {
        if header.starts_with("+CMT") {
            Some(Self::IncomingSMS)
        } else if header.starts_with("+RING") {
            Some(Self::IncomingCall)
        } else if header.starts_with("+CDS") {
            Some(Self::DeliveryReport)
        } else if header.starts_with("+CGREG:") {
            Some(Self::NetworkStatusChange)
        } else {
            None
        }
    }
}

#[derive(Debug, Default)]
pub enum ModemReadState {
    #[default] Idle,
    Command(CommandContext),
    UnsolicitedMessage {
        message_type: UnsolicitedMessageType,
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
pub enum ModemIncomingMessage {
    IncomingSMS {
        id: String,
        to: String,
        content: String,
        timestamp: u64
    },
    IncomingCall,
    DeliveryReport {
        id: String
    },
    NetworkStatusChange {
        status: u8
    },
}