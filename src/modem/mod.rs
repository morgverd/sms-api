use anyhow::{anyhow, Result};
use tracing::log::error;
use tokio::sync::mpsc;
use tokio_serial::SerialPortBuilderExt;
use crate::config::ModemConfig;
use crate::modem::commands::OutgoingCommand;
use crate::modem::sender::ModemSender;
use crate::modem::types::ModemIncomingMessage;
use crate::modem::worker::ModemWorker;

pub mod sender;
pub mod types;
mod buffer;
mod commands;
mod handlers;
mod state_machine;
mod worker;
mod parsers;

pub struct ModemManager {
    config: ModemConfig,
    main_tx: mpsc::UnboundedSender<ModemIncomingMessage>,
    command_tx: Option<mpsc::Sender<OutgoingCommand>>
}
impl ModemManager {
    pub fn new(config: ModemConfig) -> (Self, mpsc::UnboundedReceiver<ModemIncomingMessage>) {
        let (main_tx, main_rx) = mpsc::unbounded_channel();
        let manager = Self {
            config,
            main_tx,
            command_tx: None
        };

        (manager, main_rx)
    }

    pub async fn start(&mut self) -> Result<tokio::task::JoinHandle<()>> {
        let (command_tx, command_rx) = mpsc::channel(self.config.cmd_channel_buffer_size);
        self.command_tx = Some(command_tx);

        let port = tokio_serial::new(&self.config.device, self.config.baud)
            .open_native_async()
            .map_err(|e| anyhow!("Failed to open serial port {}: {}", self.config.device, e))?;

        let worker = ModemWorker::new(port, self.main_tx.clone(), self.config.clone())?;
        let handle = tokio::spawn(async move {
            if let Err(e) = worker.initialize_and_run(command_rx).await {
                error!("ModemWorker error: {}", e);
            }
        });

        Ok(handle)
    }

    pub fn get_sender(&mut self) -> Result<ModemSender> {
        if let Some(command_tx) = self.command_tx.take() {
            Ok(ModemSender::new(command_tx))
        } else {
            Err(anyhow!("Could not get ModemSender, command_tx channel has already been taken or the modem hasn't been started!"))
        }
    }
}