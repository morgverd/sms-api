use anyhow::anyhow;
use log::{debug, error};
use tokio::sync::{oneshot, mpsc};
use anyhow::Result;
use crate::modem::commands::{next_command_sequence, OutgoingCommand};
use crate::modem::types::{ModemRequest, ModemResponse};

#[derive(Clone)]
pub struct ModemSender {
    command_tx: mpsc::UnboundedSender<OutgoingCommand>
}
impl ModemSender {
    pub fn new(command_tx: mpsc::UnboundedSender<OutgoingCommand>) -> Self {
        Self { command_tx }
    }

    pub async fn send_command(&self, request: ModemRequest) -> Result<ModemResponse> {
        let sequence = next_command_sequence();
        let (tx, rx) = oneshot::channel();

        let cmd = OutgoingCommand::new(sequence, request.clone(), tx);
        debug!("Queuing command sequence {}: {:?}", sequence, request);

        // Send to the modem task.
        self.command_tx.send(cmd)
            .map_err(|_| anyhow!("Failed to queue command - modem task may be dead"))?;

        debug!("Command sequence {} sent to modem task, waiting for response...", sequence);

        // Wait for response with timeout.
        match tokio::time::timeout(tokio::time::Duration::from_secs(60), rx).await {
            Ok(Ok(response)) => {
                debug!("Command sequence {} completed with response: {:?}", sequence, response);
                Ok(response)
            }
            Ok(Err(e)) => {
                error!("Command sequence {} response channel error: {:?}", sequence, e);
                Err(anyhow!("Command sequence {} response channel closed", sequence))
            },
            Err(_) => {
                error!("Command sequence {} timed out waiting for response after 60 seconds", sequence);
                Err(anyhow!("Command sequence {} timed out waiting for response", sequence))
            }
        }
    }
}