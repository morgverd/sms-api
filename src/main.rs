mod modem;
mod http;
mod sms;
mod config;

use std::path::PathBuf;
use std::sync::Arc;
use anyhow::{bail, Result};
use env_logger::Env;
use log::{debug, error, info, warn};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;
use clap::Parser;
use crate::config::{AppConfig, HTTPConfig};
use crate::http::create_app;
use crate::modem::types::ModemIncomingMessage;
use crate::modem::ModemManager;
use crate::sms::SMSManager;

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
    receiver: JoinHandle<()>,
    http_opt: Option<JoinHandle<()>>
}

#[derive(Clone)]
struct AppState {
    pub sms_manager: Arc<SMSManager>
}
impl AppState {
    pub async fn create(config: AppConfig) -> Result<AppHandles> {
        let (mut modem, main_rx) = ModemManager::new(config.modem);

        // Start Modem task and get handle to join with HTTP server.
        let modem_handle = match modem.start().await {
            Ok(handle) => handle,
            Err(e) => bail!("Failed to start ModemManager: {}", e)
        };

        // Create shared ModemManager.
        let modem_sender = modem.get_sender()?;
        let sms_manager = Arc::new(
            SMSManager::connect(config.sms, modem_sender).await?
        );

        let handles = AppHandles {
            modem: modem_handle,
            receiver: Self::create_receiver(sms_manager.clone(), main_rx),
            http_opt: Self::try_create_http(sms_manager.clone(), config.http),
        };
        Ok(handles)
    }
    
    fn create_receiver(
        sms_manager: Arc<SMSManager>,
        mut main_rx: UnboundedReceiver<ModemIncomingMessage>
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            info!("Started ModemIncomingMessage reciever");
            while let Some(message) = main_rx.recv().await {
                debug!("AppState modem_receiver: {:?}", message);

                match message {
                    ModemIncomingMessage::IncomingSMS(incoming) => {
                        match sms_manager.handle_incoming_sms(incoming).await {
                            Ok(row_id) => debug!("Stored SMSIncomingMessage #{}", row_id),
                            Err(e) => error!("Failed to store SMSIncomingMessage with error: {:?}", e)
                        }
                    },
                    ModemIncomingMessage::DeliveryReport(report) => {
                        match sms_manager.handle_delivery_report(report).await {
                            Ok(message_id) => debug!("Updated delivery report status for message #{}", message_id),
                            Err(e) => error!("Failed to update message delivery report with error: {:?}", e)
                        }
                    },
                    _ => warn!("Unimplemented ModemIncomingMessage for SMSManager: {:?}", message)
                };
            }
        })
    }

    fn try_create_http(
        sms_manager: Arc<SMSManager>,
        config: HTTPConfig
    ) -> Option<JoinHandle<()>> {
        if !config.enabled {
            info!("HTTP server is disabled in config!");
            return None;
        }

        let address = config.address;
        let app_state = Self { sms_manager };

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
struct CliArguments {

    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init_from_env(Env::default().default_filter_or("info"));

    let args = CliArguments::parse();
    let config = AppConfig::load(args.config)?;
    println!("{:?}", config);

    let handles = AppState::create(config).await?;
    if let Some(http_handle) = handles.http_opt {
        tokio_select_with_logging! {
            "Modem Handler" => handles.modem,
            "Modem Receiver" => handles.receiver,
            "HTTP Server" => http_handle
        }
    } else {
        tokio_select_with_logging! {
            "Modem Handler" => handles.modem,
            "Modem Receiver" => handles.receiver
        }
    }

    Ok(())
}