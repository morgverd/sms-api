use std::str::FromStr;
use anyhow::anyhow;
use log::{debug, error, info};
use tokio::sync::{oneshot, mpsc};
use anyhow::Result;
use huawei_modem::gsm_encoding::GsmMessageData;
use huawei_modem::pdu::{MessageType, Pdu, PduAddress, PduFirstOctet, VpFieldValidity};
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

    /// https://github.com/eeeeeta/huawei-modem/issues/24
    pub async fn send_sms(&mut self, to: String, content: &str) -> Result<ModemResponse> {
        let mut last_response = None;

        for part in GsmMessageData::encode_message(content) {

            // FIXME: This is horrendous, the address is being re-parsed for each split message
            //  because the PDU lib doesn't allow a PduFirstOctet to be directly initialized.
            let address = PduAddress::from_str(&to)?;
            let pdu = Pdu::make_simple_message(address, part);

            let (bytes, size) = pdu.as_bytes();
            let request = ModemRequest::SendSMS {
                pdu: hex::encode(bytes),
                len: size,
            };

            let response = Some(self.send_command(request).await?);
            last_response = response;
        }
        last_response.ok_or_else(|| anyhow!("There is no final response!"))
    }

    pub async fn send_command(&mut self, request: ModemRequest) -> Result<ModemResponse> {
        let sequence = next_command_sequence();
        let (tx, rx) = oneshot::channel();

        let cmd = OutgoingCommand::new(sequence, request, tx);
        debug!("Queuing command sequence {}: {:?}", sequence, cmd.request);

        // Send to the modem task.
        self.command_tx.send(cmd)
            .map_err(|_| anyhow!("Failed to queue command - modem task may be dead"))?;

        // Wait for response with timeout.
        match tokio::time::timeout(tokio::time::Duration::from_secs(60), rx).await {
            Ok(Ok(response)) => {
                debug!("Command sequence {} completed!", sequence);
                Ok(response)
            }
            Ok(Err(e)) => {
                error!("{:?}", e);
                Err(anyhow!("Command sequence {} response channel closed", sequence))
            },
            Err(_) => Err(anyhow!("Command sequence {} timed out waiting for response", sequence)),
        }
    }
}