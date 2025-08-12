use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::oneshot;
use anyhow::{anyhow, bail, Result};
use tracing::log::{debug, error};
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
    response_tx: Option<oneshot::Sender<ModemResponse>>,
}
impl OutgoingCommand {
    pub fn new(sequence: u32, request: ModemRequest, response_tx: oneshot::Sender<ModemResponse>) -> Self {
        Self {
            sequence,
            request,
            response_tx: Some(response_tx)
        }
    }

    pub async fn respond(&mut self, response: ModemResponse) -> Result<()> {
        debug!("Attempting to respond to command #{} with: {:?}", self.sequence, response);

        if let Some(tx) = self.response_tx.take() {
            debug!("Sending response via oneshot channel for command #{}", self.sequence);
            match tx.send(response) {
                Ok(_) => {
                    debug!("Successfully sent response for command #{}", self.sequence);
                    Ok(())
                },
                Err(response) => {
                    error!("Failed to send response for command #{} - receiver likely dropped. Response was: {:?}", self.sequence, response);
                    Err(anyhow!("Failed to respond to command #{} with: {:?}", self.sequence, response))
                }
            }
        } else {
            error!("Attempted to respond to command #{} but response channel was already used", self.sequence);
            bail!("Attempted to respond to command #{} but response channel was already used", self.sequence);
        }
    }
}