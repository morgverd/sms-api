use std::fmt::{Display, Formatter};
use std::time::Duration;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModemRequest {
    SendSMS {
        pdu: String,
        len: usize
    },
    GetNetworkStatus,
    GetSignalStrength,
    GetNetworkOperator,
    GetServiceProvider,
    GetBatteryLevel
}
impl ModemRequest {
    pub fn get_timeout(&self) -> Duration {
        match self {
            ModemRequest::SendSMS { .. } => Duration::from_secs(20),
            _ => Duration::from_secs(5)
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ModemResponse {
    SendResult {
        reference_id: u8
    },
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
    Error {
        message: String
    }
}
impl Display for ModemResponse {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ModemResponse::SendResult { reference_id } =>
                write!(f, "SMSResult: Ref {}", reference_id),
            ModemResponse::NetworkStatus { operator } =>
                write!(f, "NetworkStatus: {}", operator),
            ModemResponse::SignalStrength { rssi, quality, .. } =>
                write!(f, "SignalStrength: {} dBm ({})", rssi, quality),
            ModemResponse::NetworkOperator { operator, .. } =>
                write!(f, "NetworkOperator: {}", operator),
            ModemResponse::ServiceProvider { operator, .. } =>
                write!(f, "ServiceProvider: {}", operator),
            ModemResponse::BatteryLevel { status, charge, voltage } =>
                write!(f, "BatteryLevel. Status: {}, Charge: {}, Voltage: {}", status, charge, voltage),
            ModemResponse::Error { message } =>
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
    pub baud: u32,
    
    /// The size of Command bounded mpsc sender, should be low. eg: 32
    pub cmd_channel_buffer_size: usize
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
            Some(UnsolicitedMessageType::IncomingSMS)
        } else if header.starts_with("+RING") {
            Some(UnsolicitedMessageType::IncomingCall)
        } else if header.starts_with("+CDS") {
            Some(UnsolicitedMessageType::DeliveryReport)
        } else if header.starts_with("+CGREG:") {
            Some(UnsolicitedMessageType::NetworkStatusChange)
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub enum ModemIncomingMessage {
    IncomingSMS {
        phone_number: String,
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