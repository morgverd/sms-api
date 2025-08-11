use std::time::Duration;
use anyhow::{bail, Result};
use log::{debug, error, info, warn};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;
use tokio::time::interval;
use crate::config::{AppConfig, HTTPConfig};
use crate::http::create_app;
use crate::modem::ModemManager;
use crate::modem::types::ModemIncomingMessage;
use crate::SentryGuard;
use crate::sms::{SMSManager, SMSReceiver};
use crate::webhooks::{WebhookEvent, WebhookSender};

macro_rules! tokio_select_with_logging {
    ($($name:expr => $handle:expr),+ $(,)?) => {
        tokio::select! {
            $(result = $handle => match result {
                Ok(()) => info!("{} task completed", $name),
                Err(e) => error!("{} task failed: {:?}", $name, e)
            }),+
        }
    };
}

#[derive(Clone)]
pub struct HttpState {
    pub sms_manager: SMSManager,
    pub config: HTTPConfig
}

pub struct AppHandles {
    modem: JoinHandle<()>,
    modem_cleanup: JoinHandle<()>,
    modem_channel: JoinHandle<()>,
    http_opt: Option<JoinHandle<()>>,
    webhooks_opt: Option<JoinHandle<()>>,

    // This is only filled if compiled with sentry feature.
    _sentry_guard: SentryGuard
}
impl AppHandles {
    pub async fn create(
        config: AppConfig,
        _sentry_guard: SentryGuard
    ) -> Result<AppHandles> {

        // Start Modem task and get handle to join with HTTP server.
        let (mut modem, main_rx) = ModemManager::new(config.modem);
        let (modem_handle, modem_sender) = match modem.start().await {
            Ok(handle) => (handle, modem.get_sender()?),
            Err(e) => bail!("Failed to start ModemManager: {:?}", e)
        };

        // Create webhook manager here to get its reader handle.
        let (webhooks_sender_opt, webhooks_handle) = WebhookSender::new(config.webhooks).unzip();
        let sms_manager = SMSManager::connect(config.database, modem_sender, webhooks_sender_opt.clone()).await?;

        let (receiver_cleanup_handle, receiver_channel_handle) = Self::create_receiver(sms_manager.clone(), webhooks_sender_opt.clone(), main_rx);
        let handles = AppHandles {
            modem: modem_handle,
            modem_cleanup: receiver_cleanup_handle,
            modem_channel: receiver_channel_handle,
            http_opt: Self::try_create_http(sms_manager.clone(), config.http, _sentry_guard.is_some()),
            webhooks_opt: webhooks_handle,
            _sentry_guard
        };
        Ok(handles)
    }

    pub async fn run(self) {
        match (self.http_opt, self.webhooks_opt) {
            (Some(http), Some(webhooks)) => tokio_select_with_logging! {
                "Modem Handler" => self.modem,
                "Modem Cleanup" => self.modem_cleanup,
                "Modem Channel" => self.modem_channel,
                "HTTP Server" => http,
                "Webhooks Sender" => webhooks
            },
            (Some(http), None) => tokio_select_with_logging! {
                "Modem Handler" => self.modem,
                "Modem Cleanup" => self.modem_cleanup,
                "Modem Channel" => self.modem_channel,
                "HTTP Server" => http
            },
            (None, Some(webhooks)) => tokio_select_with_logging! {
                "Modem Handler" => self.modem,
                "Modem Cleanup" => self.modem_cleanup,
                "Modem Channel" => self.modem_channel,
                "Webhooks Sender" => webhooks
            },
            (None, None) => tokio_select_with_logging! {
                "Modem Handler" => self.modem,
                "Modem Cleanup" => self.modem_cleanup,
                "Modem Channel" => self.modem_channel
            }
        }
    }

    fn create_receiver(
        sms_manager: SMSManager,
        webhooks_sender: Option<WebhookSender>,
        mut main_rx: UnboundedReceiver<ModemIncomingMessage>
    ) -> (JoinHandle<()>, JoinHandle<()>) {
        let receiver = SMSReceiver::new(sms_manager);

        // Cleanup stalled multipart messages from SMSReceiver.
        let mut cleanup_receiver = receiver.clone();
        let cleanup_handle = tokio::spawn(async move {
            info!("Started Modem Multipart Messages garbage collector");
            let mut interval = interval(Duration::from_secs(10 * 60)); // 10 minutes

            loop {
                interval.tick().await;
                cleanup_receiver.cleanup_stalled_multipart().await;
            }
        });

        // Forward ModemIncomingMessage's from the modem to SMSManager.
        let mut channel_receiver = receiver.clone();
        let channel_handle = tokio::spawn(async move {
            info!("Started ModemIncomingMessage receiver");

            while let Some(message) = main_rx.recv().await {
                debug!("AppState modem_receiver: {:?}", message);

                match message {
                    ModemIncomingMessage::IncomingSMS(incoming) => {
                        match channel_receiver.handle_incoming_sms(incoming).await {
                            Some(Ok(row_id)) => debug!("Stored SMSIncomingMessage #{}", row_id),
                            Some(Err(e)) => error!("Failed to store SMSIncomingMessage with error: {:?}", e),
                            None => debug!("Not storing SMSIncomingMessage as it is apart of a multipart message.")
                        }
                    },
                    ModemIncomingMessage::DeliveryReport(report) => {
                        match channel_receiver.handle_delivery_report(report).await {
                            Ok(message_id) => debug!("Updated delivery report status for message #{}", message_id),
                            Err(e) => warn!("Failed to update message delivery report with error: {:?}", e)
                        }
                    },
                    ModemIncomingMessage::ModemStatusUpdate { previous, current } => {
                        if let Some(webhooks) = &webhooks_sender {
                            webhooks.send(WebhookEvent::ModemStatusUpdate {
                                previous,
                                current
                            });
                        }
                    },
                    _ => warn!("Unimplemented ModemIncomingMessage for SMSManager: {:?}", message)
                };
            }
        });

        (cleanup_handle, channel_handle)
    }

    fn try_create_http(
        sms_manager: SMSManager,
        config: HTTPConfig,
        _sentry: bool
    ) -> Option<JoinHandle<()>> {
        if !config.enabled {
            info!("HTTP server is disabled in config!");
            return None;
        }

        let address = config.address;
        let app_state = HttpState { sms_manager, config };

        let handle = tokio::spawn(async move {
            let app = create_app(app_state, _sentry);
            let listener = tokio::net::TcpListener::bind(address)
                .await
                .expect("Failed to bind to address");

            info!("Started HTTP listener @ {}", address.to_string());
            match axum::serve(listener, app).await {
                Ok(_) => debug!("HTTP server terminated."),
                Err(e) => error!("HTTP server error: {:?}", e)
            }
        });
        Some(handle)
    }
}
