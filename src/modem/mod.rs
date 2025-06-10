use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, oneshot};
use anyhow::{anyhow, bail, Context, Result};
use tokio::io::{AsyncWriteExt};
use tokio_serial::{SerialPortBuilderExt, SerialStream};
use log::{debug, error, warn};
use crate::modem::buffer::LineBuffer;
use crate::modem::commands::{CommandContext, CommandTracker, next_command_sequence};
use crate::modem::handlers::command_sender;
use crate::modem::commands::OutgoingCommand;
use crate::modem::types::{
    ModemConfig,
    ModemReadState,
    ModemRequest,
    ModemResponse,
    ModemIncomingMessage
};

pub mod types;
mod handlers;
mod state_machine;
pub mod commands;
mod buffer;

#[derive(Clone)]
pub struct ModemManager {
    config: ModemConfig,
    main_tx: mpsc::UnboundedSender<ModemIncomingMessage>,
    command_tx: Option<mpsc::UnboundedSender<OutgoingCommand>>
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
        let (command_tx, command_rx) = mpsc::unbounded_channel();
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
                    "Command '{}' failed. Expected: '{}', Got: '{}'",
                    command_str, expected_str, response_str.trim()
                );
            }
        }
        debug!("Initialized modem successfully!");

        // Spawn the main modem handling task.
        // This is passed back to the main thread to be joined with the HTTP server.
        let main_tx = self.main_tx.clone();
        let handle = tokio::spawn(async move {
            if let Err(e) = Self::modem_task(port, command_rx, main_tx).await {
                error!("Modem task error: {}", e);
            }
        });

        Ok(handle)
    }

    pub async fn send_command(&mut self, request: ModemRequest) -> Result<ModemResponse> {
        if let Some(command_tx) = &self.command_tx {
            let sequence = next_command_sequence();
            let (tx, rx) = oneshot::channel();

            let cmd = OutgoingCommand::new(sequence, request, tx);
            debug!("Queuing command sequence {}: {:?}", sequence, cmd.request);

            // Send to the modem task
            command_tx.send(cmd)
                .map_err(|_| anyhow!("Failed to queue command - modem task may be dead"))?;

            // Wait for response with timeout
            match tokio::time::timeout(tokio::time::Duration::from_secs(60), rx).await {
                Ok(Ok(response)) => {
                    debug!("Command sequence {} completed: {:?}", sequence, response);
                    Ok(response)
                }
                Ok(Err(e)) => {
                    error!("{:?}", e);
                    Err(anyhow!("Command sequence {} response channel closed", sequence))
                },
                Err(_) => Err(anyhow!("Command sequence {} timed out waiting for response", sequence)),
            }
        } else {
            Err(anyhow!("Cannot send commands via an unopened channel!"))
        }
    }

    async fn modem_task(
        port: Arc<Mutex<SerialStream>>,
        mut command_rx: mpsc::UnboundedReceiver<OutgoingCommand>,
        main_tx: mpsc::UnboundedSender<ModemIncomingMessage>,
    ) -> Result<()> {
        let mut read_state = ModemReadState::Idle;
        let mut line_buffer = LineBuffer::new();
        let mut command_tracker = CommandTracker::new();

        loop {
            tokio::select! {
                // Command sender: only pick up if idle.
                Some(cmd) = command_rx.recv(), if matches!(read_state, ModemReadState::Idle) && command_tracker.active_command.is_none() => {
                    debug!("Received new command sequence {}: {:?}", cmd.sequence, cmd.request);

                    match command_sender(&port, &cmd.request).await {
                        Ok(state) => {
                            // Properly track the command
                            if let Err(e) = command_tracker.start_command(cmd) {
                                error!("Failed to start command tracking: {}", e);
                                continue;
                            }

                            // Get the command back from tracker to use in read_state
                            if let Some(tracked_cmd) = &command_tracker.active_command {
                                let ctx = CommandContext {
                                    cmd: tracked_cmd.clone(),
                                    state,
                                    response_buffer: String::new()
                                };
                                read_state = ModemReadState::Command(ctx);
                            }

                            debug!("Started tracking command sequence {}",
                                   command_tracker.get_active_sequence().unwrap_or(0));
                        }
                        Err(e) => {
                            error!("Failed to send command sequence {}: {}", cmd.sequence, e);
                            cmd.respond(ModemResponse::Error {
                                message: format!("Failed to send command: {}", e)
                            }).await;
                        }
                    }
                },

                // Response reader.
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(50)) => {
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
                            match ModemManager::process_modem_event(read_state, line_event, &main_tx, &port, &mut command_tracker).await {
                                Ok(new_state) => {
                                    read_state = new_state;
                                }
                                Err(e) => {
                                    error!("Error processing modem event: {:?}", e);
                                    read_state = ModemReadState::Idle;
                                }
                            }
                        }
                    }
                }

                // Timeout handling for commands that take too long
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(30)), if matches!(read_state, ModemReadState::Command(_)) => {
                    // Check both the read_state and command_tracker for consistency
                    if let ModemReadState::Command(ctx) = &read_state {
                        warn!("Command {} timed out", ctx.cmd.sequence);
                    }

                    // Use the command tracker's timeout logic
                    if let Some(expired_cmd) = command_tracker.force_timeout_active_command() {
                        expired_cmd.respond(ModemResponse::Error {
                            message: "Command timeout".to_string(),
                        }).await;
                    }

                    read_state = ModemReadState::Idle;
                }
            }
        }
    }

    async fn read_response_until_ok(port: &Arc<Mutex<SerialStream>>) -> Result<Vec<u8>> {
        let mut response = Vec::new();
        let mut buf = [0u8; 1024];

        tokio::time::timeout(
            tokio::time::Duration::from_secs(5),
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