mod modem;
mod http;
mod sms;
mod config;

use std::sync::Arc;
use std::time::Duration;
use anyhow::{anyhow, bail, Error, Result};
use env_logger::Env;
use log::{debug, error, info, warn};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;
use crate::config::AppConfig;
use crate::http::create_app;
use crate::modem::types::{ModemIncomingMessage, ModemRequest};
use crate::modem::ModemManager;
use crate::sms::SMSManager;
use crate::sms::types::{SMSIncomingMessage, SMSOutgoingMessage};

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
    http: JoinHandle<()>
}

#[derive(Clone)]
struct AppState {
    pub sms_manager: Arc<SMSManager>
}
impl AppState {
    pub async fn create() -> Result<AppHandles> {
        let config = AppConfig::load_from_env()?;
        let (mut modem, main_rx) = ModemManager::new(config.modem);

        // Start Modem task and get handle to join with HTTP server.
        let modem_handle = match modem.start().await {
            Ok(handle) => handle,
            Err(e) => bail!("Failed to start ModemManager: {}", e)
        };

        // Create shared ModemManager.
        let modem_sender = modem.get_sender()?;
        let sms_manager = Arc::new(
            SMSManager::new(config.sms, modem_sender).await?
        );
        
        let handles = AppHandles {
            modem: modem_handle,
            receiver: Self::create_receiver(sms_manager.clone(), main_rx),
            http: Self::create_http(sms_manager.clone()),
        };
        Ok(handles)
    }
    
    fn create_receiver(sms_manager: Arc<SMSManager>, mut main_rx: UnboundedReceiver<ModemIncomingMessage>) -> JoinHandle<()> {
        tokio::spawn(async move {
            info!("Started ModemIncomingMessage reciever");
            while let Some(message) = main_rx.recv().await {
                debug!("AppState modem_receiver: {:?}", message);

                // Forward incoming SMS messages to manager!
                let forward_result = match message {
                    ModemIncomingMessage::IncomingSMS { phone_number, content } => {
                        let incoming = SMSIncomingMessage {
                            phone_number,
                            content
                        };
                        sms_manager.accept_incoming(incoming).await
                    },
                    _ => Err(anyhow!("Unimplemented ModemIncomingMessage for SMSManager: {:?}", message))
                };
                
                match forward_result {
                    Ok(row_id) => debug!("AppState modem_receiver: Stored SMSIncomingMessage #{}", row_id),
                    Err(e) => error!("AppState modem_receiver: Failed to store SMSIncomingMessage with error: {:?}", e)
                }
            }
        })
    }
    
    fn create_http(sms_manager: Arc<SMSManager>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let app = create_app(Self { sms_manager });
            let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
                .await
                .expect("Failed to bind to address");

            info!("Started HTTP listener @ 0.0.0.0:3000");
            match axum::serve(listener, app).await {
                Ok(_) => debug!("HTTP server terminated."),
                Err(e) => error!("HTTP server error: {:?}", e)
            }
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init_from_env(Env::default().default_filter_or("info"));
    
    let handles = AppState::create().await?;
    tokio_select_with_logging! {
        "Modem Handler" => handles.modem,
        "Modem Receiver" => handles.receiver,
        "HTTP Server" => handles.http
    }

    Ok(())
}