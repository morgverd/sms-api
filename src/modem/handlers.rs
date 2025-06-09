use std::sync::Arc;
use anyhow::Result;
use log::{debug, info};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio_serial::SerialStream;
use crate::modem::commands::CommandState;
use crate::modem::types::{ModemRequest, ModemResponse, ReceivedSMSMessage};

pub async fn command_sender(
    port: &Arc<Mutex<SerialStream>>,
    request: &ModemRequest
) -> Result<CommandState> {
    match request {
        ModemRequest::SendSMS { len, .. } => {
            debug!("Sending CMGS length {} for SendSMS!", len);
            {
                let mut port_guard = port.lock().await;
                port_guard.write_all(format!("AT+CMGS={}\r\n", len).as_bytes()).await?;
            }
            return Ok(CommandState::WaitingForPrompt);
        }
        ModemRequest::GetNetworkStatus => {
            let mut port_guard = port.lock().await;
            port_guard.write_all(b"AT+CREG?\r\n").await?;
        }
        ModemRequest::GetSignalStrength => {
            let mut port_guard = port.lock().await;
            port_guard.write_all(b"AT+CSQ\r\n").await?;
        }
    }

    Ok(CommandState::WaitingForData)
}

pub async fn prompt_handler(
    port: &Arc<Mutex<SerialStream>>,
    request: &ModemRequest
) -> Result<Option<CommandState>> {

    if let ModemRequest::SendSMS { len, pdu } = request {
        info!("Sending PDU: len = {}, pdu = {}", len, pdu);
        {
            let mut port_guard = port.lock().await;
            port_guard.write_all(pdu.as_bytes()).await?;
            port_guard.write_all(b"\x1A").await?;
        }
        return Ok(Some(CommandState::WaitingForOk))
    }

    Ok(None)
}

pub async fn handle_incoming_sms(
    content: &str
) -> Result<Option<ReceivedSMSMessage>> {
    Ok(None)
}

pub async fn command_responder(
    request: &ModemRequest,
    response: &String
) -> Result<ModemResponse> {
    info!("Command response: {:?} -> {:?}", request, response);
    Ok(ModemResponse::Error { message: response.to_string() })
}