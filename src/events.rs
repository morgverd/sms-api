use serde::Serialize;
use tokio::task::JoinHandle;
use tracing::log::debug;
use crate::config::{ConfiguredWebhook, ConfiguredWebhookEvent};
use crate::http::websocket::WebSocketManager;
use crate::modem::types::{GNSSLocation, ModemStatus};
use crate::sms::types::{SMSIncomingDeliveryReport, SMSMessage};
use crate::webhooks::WebhookSender;

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
    pub fn to_configured_event(&self) -> ConfiguredWebhookEvent {
        match self {
            Event::IncomingMessage(_) => ConfiguredWebhookEvent::IncomingMessage,
            Event::OutgoingMessage(_) => ConfiguredWebhookEvent::OutgoingMessage,
            Event::DeliveryReport { .. } => ConfiguredWebhookEvent::DeliveryReport,
            Event::ModemStatusUpdate { .. } => ConfiguredWebhookEvent::ModemStatusUpdate,
            Event::GNSSPositionReport(_) => ConfiguredWebhookEvent::GNSSPositionReport
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