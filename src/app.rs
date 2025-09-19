use std::time::Duration;
use anyhow::{bail, Result};
use axum::ServiceExt;
use tracing::log::{debug, error, info, warn};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;
use tokio::time::interval;

use crate::config::{AppConfig, HTTPConfig};
use crate::events::{Event, EventBroadcaster};
use crate::http::create_app;
use crate::http::websocket::WebSocketManager;
use crate::modem::ModemManager;
use crate::modem::types::ModemIncomingMessage;
use crate::sms::{SMSManager, SMSReceiver};
use crate::TracingReloadHandle;

#[cfg(feature = "sentry")]
pub type SentryGuard = Option<sentry::ClientInitGuard>;

#[cfg(not(feature = "sentry"))]
pub type SentryGuard = Option<()>;

pub struct AppHandles {
    tasks: Vec<(&'static str, JoinHandle<()>)>,
    _sentry_guard: SentryGuard,
}
impl AppHandles {
    pub async fn create(
        config: AppConfig,
        tracing_reload: TracingReloadHandle,
        _sentry_guard: SentryGuard,
    ) -> Result<AppHandles> {
        let mut tasks = Vec::new();

        // Start modem manager
        let (mut modem, main_rx) = ModemManager::new(config.modem);
        let (modem_handle, modem_sender) = match modem.start().await {
            Ok(handle) => (handle, modem.get_sender()?),
            Err(e) => bail!("Failed to start ModemManager: {:?}", e),
        };
        tasks.push(("Modem Handler", modem_handle));

        // Create event broadcaster (and webhook worker handle).
        let (broadcaster, webhooks_handle) = EventBroadcaster::create(config.webhooks, config.http.websocket_enabled);
        if let Some(webhooks_worker) = webhooks_handle {
            tasks.push(("Webhooks Worker", webhooks_worker));
        }

        // Setup SMS manager and receivers.
        let sms_manager = SMSManager::connect(
            config.database,
            modem_sender,
            broadcaster.clone()
        ).await?;

        let (cleanup_handle, channel_handle) = Self::start_sms_receiver(
            main_rx,
            sms_manager.clone(),
            broadcaster.clone()
        );
        tasks.push(("Modem Cleanup", cleanup_handle));
        tasks.push(("Modem Channel", channel_handle));

        // Setup HTTP server if enabled.
        if let Some(http_handle) = Self::start_http_server(
            config.http,
            broadcaster.and_then(|broadcaster| broadcaster.websocket),
            sms_manager,
            tracing_reload,
            _sentry_guard.is_some(),
        )? {
            tasks.push(("HTTP Server", http_handle));
        }

        Ok(AppHandles {
            tasks,
            _sentry_guard,
        })
    }

    pub async fn run(self) {
        let futures: Vec<_> = self.tasks
            .into_iter()
            .map(|(name, handle)| {
                info!("Starting task: {}.", name);
                Box::pin(async move {
                    match handle.await {
                        Ok(()) => info!("{} task completed!", name),
                        Err(e) => error!("{} task failed: {:?}!", name, e),
                    }
                })
            })
            .collect();

        // Wait for any task to complete. All handles are boxed, so when dropped they are cancelled.
        let (_, _, remaining) = futures::future::select_all(futures).await;
        drop(remaining);
    }

    fn start_sms_receiver(
        mut main_rx: UnboundedReceiver<ModemIncomingMessage>,
        sms_manager: SMSManager,
        broadcaster: Option<EventBroadcaster>
    ) -> (JoinHandle<()>, JoinHandle<()>) {
        let receiver = SMSReceiver::new(sms_manager);

        // Cleanup task
        let mut cleanup_receiver = receiver.clone();
        let cleanup_handle = tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(600)); // 10 minutes

            loop {
                interval.tick().await;
                cleanup_receiver.cleanup_stalled_multipart().await;
            }
        });

        // Message handling task
        let mut message_receiver = receiver;
        let channel_handle = tokio::spawn(async move {
            while let Some(message) = main_rx.recv().await {
                Self::handle_modem_message(message, &mut message_receiver, &broadcaster).await;
            }
        });

        (cleanup_handle, channel_handle)
    }

    async fn handle_modem_message(
        message: ModemIncomingMessage,
        receiver: &mut SMSReceiver,
        broadcaster: &Option<EventBroadcaster>,
    ) {
        match message {
            ModemIncomingMessage::IncomingSMS(incoming) => {
                match receiver.handle_incoming_sms(incoming).await {
                    Some(Ok(row_id)) => debug!("Stored SMS message #{}", row_id),
                    Some(Err(e)) => error!("Failed to store SMS: {:?}", e),
                    None => debug!("SMS is part of multipart message, not storing yet"),
                }
            }
            ModemIncomingMessage::DeliveryReport(report) => {
                match receiver.handle_delivery_report(report).await {
                    Ok(message_id) => debug!("Updated delivery status for message #{}", message_id),
                    Err(e) => warn!("Failed to update delivery report: {:?}", e),
                }
            }
            ModemIncomingMessage::ModemStatusUpdate { previous, current } => {
                if let Some(broadcaster) = broadcaster {
                    broadcaster.broadcast(Event::ModemStatusUpdate { previous, current }).await;
                }
            }
            ModemIncomingMessage::GNSSPositionReport(location) => {
                if let Some(broadcaster) = broadcaster {
                    broadcaster.broadcast(Event::GNSSPositionReport(location)).await;
                }
            }
            _ => warn!("Unhandled message type: {:?}", message),
        }
    }

    fn start_http_server(
        config: HTTPConfig,
        websocket: Option<WebSocketManager>,
        sms_manager: SMSManager,
        tracing_reload: TracingReloadHandle,
        sentry_enabled: bool,
    ) -> Result<Option<JoinHandle<()>>> {
        if !config.enabled {
            info!("HTTP server disabled in config");
            return Ok(None);
        }

        let tls_config = config.tls.clone();
        let address = config.address;

        let app = create_app(config, websocket, sms_manager, tracing_reload, sentry_enabled)?;
        let handle = tokio::spawn(async move {
            let result = match tls_config {
                Some(tls_config) => {
                    info!("Starting HTTPS (secure) server on {}.", address);

                    #[cfg(feature = "rust-tls")]
                    {
                        let _  = rustls::crypto::CryptoProvider::install_default(
                            rustls::crypto::aws_lc_rs::default_provider()
                        );
                        let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(
                            &tls_config.cert_path, &tls_config.key_path
                        ).await.expect("Failed to load rustls TLS certificates!");
                        axum_server::bind_rustls(address, tls).serve(app.into_make_service()).await
                    }
                    #[cfg(feature = "default-tls")]
                    {
                        let tls = axum_server::tls_openssl::OpenSSLConfig::from_pem_file(
                            &tls_config.cert_path, &tls_config.key_path
                        ).expect("Failed to load openssl TLS certificates!");
                        axum_server::bind_openssl(address, tls).serve(app.into_make_service()).await
                    }
                },
                None => {
                    info!("Starting HTTP (insecure) server on {}.", address);
                    axum_server::bind(address).serve(app.into_make_service()).await
                }
            };

            if let Err(e) = result {
                error!("Server error: {:?}", e);
            }
        });

        Ok(Some(handle))
    }
}