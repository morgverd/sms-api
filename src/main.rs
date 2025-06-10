use anyhow::{anyhow, bail, Result};
use huawei_modem::pdu::DeliverPdu;
use log::{error, info};
use crate::http::create_app;
use crate::http::types::AppState;
use crate::modem::types::ModemConfig;
use crate::modem::ModemManager;

mod modem;
mod http;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Debug)
        .init();

    // Initialize modem connection.
    let config = ModemConfig {
        device: "/dev/ttyS0",
        baud: 115200,
    };
    let (mut modem, mut main_rx) = ModemManager::new(config);

    // Spawn main receiver task.
    tokio::spawn(async move {
        while let Some(message) = main_rx.recv().await {
            info!("RECV: {:?}", message);
        }
    });

    // Start Modem task and get handle to join with HTTP server.
    let modem_handle = match modem.start().await {
        Ok(handle) => handle,
        Err(e) => bail!("Failed to start ModemManager: {}", e)
    };

    // Create API Application and listener.
    let app = create_app(AppState { sender: modem.get_sender()? });
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .expect("Failed to bind to address");

    tokio::select! {
        result = axum::serve(listener, app) => {
            if let Err(e) = result {
                error!("HTTP server error: {}", e);
            }
        }
        result = modem_handle => {
            if let Err(e) = result {
                log::error!("Modem task error: {}", e);
            }
        }
    }

    Ok(())
}