use std::fmt::{Display, Formatter};
use std::time::Duration;
use serde::{Deserialize, Serialize};
use crate::sms::types::{SMSIncomingDeliveryReport, SMSIncomingMessage};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModemRequest {
    SendSMS {
        len: usize,
        pdu: String
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
#[serde(tag = "type", content = "data")]
pub enum ModemResponse {
    SendResult {
        reference_id: u8
    },
    NetworkStatus {
        registration: u8,
        technology: u8
    },
    SignalStrength {
        rssi: i32,
        ber: i32
    },
    NetworkOperator {
        status: u8,
        format: u8,
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
            ModemResponse::NetworkStatus { registration, technology } =>
                write!(f, "NetworkStatus: Reg: {}, Tech: {}", registration, technology),
            ModemResponse::SignalStrength { rssi, ber } =>
                write!(f, "SignalStrength: {} dBm ({})", rssi, ber),
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

#[derive(Debug)]
pub enum UnsolicitedMessageType {
    IncomingSMS,
    DeliveryReport,
    NetworkStatusChange,
    ShuttingDown
}
impl UnsolicitedMessageType {
    pub fn from_header(header: &str) -> Option<Self> {
        if header.starts_with("+CMT") {
            Some(UnsolicitedMessageType::IncomingSMS)
        } else if header.starts_with("+CDS") {
            Some(UnsolicitedMessageType::DeliveryReport)
        } else if header.starts_with("+CGREG:") {
            Some(UnsolicitedMessageType::NetworkStatusChange)
        } else {
            match header {
                "NORMAL POWER DOWN" | "POWER DOWN" | "SHUTDOWN" | "POWERING DOWN" => {
                    Some(UnsolicitedMessageType::ShuttingDown)
                },
                _ => None
            }
        }
    }
}

#[derive(Debug)]
pub enum ModemIncomingMessage {
    IncomingSMS(SMSIncomingMessage),
    DeliveryReport(SMSIncomingDeliveryReport),
    NetworkStatusChange {
        status: u8
    },
}