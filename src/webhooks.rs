use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use futures::{stream, StreamExt};
use tracing::log::{debug, error, info, warn};
use reqwest::Client;
use reqwest::header::HeaderMap;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use anyhow::{bail, Context, Result};
use crate::config::ConfiguredWebhook;
use crate::events::{Event, EventType};

const CONCURRENCY_LIMIT: usize = 10;
const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct WebhookSender {
    event_sender: mpsc::UnboundedSender<Event>,
}
impl WebhookSender {
    pub fn new(webhooks: Vec<ConfiguredWebhook>) -> (Self, JoinHandle<()>) {

        // Use an unbounded channel to ensure no webhooks are ever dropped.
        // The modem command channel is bound, so we should be fine from API spam.
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        let handle = tokio::spawn(async move {
            let worker = WebhookWorker::new(webhooks, event_receiver);
            worker.run().await;
        });

        let manager = Self { event_sender };
        (manager, handle)
    }

    pub fn send(&self, event: Event) {
        if let Err(e) = self.event_sender.send(event) {
            error!("Failed to queue webhook job: {}", e);
        }
    }
}

type StoredWebhook = (ConfiguredWebhook, Option<HeaderMap>);

struct WebhookWorker {
    webhooks: Arc<[StoredWebhook]>,
    events_map: HashMap<EventType, Vec<usize>>,
    event_receiver: mpsc::UnboundedReceiver<Event>,
    client: Client
}
impl WebhookWorker {
    fn new(webhooks: Vec<ConfiguredWebhook>, event_receiver: mpsc::UnboundedReceiver<Event>) -> Self {
        let mut events_map: HashMap<EventType, Vec<usize>> = HashMap::new();
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

    async fn process(&self, event: Event) {
        let webhook_indices = match self.events_map.get(&event.to_event_type()) {
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

                // TODO: Maybe re-queue failed webhooks?
                async move {
                    match Self::execute_webhook(webhook, &client, &event).await {
                        Ok(()) => debug!("Webhook #{} for task #{} was sent successfully!", webhook_idx, task_idx),
                        Err(e) => warn!("Failed to send Webhook #{} for task #{} with error: {}", webhook_idx, task_idx, e)
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
        event: &Event
    ) -> Result<()> {
        let mut request = client
            .post(&webhook.url)
            .json(event);

        if let Some(headers) = headers {
            request = request.headers(headers.clone());
        }

        let status = request.send().await
            .with_context(|| "Network error")?
            .status();

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