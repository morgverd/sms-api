mod modem;
mod http;
mod sms;
mod config;
pub mod webhooks;

use std::path::PathBuf;
use std::time::Duration;
use anyhow::{bail, Result};
use env_logger::Env;
use log::{debug, error, info, warn};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;
use clap::Parser;
use tokio::time::interval;
use crate::config::{AppConfig, HTTPConfig};
use crate::http::create_app;
use crate::modem::types::ModemIncomingMessage;
use crate::modem::ModemManager;
use crate::sms::{SMSManager, SMSReceiver};
use webhooks::WebhookSender;
use crate::webhooks::WebhookEvent;

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

struct AppHandles {
    modem: JoinHandle<()>,
    modem_cleanup: JoinHandle<()>,
    modem_channel: JoinHandle<()>,
    http_opt: Option<JoinHandle<()>>,
    webhooks_opt: Option<JoinHandle<()>>
}

#[derive(Clone)]
struct AppState {
    pub sms_manager: SMSManager,
    pub config: HTTPConfig
}
impl AppState {
    pub async fn create(config: AppConfig) -> Result<AppHandles> {
        let (mut modem, main_rx) = ModemManager::new(config.modem);

        // Start Modem task and get handle to join with HTTP server.
        let (modem_handle, modem_sender) = match modem.start().await {
            Ok(handle) => (handle, modem.get_sender()?),
            Err(e) => bail!("Failed to start ModemManager: {}", e)
        };

        // Create webhook manager here to get its reader handle.
        let (webhooks_sender_opt, webhooks_handle) = WebhookSender::new(config.webhooks).unzip();
        let sms_manager = SMSManager::connect(config.database, modem_sender, webhooks_sender_opt.clone()).await?;

        let (receiver_cleanup_handle, receiver_channel_handle) = Self::create_receiver(sms_manager.clone(), webhooks_sender_opt.clone(), main_rx);
        let handles = AppHandles {
            modem: modem_handle,
            modem_cleanup: receiver_cleanup_handle,
            modem_channel: receiver_channel_handle,
            http_opt: Self::try_create_http(sms_manager.clone(), config.http),
            webhooks_opt: webhooks_handle
        };
        Ok(handles)
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
                            Err(e) => error!("Failed to update message delivery report with error: {:?}", e)
                        }
                    },
                    ModemIncomingMessage::ModemStatusUpdate(status) => {
                        if let Some(webhooks) = &webhooks_sender {
                            webhooks.send(WebhookEvent::ModemStatusUpdate(status));
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
        config: HTTPConfig
    ) -> Option<JoinHandle<()>> {
        if !config.enabled {
            info!("HTTP server is disabled in config!");
            return None;
        }

        let address = config.address;
        let app_state = Self { sms_manager, config };

        let handle = tokio::spawn(async move {
            let app = create_app(app_state);
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

#[derive(Parser)]
#[command(name = "sms-api")]
#[command(about = "A HTTP API that accepts and sends SMS messages.")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct CliArguments {

    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init_from_env(Env::default().default_filter_or("info"));

    let args = CliArguments::parse();
    let config = AppConfig::load(args.config)?;

    let handles = AppState::create(config).await?;
    match (handles.http_opt, handles.webhooks_opt) {
        (Some(http), Some(webhooks)) => tokio_select_with_logging! {
            "Modem Handler" => handles.modem,
            "Modem Cleanup" => handles.modem_cleanup,
            "Modem Channel" => handles.modem_channel,
            "HTTP Server" => http,
            "Webhooks Sender" => webhooks
        },
        (Some(http), None) => tokio_select_with_logging! {
            "Modem Handler" => handles.modem,
            "Modem Cleanup" => handles.modem_cleanup,
            "Modem Channel" => handles.modem_channel,
            "HTTP Server" => http
        },
        (None, Some(webhooks)) => tokio_select_with_logging! {
            "Modem Handler" => handles.modem,
            "Modem Cleanup" => handles.modem_cleanup,
            "Modem Channel" => handles.modem_channel,
            "Webhooks Sender" => webhooks
        },
        (None, None) => tokio_select_with_logging! {
            "Modem Handler" => handles.modem,
            "Modem Cleanup" => handles.modem_cleanup,
            "Modem Channel" => handles.modem_channel
        }
    }

    Ok(())
}