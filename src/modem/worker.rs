use std::time::{Duration, Instant};
use anyhow::{anyhow, Result};
use log::{debug, error, info, warn};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::interval;
use tokio_serial::SerialStream;
use crate::config::ModemConfig;
use crate::modem::buffer::LineBuffer;
use crate::modem::commands::OutgoingCommand;
use crate::modem::state_machine::ModemStateMachine;
use crate::modem::types::{ModemIncomingMessage, ModemResponse};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModemStatus {
    Online,
    ShuttingDown,
    Offline
}

#[derive(Debug)]
pub enum WorkerEvent {
    SetStatus(ModemStatus),
    WriteCommand(Vec<u8>),
}

pub struct ModemWorker {
    port: SerialStream,
    status: ModemStatus,
    offline_since: Option<Instant>,
    state_machine: ModemStateMachine,
    read_buffer: Box<[u8]>,
    worker_event_rx: mpsc::UnboundedReceiver<WorkerEvent>,
    config: ModemConfig
}
impl ModemWorker {
    pub fn new(port: SerialStream, main_tx: mpsc::UnboundedSender<ModemIncomingMessage>, config: ModemConfig) -> Self {
        let (worker_event_tx, worker_event_rx) = mpsc::unbounded_channel();

        Self {
            port,
            status: ModemStatus::Online,
            offline_since: None,
            state_machine: ModemStateMachine::new(main_tx, worker_event_tx),
            read_buffer: vec![0u8; config.read_buffer_size].into_boxed_slice(),
            worker_event_rx,
            config
        }
    }

    pub async fn initialize_and_run(mut self, command_rx: mpsc::Receiver<OutgoingCommand>) -> Result<()> {
        match self.initialize_modem().await {
            Ok(()) => {
                info!("Modem initialized successfully");
                self.set_online();
            }
            Err(e) => {
                error!("Failed to initialize modem: {}", e);
                self.set_offline();
            }
        }
        self.run(command_rx).await
    }

    pub async fn write(&mut self, data: &[u8]) -> Result<()> {
        if self.status != ModemStatus::Online {
            return Err(anyhow!("Modem is offline"));
        }
        self.port.write_all(data)
            .await
            .map_err(|e| anyhow!(e))
    }

