use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use anyhow::{anyhow, bail, Context, Result};
use tokio::io::{AsyncWriteExt};
use tokio_serial::{SerialPortBuilderExt, SerialStream};
use log::{debug, error, info};
use tokio::time::interval;
use crate::modem::buffer::LineBuffer;
use crate::modem::commands::OutgoingCommand;
use crate::modem::handlers::ModemEventHandlers;
use crate::modem::sender::ModemSender;
use crate::modem::state_machine::ModemStateMachine;
use crate::modem::types::{
    ModemConfig,
    ModemResponse,
    ModemIncomingMessage
};

pub mod types;
pub mod commands;
mod buffer;
pub mod sender;
mod handlers;
mod state_machine;

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

        let mut port = Arc::new(Mutex::new(
            tokio_serial::new(*&self.config.device, self.config.baud)
                .open_native_async()
                .map_err(|e| anyhow!("Failed to open serial port {}: {}", self.config.device, e))?
        ));

        // Initialize modem connection.
        let initialization_commands: Vec<(&[u8], &[u8])> = vec![
            (b"ATZ", b"OK"),                // Reset
            (b"AT", b"OK"),                 // Test connection
            (b"ATE0", b"OK"),               // Disable echo
            (b"AT+CMGF=0", b"OK"),          // Set SMS message format to PDU
            (b"AT+CLIP=1", b"OK"),          // Enable calling line identification (RING identifier)
            (b"AT+CSCS=\"GSM\"", b"OK"),    // Use GSM 7-bit alphabet
            (b"AT+CNMI=2,2,0,1,0", b"OK"),  // Receive all incoming SMS messages and delivery reports
            (b"AT+CSMP=49,167,0,0", b"OK"), // Receive delivery receipts from sent messages
            (b"AT+CPMS=\"ME\",\"ME\",\"ME\"", b"+CPMS: 1,50,1,50,1,50\r\n\r\nOK") // Store all messages in memory only
        ];
        for (command, expected) in initialization_commands {
            let command_str = String::from_utf8_lossy(command);
            {
                let mut port_guard = port.lock().await;
                port_guard.write_all(command).await?;
                port_guard.write_all(b"\r\n").await?;
            }

            let response = Self::read_response_until_ok(&mut port).await
                .with_context(|| format!("Failed to read response for initialization command: {}", command_str))?;

            let response_str = String::from_utf8_lossy(&response);
            let expected_str = String::from_utf8_lossy(expected);

            if !response_str.contains(&*expected_str) {
                bail!(
                    "Initialization command '{}' failed. Expected: '{}', Got: '{}'",
                    command_str, expected_str, response_str.trim()
                );
            }
        }
        debug!("Initialized modem successfully!");

        // Spawn the main modem handling task.
        // This is passed back to the main thread to be joined with the HTTP server.
        let main_tx = self.main_tx.clone();
        let read_interval_duration = self.config.read_interval_duration;
        let handle = tokio::spawn(async move {
            if let Err(e) = Self::modem_task(port, command_rx, main_tx, read_interval_duration).await {
                error!("Modem task error: {}", e);
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

    async fn modem_task(
        port: Arc<Mutex<SerialStream>>,
        mut command_rx: mpsc::Receiver<OutgoingCommand>,
        main_tx: mpsc::UnboundedSender<ModemIncomingMessage>,
        read_interval_duration: Duration
    ) -> Result<()> {
        let mut state_machine = ModemStateMachine::default();
        let mut line_buffer = LineBuffer::new();

        let mut read_interval = interval(read_interval_duration);
        let mut timeout_interval = interval(Duration::from_secs(1));

        info!("Started ModemManager socket handler");
        loop {
            tokio::select! {
                Some(mut cmd) = command_rx.recv(), if state_machine.can_accept_command() => {
                    debug!("Received new command sequence {}: {:?}", cmd.sequence, cmd.request);

                    match ModemEventHandlers::command_sender(&port, &cmd.request).await {
                        Ok(state) => {
                            state_machine.start_command(cmd, state);
                        }
                        Err(e) => {
                            error!("Failed to send command sequence {}: {}", cmd.sequence, e);
                            cmd.respond(ModemResponse::Error {
                                message: format!("Failed to send command: {}", e)
                            }).await?
                        }
                    }
                },

                // Response reader.
                _ = read_interval.tick() => {
                    let new_data = {
                        let mut port_guard = port.lock().await;
                        let mut buf = [0u8; 1024];
                        match port_guard.try_read(&mut buf) {
                            Ok(n) if n > 0 => Some(String::from_utf8_lossy(&buf[..n]).to_string()),
                            _ => None,
                        }
                    };

                    if let Some(data) = new_data {
                        debug!("Received raw: {:?}", data);

                        // Process all complete lines/prompts from the buffer.
                        for line_event in line_buffer.process_data(&data) {
                            match state_machine.transition_state(
                                line_event,
                                &main_tx,
                                &port
                            ).await {
                                Ok(_) => { },
                                Err(e) => {
                                    error!("Error processing modem event: {:?}", e);
                                    state_machine.reset_to_idle();
                                }
                            }
                        }
                    }
                },
                
                // Command timeout.
                _ = timeout_interval.tick() => {
                    if let Err(e) = state_machine.handle_command_timeout().await {
                        error!("Error while handling command timeout: {:?}", e);
                    }
                }
            }
        }
    }

    async fn read_response_until_ok(port: &Arc<Mutex<SerialStream>>) -> Result<Vec<u8>> {
        let mut response = Vec::new();
        let mut buf = [0u8; 1024];

        tokio::time::timeout(
            Duration::from_secs(5),
            async {
                loop {
                    let read_result = {
                        let mut port_guard = port.lock().await;
                        port_guard.try_read(&mut buf)
                    };

                    match read_result {
                        Ok(n) if n > 0 => {
                            response.extend_from_slice(&buf[..n]);
                            let response_str = String::from_utf8_lossy(&response);

                            // Look for OK or ERROR termination
                            if response_str.contains("OK\r\n") || response_str.contains("ERROR") {
                                break;
                            }
                        }
                        Ok(_) => tokio::time::sleep(tokio::time::Duration::from_millis(50)).await,
                        Err(_) => tokio::time::sleep(tokio::time::Duration::from_millis(50)).await,
                    }
                }
            }
        ).await.map_err(|_| anyhow!("Timeout waiting for response"))?;

        Ok(response)
    }
}