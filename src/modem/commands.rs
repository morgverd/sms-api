use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::{Mutex, oneshot};
use tokio::time::Instant;
use anyhow::{anyhow, Result};
use log::{debug, error, warn};
use crate::modem::types::{ModemRequest, ModemResponse};

static COMMAND_SEQUENCE: AtomicU32 = AtomicU32::new(1);

pub fn next_command_sequence() -> u32 {
    COMMAND_SEQUENCE.fetch_add(1, Ordering::SeqCst)
}

#[derive(Debug)]
pub struct CommandContext {
    pub sequence: u32,
    pub state: CommandState,
    pub response_buffer: String
}

#[derive(Debug, Clone)]
pub enum CommandState {
    WaitingForOk,
    WaitingForPrompt,
    WaitingForData
}
impl CommandState {
    pub fn is_complete(&self, content: &str) -> bool {
        match self {
            CommandState::WaitingForOk => {
                content == "OK" || content == "ERROR" ||
                    content.starts_with("+CME ERROR:") || content.starts_with("+CMS ERROR:")
            }
            CommandState::WaitingForPrompt => false,
            CommandState::WaitingForData => {
                // For SMS, look for the confirmation
                content.starts_with("+CMGS:") || content == "OK" || content == "ERROR"
            }
        }
    }
}

#[derive(Debug)]
pub struct OutgoingCommand {
    pub sequence: u32,
    pub request: ModemRequest,
    response_tx: Arc<Mutex<Option<oneshot::Sender<ModemResponse>>>>,
    timestamp: Instant
}
impl OutgoingCommand {
    pub fn new(sequence: u32, request: ModemRequest, response_tx: oneshot::Sender<ModemResponse>) -> Self {
        Self {
            sequence,
            request,
            response_tx: Arc::new(Mutex::new(Some(response_tx))),
            timestamp: Instant::now(),
        }
    }

    pub fn is_expired(&self, timeout_secs: u64) -> bool {
        self.timestamp.elapsed().as_secs() > timeout_secs
    }

    pub async fn respond(&self, response: ModemResponse) {
        let mut tx_guard = self.response_tx.lock().await;
        if let Some(tx) = tx_guard.take() {
            if let Err(response) = tx.send(response) {
                error!("Failed to send response as receiver was dropped: {:?}", response);
            }
        } else {
            warn!("Attempted to respond to command {} but response channel was already used", self.sequence);
        }
    }
}

#[derive(Debug)]
pub struct CommandTracker {
    active_command: Option<OutgoingCommand>,
}
impl CommandTracker {
    pub fn new() -> Self {
        Self {
            active_command: None
        }
    }

    pub fn is_idle(&self) -> bool {
        self.active_command.is_none()
    }

    pub fn get_active_command(&self) -> Option<&OutgoingCommand> {
        self.active_command.as_ref()
    }

    pub async fn complete_active_command(&mut self, response: ModemResponse) -> Result<()> {
        if let Some(cmd) = self.active_command.take() {
            cmd.respond(response).await;
            Ok(())
        } else {
            Err(anyhow::anyhow!("No active command to respond to"))
        }
    }

    pub fn start_command(&mut self, cmd: OutgoingCommand) -> Result<()> {
        if !self.is_idle() {
            return Err(anyhow!("Cannot start command - another command is active"));
        }

        debug!("Starting OutgoingCommand {}: {:?}", cmd.sequence, cmd.request);
        self.active_command = Some(cmd);
        Ok(())
    }

    pub fn is_command_expired(&self, timeout_secs: u64) -> bool {
        self.active_command.as_ref()
            .map(|cmd| cmd.is_expired(timeout_secs))
            .unwrap_or(false)
    }

    pub fn force_timeout_active_command(&mut self) -> Option<OutgoingCommand> {
        if self.is_command_expired(30) {
            warn!("Force timing out expired command: {:?}",
                  self.active_command.as_ref().map(|c| (&c.sequence, &c.request)));
            self.active_command.take()
        } else {
            None
        }
    }
}
