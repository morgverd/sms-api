use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use futures::{stream, StreamExt};
use log::{debug, error, info};
use reqwest::Client;
use reqwest::header::HeaderMap;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use crate::config::{ConfiguredWebhook, ConfiguredWebhookEvent};
use crate::sms::types::{SMSIncomingDeliveryReport, SMSMessage};
use crate::modem::types::ModemStatus;

const CONCURRENCY_LIMIT: usize = 10;
const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum WebhookEvent {
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
    ModemStatusUpdate(ModemStatus)
}
impl WebhookEvent {

    #[inline]
    pub fn to_configured_event(&self) -> ConfiguredWebhookEvent {
        match self {
            WebhookEvent::IncomingMessage(_) => ConfiguredWebhookEvent::IncomingMessage,
            WebhookEvent::OutgoingMessage(_) => ConfiguredWebhookEvent::OutgoingMessage,
            WebhookEvent::DeliveryReport { .. } => ConfiguredWebhookEvent::DeliveryReport,
            WebhookEvent::ModemStatusUpdate { .. } => ConfiguredWebhookEvent::ModemStatusUpdate
        }
    }
}

#[derive(Clone)]
pub struct WebhookSender {
    event_sender: mpsc::UnboundedSender<WebhookEvent>,
}
impl WebhookSender {
    pub fn new(webhooks: Option<Vec<ConfiguredWebhook>>) -> Option<(Self, JoinHandle<()>)> {
        let webhooks = match webhooks {
            Some(webhooks) => webhooks,
            None => {
                info!("There are no webhook targets within config!");
                return None;
            }
        };

        // Use an unbounded channel to ensure no webhooks are ever dropped.
        // The modem command channel is bound, so we should be fine from API spam.
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        let handle = tokio::spawn(async move {
            let worker = WebhookWorker::new(webhooks, event_receiver);
            worker.run().await;
        });

        let manager = Self { event_sender };
        Some((manager, handle))
    }

    pub fn send(&self, event: WebhookEvent) {
        debug!("Sending webhook event: {:?}", event);
        if let Err(e) = self.event_sender.send(event) {
            error!("Failed to queue webhook job: {}", e);
        }
    }
}

type StoredWebhook = (ConfiguredWebhook, Option<HeaderMap>);

struct WebhookWorker {
    webhooks: Arc<[StoredWebhook]>,
    events_map: HashMap<ConfiguredWebhookEvent, Vec<usize>>,
    event_receiver: mpsc::UnboundedReceiver<WebhookEvent>,
    client: Client
}
impl WebhookWorker {
    fn new(webhooks: Vec<ConfiguredWebhook>, event_receiver: mpsc::UnboundedReceiver<WebhookEvent>) -> Self {
        let mut events_map: HashMap<ConfiguredWebhookEvent, Vec<usize>> = HashMap::new();
        for (idx, webhook) in webhooks.iter().enumerate() {
            for event in &webhook.events {
                events_map.entry(*event)
                    .or_default()
                    .push(idx);
            }
        }

        let client = Client::builder()
            .timeout(WEBHOOK_TIMEOUT)
            .build()
            .unwrap_or_else(|e| {
                error!("Could not build timeout HTTP client with error: {}", e);
                Client::new()
            });

        Self {

            // Cache all webhook HeaderMaps now instead of re-creating each time.
            webhooks: webhooks.into_iter()
                .enumerate()
                .map(|(idx, webhook)| {
                    let headers = webhook.get_header_map()
                        .unwrap_or_else(|e| {
                            error!("Failed to create Webhook #{} HeaderMap with error: {}", idx, e);
                            None
                        });

                    (webhook, headers)
                })
                .collect::<Vec<StoredWebhook>>()
                .into(),

            events_map,
            event_receiver,
            client
        }
    }

    async fn run(mut self) {
        info!("Starting webhook worker");
        while let Some(event) = self.event_receiver.recv().await {
            self.process(event).await;
        }
    }

    async fn process(&self, event: WebhookEvent) {
        let webhook_indices = match self.events_map.get(&event.to_configured_event()) {
            Some(indices) => indices.clone(),
            None => return
        };

        let event = Arc::new(event);
        let webhooks = Arc::clone(&self.webhooks);

        stream::iter(webhook_indices.into_iter().enumerate())
            .map(|(task_idx, webhook_idx)| {
                let webhook = &webhooks[webhook_idx];
                let event = Arc::clone(&event);
                let client = &self.client;

                async move {
                    let result = Self::execute_webhook(
                        webhook,
                        &client,
                        &event
                    ).await;

                    // TODO: Maybe re-queue failed webhooks?
                    match result {
                        Ok(()) => debug!("Webhook #{} for task #{} was sent successfully!", webhook_idx, task_idx),
                        Err(e) => error!("Failed to send Webhook #{} for task #{} with error: {}", webhook_idx, task_idx, e)
                    }
                }
            })
            .buffer_unordered(CONCURRENCY_LIMIT)
            .for_each(|_| async {})
            .await;
    }

    async fn execute_webhook(
        (webhook, headers): &StoredWebhook,
        client: &Client,
        event: &WebhookEvent
    ) -> Result<()> {
        let mut request = client
            .post(&webhook.url)
            .json(event);

        if let Some(headers) = headers {
            request = request.headers(headers.clone());
        }

        debug!("Sending webhook to: {}", webhook.url);
        let response = request.send().await
            .with_context(|| "Network error")?;

        let status = response.status();
        match webhook.expected_status {
            Some(expected) if status.as_u16() != expected => {
                bail!("Got {} expected {}!", status.as_u16(), expected);
            }
            None if !status.is_success() => {
                bail!("Unsuccessful status {}", status);
            }
            _ => Ok(())
        }
    }
}