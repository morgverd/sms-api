use std::fmt::{Display, Formatter};
use std::time::Duration;
use anyhow::{anyhow, Context};
use serde::Serialize;
use crate::sms::types::{SMSIncomingDeliveryReport, SMSIncomingMessage};

#[derive(Debug, Clone)]
pub enum ModemRequest {
    SendSMS {
        len: usize,
        pdu: String
    },
    GetNetworkStatus,
    GetSignalStrength,
    GetNetworkOperator,
    GetServiceProvider,
    GetBatteryLevel,

    // These only work if GNSS is enabled in modem config.
    GetGNSSStatus,
    GetGNSSLocation
}
impl ModemRequest {
    pub fn get_timeout(&self) -> Duration {
        match self {
            ModemRequest::SendSMS { .. } => Duration::from_secs(20),
            _ => Duration::from_secs(5)
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum ModemResponse {
    SendResult(u8),
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
    ServiceProvider(String),
    BatteryLevel {
        status: u8,
        charge: u8,
        voltage: f32
    },
    GNSSStatus(GNSSFixStatus),
    GNSSLocation(GNSSLocation),
    Error(String)
}
impl Display for ModemResponse {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ModemResponse::SendResult(reference_id) =>
                write!(f, "SMSResult: Ref {}", reference_id),
            ModemResponse::NetworkStatus { registration, technology } =>
                write!(f, "NetworkStatus: Reg: {}, Tech: {}", registration, technology),
            ModemResponse::SignalStrength { rssi, ber } =>
                write!(f, "SignalStrength: {} dBm ({})", rssi, ber),
            ModemResponse::NetworkOperator { operator, .. } =>
                write!(f, "NetworkOperator: {}", operator),
            ModemResponse::ServiceProvider(operator) =>
                write!(f, "ServiceProvider: {}", operator),
            ModemResponse::BatteryLevel { status, charge, voltage } =>
                write!(f, "BatteryLevel. Status: {}, Charge: {}, Voltage: {}", status, charge, voltage),
            ModemResponse::GNSSStatus(status) =>
                write!(f, "GNSS-Status: {:?}", status),
            ModemResponse::GNSSLocation(location) =>
                write!(f, "GNSS-Location: {:?}", location),
            ModemResponse::Error(message) =>
                write!(f, "Error: {}", message)
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum ModemStatus {
    Startup,
    Online,
    ShuttingDown,
    Offline
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

    /// Check if the notification contains additional data on a new line.
    pub fn has_next_line(&self) -> bool {
        match self {
            UnsolicitedMessageType::ShuttingDown => false,
            _ => true
        }
    }
}

#[derive(Debug, Clone)]
pub enum ModemIncomingMessage {
    IncomingSMS(SMSIncomingMessage),
    DeliveryReport(SMSIncomingDeliveryReport),
    ModemStatusUpdate {
        previous: ModemStatus,
        current: ModemStatus
    },
    NetworkStatusChange(u8)
}

#[derive(Debug, Serialize)]
pub enum GNSSFixStatus {
    Unknown,
    NotFix,
    Fix2D,
    Fix3D
}
impl TryFrom<&str> for GNSSFixStatus {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value.trim() {
            "Location Unknown" | "Unknown" => Ok(GNSSFixStatus::Unknown),
            "Location Not Fix" | "Not Fix" => Ok(GNSSFixStatus::NotFix),
            "Location 2D Fix"  | "2D Fix" => Ok(GNSSFixStatus::Fix2D),
            "Location 3D Fix"  | "3D Fix" => Ok(GNSSFixStatus::Fix3D),
            _ => Err(anyhow!("Invalid GNSS fix status: '{}'", value))
        }
    }
}

#[derive(Debug, Serialize)]
pub struct GNSSLocation {
    longitude: DirectionalCoordinate,
    latitude: DirectionalCoordinate,
    altitude: f32,
    utc_time: u32,
    satellites_used: u8,
    hdop: f32,
    geoid_separation: f32,
    position_fix_indicator: u8
}
impl TryFrom<Vec<&str>> for GNSSLocation {
    type Error = anyhow::Error;

    fn try_from(fields: Vec<&str>) -> Result<Self, Self::Error> {
        let parse_optional_f32 = |s: &str| -> anyhow::Result<f32> {
            if s.is_empty() {
                Ok(0.0)
            } else {
                s.parse().map_err(|_| anyhow!("Invalid number format: '{}'", s))
            }
        };
        let parse_optional_u8 = |s: &str| -> anyhow::Result<u8> {
            if s.is_empty() {
                Ok(0)
            } else {
                s.parse().map_err(|_| anyhow!("Invalid number format: '{}'", s))
            }
        };
        let parse_coordinate = |coord: &str, dir: &str, name: &'static str| -> anyhow::Result<DirectionalCoordinate> {
            Ok((
                coord.parse().with_context(|| format!("Could not parse {} coordinate: {}", name, coord))?,
                dir.parse().with_context(|| format!("Invalid {} coordinate direction: {}", name, dir))?,
            ))
        };

        Ok(Self {
            longitude: parse_coordinate(fields[2], fields[3], "longitude")?,
            latitude: parse_coordinate(fields[4], fields[5], "latitude")?,
            altitude: parse_optional_f32(fields[9])?,
            utc_time: fields[1].parse::<f64>().unwrap_or(0.0) as u32,
            satellites_used: parse_optional_u8(fields[7])?,
            hdop: parse_optional_f32(fields[8])?,
            geoid_separation: parse_optional_f32(fields[11])?,
            position_fix_indicator: parse_optional_u8(fields[6])?,
        })
    }
}

/// Coordinate value, Compass direction (N,S,W,E)
pub type DirectionalCoordinate = (f64, char);