    pub async fn run(mut self, mut command_rx: mpsc::Receiver<OutgoingCommand>) -> Result<()> {
        let mut line_buffer = LineBuffer::with_max_size(self.config.line_buffer_size);

        let mut timeout_interval = interval(Duration::from_secs(1));
        let mut reconnect_interval = interval(Duration::from_secs(30));

        info!("Started ModemWorker");
        loop {
            match self.status {

                ModemStatus::Online => {
                    tokio::select! {
                        biased;

                        // Handle internal worker events
                        Some(event) = self.worker_event_rx.recv() => {
                            if let Err(e) = self.handle_worker_event(event).await {
                                error!("Error handling worker event: {}", e);
                            }
                        },

                        // Accept commands when online and state machine is ready
                        Some(cmd) = command_rx.recv(), if self.state_machine.can_accept_command() => {
                            debug!("Received new command sequence {}: {:?}", cmd.sequence, cmd.request);
                            if let Err(e) = self.state_machine.start_command(cmd).await {
                                error!("Failed to start command: {}", e);
                            }
                        },

                        // Main reader.
                        result = self.port.read(&mut self.read_buffer) => {
                            match result {
                                Ok(0) => {
                                    warn!("Serial port closed, going offline");
                                    self.set_offline();
                                },
                                Ok(n) => {
                                    for line_event in line_buffer.process_data(&self.read_buffer[..n]) {
                                        if let Err(e) = self.state_machine.transition_state(line_event).await {
                                            error!("Error processing modem event: {:?}", e);
                                            self.state_machine.reset_to_idle();
                                        }
                                    }
                                },
                                Err(e) => {
                                    error!("Read error: {}", e);
                                    self.set_offline();
                                }
                            }
                        },

                        // Command timeout handling
                        _ = timeout_interval.tick() => {
                            let timed_out = self.state_machine.handle_command_timeout()
                                .await
                                .unwrap_or_else(|e| {
                                    error!("Error while handling command timeout: {:?}", e);
                                    true
                                });

                            if timed_out {
                                line_buffer.clear();
                            }
                        }
                    }
                },
                ModemStatus::ShuttingDown => {
                    // Process any pending worker events
                    while let Ok(event) = self.worker_event_rx.try_recv() {
                        if let Err(e) = self.handle_worker_event(event).await {
                            error!("Error handling worker event during shutdown: {}", e);
                        }
                    }

                    // Reject any pending commands
                    while let Ok(mut cmd) = command_rx.try_recv() {
                        let _ = cmd.respond(ModemResponse::Error {
                            message: "Modem is shutting down".to_string()
                        }).await;
                    }

                    // Wait a bit then transition to offline
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    self.set_offline();
                    self.state_machine.reset_to_idle();
                    line_buffer.clear();
                },
                ModemStatus::Offline => {
                    tokio::select! {
                        // Still process worker events when offline
                        Some(event) = self.worker_event_rx.recv() => {
                            if let Err(e) = self.handle_worker_event(event).await {
                                error!("Error handling worker event while offline: {}", e);
                            }
                        },

                        // Reject commands immediately when offline
                        Some(mut cmd) = command_rx.recv() => {
                            let _ = cmd.respond(ModemResponse::Error {
                                message: "Modem is offline".to_string()
                            }).await;
                        },

                        // Attempt reconnection
                        _ = reconnect_interval.tick() => {
                            match self.try_reconnect().await {
                                Ok(true) => {
                                    info!("Successfully reconnected to modem");
                                    self.state_machine.reset_to_idle();
                                    line_buffer.clear();
                                },
                                Ok(false) => { },
                                Err(e) => {
                                    error!("Error during reconnection attempt: {}", e);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    async fn handle_worker_event(&mut self, event: WorkerEvent) -> Result<()> {
        match event {
            WorkerEvent::SetStatus(status) => {
                match status {
                    ModemStatus::Online => self.set_online(),
                    ModemStatus::ShuttingDown => self.set_shutting_down(),
                    ModemStatus::Offline => self.set_offline(),
                }
            },
            WorkerEvent::WriteCommand(data) => {
                if let Err(e) = self.write(&data).await {
                    error!("Failed to write command: {}", e);
                    self.set_offline();
                }
            }
        }
        Ok(())
    }

    fn set_shutting_down(&mut self) {
        if self.status == ModemStatus::Online {
            warn!("Modem is shutting down!");
            self.status = ModemStatus::ShuttingDown;
        }
    }

    fn set_offline(&mut self) {
        if self.status != ModemStatus::Offline {
            error!("Modem is now offline");
            self.status = ModemStatus::Offline;
            self.offline_since = Some(Instant::now());
        }
    }

    fn set_online(&mut self) {
        if self.status != ModemStatus::Online {
            info!("Modem is back online");
            self.status = ModemStatus::Online;
            self.offline_since = None;
        }
    }

    async fn try_reconnect(&mut self) -> Result<bool> {
        if self.status != ModemStatus::Offline {
            return Ok(false);
        }

        match self.test_connection().await {
            Ok(()) => {
                debug!("Basic connection test passed, initializing modem...");

                // Re-initialize the modem after reconnection
                match self.initialize_modem().await {
                    Ok(()) => {
                        info!("Modem reconnected and reinitialized successfully");
                        self.set_online();
                        Ok(true)
                    }
                    Err(e) => {
                        error!("Reconnection failed during initialization: {}", e);
                        self.offline_since = Some(Instant::now());
                        Ok(false)
                    }
                }
            }
            Err(e) => {
                debug!("Basic connection test failed: {}", e);
                self.offline_since = Some(Instant::now());
                Ok(false)
            }
        }
    }

    async fn initialize_modem(&mut self) -> Result<()> {
        let initialization_commands: Vec<(&[u8], &[u8])> = vec![
            (b"ATZ\r\n", b"OK"),                // Reset
            (b"AT\r\n", b"OK"),                 // Test connection
            (b"ATE0\r\n", b"OK"),               // Disable echo
            (b"AT+CMGF=0\r\n", b"OK"),          // Set SMS message format to PDU
            (b"AT+CSCS=\"GSM\"\r\n", b"OK"),    // Use GSM 7-bit alphabet
            (b"AT+CNMI=2,2,0,1,0\r\n", b"OK"),  // Receive all incoming SMS messages and delivery reports
            (b"AT+CSMP=49,167,0,0\r\n", b"OK"), // Receive delivery receipts from sent messages
            (b"AT+CPMS=\"ME\",\"ME\",\"ME\"\r\n", b"+CPMS:") // Store all messages in memory only
        ];

        for (command, expected) in initialization_commands {
            let command_str = String::from_utf8_lossy(command);
            debug!("Sending initialization command: {}", command_str.trim());

            self.port.write_all(command).await?;

            let response = self.read_response_until_ok().await?;
            let response_str = String::from_utf8_lossy(&response);
            let expected_str = String::from_utf8_lossy(expected);

            debug!("Response: {}", response_str.trim());

            if !response_str.contains(&*expected_str) {
                return Err(anyhow!(
                    "Initialization command '{}' failed. Expected: '{}', Got: '{}'",
                    command_str.trim(), expected_str, response_str.trim()
                ));
            }
        }

        debug!("Modem initialization completed successfully!");
        Ok(())
    }

    async fn read_response_until_ok(&mut self) -> Result<Vec<u8>> {
        let mut response = Vec::new();
        let mut buf = [0u8; 1024];

        let timeout = Duration::from_millis(50);
        tokio::time::timeout(
            Duration::from_secs(5),
            async {
                loop {
                    match self.port.try_read(&mut buf) {
                        Ok(n) if n > 0 => {
                            response.extend_from_slice(&buf[..n]);
                            let response_str = String::from_utf8_lossy(&response);

                            if response_str.contains("OK\r\n") || response_str.contains("ERROR") {
                                break;
                            }
                        }
                        Ok(_) => tokio::time::sleep(timeout).await,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            tokio::time::sleep(timeout).await
                        },
                        Err(e) => return Err(anyhow!("Read error during initialization: {}", e)),
                    }
                }
                Ok(())
            }
        ).await.map_err(|_| anyhow!("Timeout waiting for response"))??;

        Ok(response)
    }

    async fn test_connection(&mut self) -> Result<()> {
        self.port.write_all(b"AT\r\n").await?;

        let mut buf = [0u8; 64];
        let timeout = Duration::from_secs(2);

        tokio::time::timeout(timeout, async {
            loop {
                match self.port.try_read(&mut buf) {
                    Ok(n) if n > 0 => {
                        let response = String::from_utf8_lossy(&buf[..n]);
                        if response.contains("OK") {
                            return Ok(());
                        }
                    }
                    Ok(_) => tokio::time::sleep(Duration::from_millis(100)).await,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        tokio::time::sleep(Duration::from_millis(100)).await
                    },
                    Err(e) => return Err(anyhow!("Connection test error: {}", e)),
                }
            }
        }).await.map_err(|_| anyhow!("Timeout testing connection"))?
    }
}