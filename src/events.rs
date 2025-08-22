use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tracing::log::debug;
use crate::config::ConfiguredWebhook;
use crate::http::websocket::WebSocketManager;
use crate::modem::types::{GNSSLocation, ModemStatus};
use crate::sms::types::{SMSIncomingDeliveryReport, SMSMessage};
use crate::webhooks::WebhookSender;

#[derive(Eq, PartialEq, Hash, Debug, Clone, Copy, Deserialize)]
pub enum EventType {
    #[serde(rename = "incoming")]
    IncomingMessage,

    #[serde(rename = "outgoing")]
    OutgoingMessage,

    #[serde(rename = "delivery")]
    DeliveryReport,

    #[serde(rename = "modem_status_update")]
    ModemStatusUpdate,

    #[serde(rename = "gnss_position_report")]
    GNSSPositionReport
}
impl EventType {
    pub const fn to_bit(self) -> u8 {
        match self {
            EventType::IncomingMessage => 1 << 0,     // 0b00001
            EventType::OutgoingMessage => 1 << 1,     // 0b00010
            EventType::DeliveryReport => 1 << 2,      // 0b00100
            EventType::ModemStatusUpdate => 1 << 3,   // 0b01000
            EventType::GNSSPositionReport => 1 << 4,  // 0b10000
        }
    }

    pub const fn all_bits() -> u8 {
        (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4) // 0b11111
    }

    pub fn events_to_mask(events: &[EventType]) -> u8 {
        events.iter().fold(0, |acc, event| acc | event.to_bit())
    }
}
impl TryFrom<&str> for EventType {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "incoming" => Ok(EventType::IncomingMessage),
            "outgoing" => Ok(EventType::OutgoingMessage),
            "delivery" => Ok(EventType::DeliveryReport),
            "modem_status_update" => Ok(EventType::ModemStatusUpdate),
            "gnss_position_report" => Ok(EventType::GNSSPositionReport),
            _ => Err(anyhow!("Unknown event type {}", value))
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum Event {
    #[serde(rename = "incoming")]
    IncomingMessage(SMSMessage),

    #[serde(rename = "outgoing")]
    OutgoingMessage(SMSMessage),

    #[serde(rename = "delivery")]
    DeliveryReport {
        message_id: i64,
        report: SMSIncomingDeliveryReport
    },

    #[serde(rename = "modem_status_update")]
    ModemStatusUpdate {
        previous: ModemStatus,
        current: ModemStatus
    },

    #[serde(rename = "gnss_position_report")]
    GNSSPositionReport(GNSSLocation)
}
impl Event {

    #[inline]
    pub fn to_event_type(&self) -> EventType {
        match self {
            Event::IncomingMessage(_) => EventType::IncomingMessage,
            Event::OutgoingMessage(_) => EventType::OutgoingMessage,
            Event::DeliveryReport { .. } => EventType::DeliveryReport,
            Event::ModemStatusUpdate { .. } => EventType::ModemStatusUpdate,
            Event::GNSSPositionReport(_) => EventType::GNSSPositionReport
        }
    }
}

#[derive(Clone)]
pub struct EventBroadcaster {
    pub webhooks: Option<WebhookSender>,
    pub websocket: Option<WebSocketManager>,
}
impl EventBroadcaster {
    pub fn create(
        webhooks: Option<Vec<ConfiguredWebhook>>,
        websocket_enabled: bool
    ) -> (Option<Self>, Option<JoinHandle<()>>) {
        let (webhook_sender, webhook_handle) = webhooks.map(WebhookSender::new)
            .map_or((None, None), |(sender, handle)| (Some(sender), Some(handle)));

        let enabled = websocket_enabled || webhook_sender.is_some();
        (
            if enabled {
                Some(EventBroadcaster {
                    webhooks: webhook_sender,
                    websocket: websocket_enabled.then(WebSocketManager::new)
                })
            } else {
                None
            },
            webhook_handle
        )
    }

    #[inline]
    pub async fn broadcast(&self, event: Event) {
        debug!("Broadcasting event: {:?}", event);
        if let Some(webhooks) = &self.webhooks {
            webhooks.send(event.clone());
        }
        if let Some(websocket) = &self.websocket {
            websocket.broadcast(event).await;
        }
    }
}