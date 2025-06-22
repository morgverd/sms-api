use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use futures::{stream, StreamExt};
use log::{debug, error, info, warn};
use reqwest::Client;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use crate::config::{ConfiguredWebhook, ConfiguredWebhookEvent};
use crate::sms::types::SMSMessage;

const CONCURRENCY_LIMIT: usize = 10;
const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct WebhookJob {
    pub event: ConfiguredWebhookEvent,
    pub message: SMSMessage,
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

struct SMSWebhookWorker {
    webhooks: Arc<[ConfiguredWebhook]>,
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
            .unwrap_or_else(|_| Client::new());

        Self {
            webhooks: webhooks.into(),
            events_map,
            job_receiver,
            client,
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
                    let success = Self::execute_webhook(
                        &client,
                        webhook,
                        &message,
                    ).await;
                    debug!("Webhook #{} for task #{} {}!", webhook_idx, task_idx, if success { "was sent successfully" } else { "failed to send" });
                }
            })
            .buffer_unordered(CONCURRENCY_LIMIT)
            .for_each(|_| async {})
            .await;
    }

    async fn execute_webhook(
        client: &Client,
        webhook: &ConfiguredWebhook,
        message: &SMSMessage,
    ) -> bool {
        let mut request = client
            .post(&webhook.url)
            .json(message);

        // Apply optional request headers.
        match webhook.get_header_map() {
            Ok(headers) => {
                if let Some(headers) = headers {
                    request = request.headers(headers);
                }
            },
            Err(e) => {
                error!("Could not create header map for webhook with error: {}", e);
                return false;
            }
        }

        // Send request and verify response status.
        debug!("Sending webhook to: {}", webhook.url);
        match request.send().await {
            Ok(response) => {
                let status = response.status();
                if let Some(expected) = webhook.expected_status {
                    expected == status.as_u16()
                } else {
                    status.is_success()
                }
            },
            Err(e) => {
                warn!("Webhook request to {} failed with error: {}", webhook.url, e);
                false
            }
        }
    }
}