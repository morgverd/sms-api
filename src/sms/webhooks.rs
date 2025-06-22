use std::collections::HashMap;
use std::sync::Arc;
use futures::{stream, StreamExt};
use log::{debug, error, info, warn};
use reqwest::Client;
use crate::config::{ConfiguredWebhook, ConfiguredWebhookEvent};
use crate::sms::types::SMSMessage;

const CONCURRENCY_LIMIT: usize = 10;

#[derive(Clone)]
pub struct SMSWebhookManager {
    webhooks: Vec<ConfiguredWebhook>,
    events_map: HashMap<ConfiguredWebhookEvent, Vec<usize>>,
    client: Client,
}
impl SMSWebhookManager {
    pub fn new(webhooks: Option<Vec<ConfiguredWebhook>>) -> Option<Self> {
        let webhooks = match webhooks {
            Some(webhooks) => webhooks,
            None => {
                info!("There are no webhook targets within config!");
                return None
            }
        };

        // Store a map of webhooks related to each event for more efficient lookup.
        let mut events_map = HashMap::with_capacity(webhooks.len());
        for (idx, webhook) in webhooks.iter().enumerate() {
            for event in &webhook.events {
                events_map.entry(event.clone())
                    .or_insert_with(Vec::new)
                    .push(idx);
            }
        }

        let manager = Self {
            webhooks,
            events_map,
            client: Client::new()
        };
        Some(manager)
    }

    pub async fn send(&self, event: ConfiguredWebhookEvent, message: SMSMessage) -> () {
        let webhooks = match self.get_webhooks(&event) {
            Some(webhooks) => webhooks,
            None => return debug!("Could not find any target webhooks for: {:?}", event)
        };

        let message = Arc::new(message);
        stream::iter(webhooks.into_iter().enumerate())
            .map(|(idx, webhook)| {
                let message = Arc::clone(&message);
                async move {
                    let mut request = self.client
                        .post(&webhook.url)
                        .json(&*message);

                    // Apply optional request headers.
                    match webhook.get_header_map() {
                        Ok(headers) => if let Some(headers) = headers {
                            request = request.headers(headers);
                        },
                        Err(e) => error!("Could not create header map for Webhook #{} with error: {}", idx, e)
                    };

                    // Attempt to send request, verify response status.
                    debug!("Sending webhook #{}!", idx);
                    let success = match request.send().await {
                        Ok(response) => {
                            let status = response.status();
                            if let Some(expected) = webhook.expected_status {
                                expected == status.as_u16()
                            } else {
                                status.is_success()
                            }
                        },
                        Err(e) => {
                            warn!("Webhook #{} request failed with error: {}", idx, e);
                            false
                        }
                    };

                    (idx, success)
                }
            })
            .buffer_unordered(CONCURRENCY_LIMIT)
            .for_each(|(idx, success)| async move {
                debug!("Webhook #{} for {:?} {}!", idx, event, if success { "was sent successfully" } else { "failed to send" })
            })
            .await
    }

    fn get_webhooks(&self, event: &ConfiguredWebhookEvent) -> Option<Vec<ConfiguredWebhook>> {
        self.events_map
            .get(event)
            .map(|indices| {
                indices.iter()
                    .map(|&idx| self.webhooks[idx].clone())
                    .collect()
            })
    }
}