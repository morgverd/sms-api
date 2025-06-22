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
use crate::config::{ConfiguredWebhook, ConfiguredWebhookEvent};
use crate::sms::types::SMSMessage;

const CONCURRENCY_LIMIT: usize = 10;
const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct WebhookJob {
    pub event: ConfiguredWebhookEvent,
    pub message: SMSMessage
}

#[derive(Clone)]
pub struct SMSWebhookManager {
    job_sender: mpsc::UnboundedSender<WebhookJob>,
}
impl SMSWebhookManager {
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
        let (job_sender, job_receiver) = mpsc::unbounded_channel();
        let handle = tokio::spawn(async move {
            let worker = SMSWebhookWorker::new(webhooks, job_receiver);
            worker.run().await;
        });

        let manager = Self { job_sender };
        Some((manager, handle))
    }

    pub fn send(&self, event: ConfiguredWebhookEvent, message: SMSMessage) {
        let job = WebhookJob { event, message };
        if let Err(e) = self.job_sender.send(job) {
            error!("Failed to queue webhook job: {}", e);
        }
    }
}

type StoredWebhook = (ConfiguredWebhook, Option<HeaderMap>);

struct SMSWebhookWorker {
    webhooks: Arc<[StoredWebhook]>,
    events_map: HashMap<ConfiguredWebhookEvent, Vec<usize>>,
    job_receiver: mpsc::UnboundedReceiver<WebhookJob>,
    client: Client
}
impl SMSWebhookWorker {
    fn new(webhooks: Vec<ConfiguredWebhook>, job_receiver: mpsc::UnboundedReceiver<WebhookJob>) -> Self {
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
                            error!("Failed to create Webhook HeaderMap with error: {}", e);
                            None
                        });

                    (webhook, headers)
                })
                .collect::<Vec<StoredWebhook>>()
                .into(),

            events_map,
            job_receiver,
            client
        }
    }

    async fn run(mut self) {
        info!("Starting webhook worker");
        while let Some(job) = self.job_receiver.recv().await {
            self.process_job(job).await;
        }
    }

    async fn process_job(&self, job: WebhookJob) {
        let webhook_indices = match self.events_map.get(&job.event) {
            Some(indices) => indices.clone(),
            None => return
        };

        let message = Arc::new(job.message);
        let webhooks = Arc::clone(&self.webhooks);

        stream::iter(webhook_indices.into_iter().enumerate())
            .map(|(task_idx, webhook_idx)| {
                let webhook = &webhooks[webhook_idx];
                let message = Arc::clone(&message);
                let client = &self.client;

                async move {
                    let result = Self::execute_webhook(
                        webhook,
                        &client,
                        &message,
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
        message: &SMSMessage
    ) -> Result<()> {
        let mut request = client
            .post(&webhook.url)
            .json(message);

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