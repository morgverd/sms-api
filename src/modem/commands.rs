use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::{Mutex, oneshot};
use tokio::time::Instant;
use anyhow::Result;
use log::{debug, error, warn};
use crate::modem::types::{ModemRequest, ModemResponse};

static COMMAND_SEQUENCE: AtomicU32 = AtomicU32::new(1);

pub fn next_command_sequence() -> u32 {
    COMMAND_SEQUENCE.fetch_add(1, Ordering::SeqCst)
}

#[derive(Debug)]
pub struct CommandContext {
    pub cmd: OutgoingCommand,
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

impl Clone for OutgoingCommand {
    fn clone(&self) -> Self {
        Self {
            sequence: self.sequence,
            request: self.request.clone(),
            response_tx: Arc::clone(&self.response_tx),
            timestamp: self.timestamp,
        }
    }
}


#[derive(Debug)]
pub struct CommandTracker {
    pub active_command: Option<OutgoingCommand>,
    pub command_history: Vec<(u32, String)>
}
impl CommandTracker {
    pub fn new() -> Self {
        Self {
            active_command: None,
            command_history: Vec::new(),
        }
    }

    pub fn start_command(&mut self, cmd: OutgoingCommand) -> Result<()> {
        if self.active_command.is_some() {
            return Err(anyhow::anyhow!("Cannot start command - another command is active"));
        }

        debug!("Starting command sequence {}: {:?}", cmd.sequence, cmd.request);
        self.command_history.push((cmd.sequence, format!("{:?}", cmd.request)));

        // Keep only last 10 commands in history
        if self.command_history.len() > 10 {
            self.command_history.remove(0);
        }

        self.active_command = Some(cmd);
        Ok(())
    }

    pub fn complete_command(&mut self) -> Option<OutgoingCommand> {
        if let Some(cmd) = self.active_command.take() {
            debug!("Completed command sequence {}: {:?}", cmd.sequence, cmd.request);
            Some(cmd)
        } else {
            warn!("Attempted to complete command but no active command found");
            None
        }
    }

    pub fn get_active_sequence(&self) -> Option<u32> {
        self.active_command.as_ref().map(|cmd| cmd.sequence)
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
