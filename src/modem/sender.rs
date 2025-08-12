use std::time::Duration;
use anyhow::{anyhow, bail};
use tracing::log::{debug, error};
use tokio::sync::{oneshot, mpsc};
use anyhow::Result;
use crate::modem::commands::{next_command_sequence, OutgoingCommand};
use crate::modem::types::{ModemRequest, ModemResponse};

const SEND_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Clone)]
pub struct ModemSender {
    command_tx: mpsc::Sender<OutgoingCommand>
}
impl ModemSender {
    pub fn new(command_tx: mpsc::Sender<OutgoingCommand>) -> Self {
        Self { command_tx }
    }

    pub async fn send_command(&self, request: ModemRequest) -> Result<ModemResponse> {
        let sequence = next_command_sequence();
        let (tx, rx) = oneshot::channel();

        let cmd = OutgoingCommand::new(sequence, request.clone(), tx);
        debug!("Queuing command sequence {}: {:?}", sequence, request);
        
        // Try to queue without blocking.
        match self.command_tx.try_send(cmd) {
            Ok(_) => debug!("Command sequence {} successfully queued", sequence),
            Err(mpsc::error::TrySendError::Full(_)) => bail!("Command queue is full! The modem may be overwhelmed."),
            Err(mpsc::error::TrySendError::Closed(_)) => bail!("Command queue is closed.")
        }
        
        // Wait for response with timeout.
        match tokio::time::timeout(SEND_TIMEOUT, rx).await {
            Ok(Ok(response)) => {
                debug!("Command sequence {} completed with response: {:?}", sequence, response);
                Ok(response)
            }
            Ok(Err(e)) => {
                error!("Command sequence {} response channel error: {:?}", sequence, e);
                Err(anyhow!("Command sequence {} response channel closed", sequence))
            },
            Err(_) => {
                error!("Command sequence {} timed out waiting for response", sequence);
                Err(anyhow!("Command sequence {} timed out waiting for response", sequence))
            }
        }
    }
}