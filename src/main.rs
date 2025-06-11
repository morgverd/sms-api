use std::sync::Arc;
use anyhow::{anyhow, bail, Result};
use log::{debug, error, info};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;
use crate::http::create_app;
use crate::modem::types::{ModemConfig, ModemIncomingMessage};
use crate::modem::ModemManager;
use crate::sms::SMSManager;
use crate::sms::types::SMSIncomingMessage;

mod modem;
mod http;
mod sms;

struct AppHandles {
    modem: JoinHandle<()>,
    receiver: JoinHandle<()>,
    http: JoinHandle<Result<()>>
}

#[derive(Clone)]
struct AppState {
    pub sms_manager: Arc<SMSManager>
}
impl AppState {
    pub async fn create() -> Result<AppHandles> {
        let (mut modem, main_rx) = ModemManager::new(ModemConfig {
            device: "/dev/ttyS0",
            baud: 115200,
        });

        // Start Modem task and get handle to join with HTTP server.
        let modem_handle = match modem.start().await {
            Ok(handle) => handle,
            Err(e) => bail!("Failed to start ModemManager: {}", e)
        };

        // FIXME: TEMP!
        let database_url = "./sms.db";
        let encryption_key = [
            147, 203, 89, 45, 12, 178, 234, 67, 91, 156, 23, 88, 201, 142, 76, 39,
            165, 118, 95, 212, 33, 184, 157, 72, 109, 246, 58, 131, 194, 85, 167, 29
        ];

        // Create shared ModemManager.
        let modem_sender = modem.get_sender()?;
        let sms_manager = Arc::new(
            SMSManager::new(modem_sender, database_url, encryption_key).await?
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
            while let Some(message) = main_rx.recv().await {
                info!("AppState modem_receiver: {:?}", message);

                // Forward incoming SMS messages to manager!
                let forward_result = match message {
                    ModemIncomingMessage::IncomingSMS { phone_number, content, .. } => {
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
    
    fn create_http(sms_manager: Arc<SMSManager>) -> JoinHandle<Result<()>> {
        tokio::spawn(async move {
            let app = create_app(Self { sms_manager });
            let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
                .await
                .expect("Failed to bind to address");

            axum::serve(listener, app).await
                .map_err(|e| anyhow!("HTTP server error: {}", e))
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Debug)
        .init();
    
    let handles = AppState::create().await?;
    tokio::select! {
        result = handles.modem => {
            match result {
                Ok(()) => info!("Modem Handler task completed successfully"),
                Err(e) => error!("Modem Handler task failed: {}", e)
            }
        }
        result = handles.receiver => {
            match result {
                Ok(()) => info!("Modem Receiver task completed successfully"),
                Err(e) => error!("Modem Receiver task failed: {}", e)
            }
        },
        result = handles.http => {
            match result {
                Ok(Ok(())) => info!("HTTP task completed successfully"),
                Ok(Err(e)) => error!("HTTP task failed: {}", e),
                Err(e) => error!("HTTP task panicked: {}", e),
            }
        }
    }

    Ok(())
}