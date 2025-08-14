use std::time::Duration;
use anyhow::{anyhow, Result};
use tracing::log::{debug, error, info, warn};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::interval;
use tokio_serial::SerialStream;
use crate::config::ModemConfig;
use crate::modem::buffer::LineBuffer;
use crate::modem::commands::OutgoingCommand;
use crate::modem::state_machine::ModemStateMachine;
use crate::modem::types::{ModemIncomingMessage, ModemResponse, ModemStatus};

macro_rules! init_cmd {
    ($cmd:expr, $resp:expr) => {
        ($cmd.to_string(), $resp.as_bytes().to_vec())
    };
}

#[derive(Debug)]
pub enum WorkerEvent {
    SetStatus(ModemStatus),
    WriteCommand(Vec<u8>),
}

pub struct ModemWorker {
    port: SerialStream,
    status: ModemStatus,
    state_machine: ModemStateMachine,
    read_buffer: Box<[u8]>,
    main_tx: mpsc::UnboundedSender<ModemIncomingMessage>,
    worker_event_rx: mpsc::UnboundedReceiver<WorkerEvent>,
    config: ModemConfig
}
impl ModemWorker {
    pub fn new(port: SerialStream, main_tx: mpsc::UnboundedSender<ModemIncomingMessage>, config: ModemConfig) -> Self {
        let (worker_event_tx, worker_event_rx) = mpsc::unbounded_channel();

        Self {
            port,
            status: ModemStatus::Startup,
            state_machine: ModemStateMachine::new(worker_event_tx),
            read_buffer: vec![0u8; config.read_buffer_size].into_boxed_slice(),
            main_tx,
            worker_event_rx,
            config
        }
    }

    pub async fn initialize_and_run(
        mut self,
        command_rx: mpsc::Receiver<OutgoingCommand>
    ) -> Result<()> {
        match self.initialize_modem().await {
            Ok(()) => {
                info!("Modem initialized successfully");
                self.set_status(ModemStatus::Online);
            }
            Err(e) => {
                error!("Failed to initialize modem: {}", e);
                self.set_status(ModemStatus::Offline);
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

    pub async fn run(
        mut self,
        mut command_rx: mpsc::Receiver<OutgoingCommand>
    ) -> Result<()> {
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
                                    self.set_status(ModemStatus::Offline);
                                },
                                Ok(n) => {
                                    let main_tx = &self.main_tx;
                                    for line_event in line_buffer.process_data(&self.read_buffer[..n]) {
                                        if let Err(e) = self.state_machine.transition_state(main_tx, line_event).await {
                                            error!("Error processing modem event: {:?}", e);
                                            self.state_machine.reset_to_idle();
                                        }
                                    }
                                },
                                Err(e) => {
                                    error!("Read error: {}", e);
                                    self.set_status(ModemStatus::Offline);
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
                        let _ = cmd.respond(ModemResponse::Error("Modem is shutting down".to_string())).await;
                    }

                    // Wait a bit then transition to offline
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    self.set_status(ModemStatus::Offline);
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
                            let _ = cmd.respond(ModemResponse::Error("Modem is offline".to_string())).await;
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
                },
                _ => debug!("Cannot run ModemStatus: {:?}", self.status)
            }
        }
    }

    async fn handle_worker_event(&mut self, event: WorkerEvent) -> Result<()> {
        match event {
            WorkerEvent::SetStatus(status) => self.set_status(status),
            WorkerEvent::WriteCommand(data) => {
                if let Err(e) = self.write(&data).await {
                    error!("Failed to write command: {}", e);
                    self.set_status(ModemStatus::Offline);
                }
            }
        }
        Ok(())
    }

    fn set_status(&mut self, status: ModemStatus) {
        debug!("ModemWorker Status: {:?}", status);
        if self.status == status {
            return;
        }

        let previous = self.status.clone();
        self.status = status.clone();

        // Send message outside of modem for webhooks etc.
        let message = ModemIncomingMessage::ModemStatusUpdate {
            previous,
            current: status.clone()
        };
        match self.main_tx.send(message) {
            Ok(_) => debug!("Sent ModemOnlineStatusUpdate, Status: {:?}", status),
            Err(e) => error!("Failed to send ModemOnlineStatusUpdate, Status: {:?}, Error: {}", status, e)
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
                        self.set_status(ModemStatus::Online);
                        Ok(true)
                    }
                    Err(e) => {
                        error!("Reconnection failed during initialization: {}", e);
                        Ok(false)
                    }
                }
            }
            Err(e) => {
                debug!("Basic connection test failed: {}", e);
                Ok(false)
            }
        }
    }

    async fn initialize_modem(&mut self) -> Result<()> {
        let mut initialization_commands: Vec<(String, Vec<u8>)> = vec![
            init_cmd!("ATZ\r\n", "OK"),                // Reset
            init_cmd!("AT\r\n", "OK"),                 // Test connection
            init_cmd!("ATE0\r\n", "OK"),               // Disable echo
            init_cmd!("AT+CMGF=0\r\n", "OK"),          // Set SMS message format to PDU
            init_cmd!("AT+CSCS=\"GSM\"\r\n", "OK"),    // Use GSM 7-bit alphabet
            init_cmd!("AT+CNMI=2,2,0,1,0\r\n", "OK"),  // Receive all incoming SMS messages and delivery reports
            init_cmd!("AT+CSMP=49,167,0,0\r\n", "OK"), // Receive delivery receipts from sent messages
            init_cmd!("AT+CPMS=\"ME\",\"ME\",\"ME\"\r\n", "+CPMS:") // Store all messages in memory only
        ];

        // If GNSS is enabled power it on and start its receiver.
        if self.config.gnss_enabled {
            info!("The GNSS module is enabled with a report interval of {}! Powering on...", self.config.gnss_report_interval);
            initialization_commands.push(init_cmd!("AT+CGNSPWR=1\r\n", "OK")); // Power on
            initialization_commands.push(init_cmd!("AT+CGPSRST=0\r\n", "OK")); // Cold start

            // Create GNSS report interval command (0 = disabled).
            let interval_command= format!("AT+CGNSURC={}\r\n", self.config.gnss_report_interval);
            initialization_commands.push((interval_command, b"OK".to_vec())); // Set navigation URC report interval
        }

        for (command, expected) in initialization_commands {
            debug!("Sending initialization command: {}", command.trim());

            self.port.write_all(command.as_bytes()).await?;

            let response = self.read_response_until_ok().await?;
            let response_str = String::from_utf8_lossy(&response);
            let expected_str = String::from_utf8_lossy(&*expected);

            debug!("Response: {}", response_str.trim());
            if !response_str.contains(&*expected_str) {
                return Err(anyhow!(
                    "Initialization command '{}' failed. Expected: '{}', Got: '{}'",
                    command.trim(), expected_str, response_str.trim()
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
            Duration::from_secs(10),
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