use std::sync::Arc;
use anyhow::{anyhow, Result};
use log::{debug, info};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio_serial::SerialStream;
use huawei_modem::pdu::DeliverPdu;
use crate::modem::commands::CommandState;
use crate::modem::types::{
    ModemRequest,
    ModemResponse,
    ModemIncomingMessage,
    UnsolicitedMessageType
};

pub struct ModemEventHandlers;
impl ModemEventHandlers {

    pub async fn command_sender(
        port: &Arc<Mutex<SerialStream>>,
        request: &ModemRequest
    ) -> Result<CommandState> {
        match request {
            ModemRequest::SendSMS { len, .. } => {
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
            },
            ModemRequest::GetNetworkOperator => {
                let mut port_guard = port.lock().await;
                port_guard.write_all(b"AT+COPS?\r\n").await?;
            },
            ModemRequest::GetServiceProvider => {
                let mut port_guard = port.lock().await;
                port_guard.write_all(b"AT+CSPN?\r\n").await?;
            },
            ModemRequest::GetBatteryLevel => {
                let mut port_guard = port.lock().await;
                port_guard.write_all(b"AT+CBC\r\n").await?;
            }
        }

        Ok(CommandState::WaitingForData)
    }

    pub async fn prompt_handler(
        port: &Arc<Mutex<SerialStream>>,
        request: &ModemRequest
    ) -> Result<Option<CommandState>> {
        if let ModemRequest::SendSMS { len, pdu } = request {
            debug!("Sending PDU: len = {}", len);
            {
                let mut port_guard = port.lock().await;
                port_guard.write_all(pdu.as_bytes()).await?;
                port_guard.write_all(b"\x1A").await?;
            }
            return Ok(Some(CommandState::WaitingForOk))
        }

        Ok(None)
    }

    pub async fn handle_unsolicited_message(
        message_type: &UnsolicitedMessageType,
        header: &str,
        content: &str
    ) -> Result<Option<ModemIncomingMessage>> {
        match message_type {
            UnsolicitedMessageType::IncomingSMS => {

                // Decode SMS_DELIVER PDU into an IncomingSMS before sending through to main channel.
                // It is handled here to make sure errors are thrown back up to the ModemManager thread.
                let content_hex = hex::decode(content).map_err(|e| anyhow!(e))?;
                let deliver_pdu = DeliverPdu::try_from(&content_hex as &[u8]).map_err(|e| anyhow!(e))?;

                Ok(Some(ModemIncomingMessage::IncomingSMS {
                    phone_number: deliver_pdu.originating_address.to_string(),
                    content: deliver_pdu.get_message_data()
                        .decode_message()
                        .map_err(|e| anyhow!(e))?.text,
                    timestamp: 0 // TODO: Convert PDU SCTS back into a UNIX timestamp (u64)
                }))
            },
            UnsolicitedMessageType::IncomingCall => {
                Ok(Some(ModemIncomingMessage::IncomingCall))
            },
            UnsolicitedMessageType::DeliveryReport => {
                Ok(Some(ModemIncomingMessage::DeliveryReport {
                    id: content.to_string(),
                }))
            },
            UnsolicitedMessageType::NetworkStatusChange => {
                Ok(Some(ModemIncomingMessage::NetworkStatusChange {
                    status: 0
                }))
            }
        }
    }

    pub async fn command_responder(
        request: &ModemRequest,
        response: &String
    ) -> Result<ModemResponse> {
        info!("Command response: {:?} -> {:?}", request, response);
        
        match request {
            ModemRequest::SendSMS { .. } => {
                
                // >+CMGS: 123\nOK\n
                let reference_id: u8 = response
                    .strip_prefix(">+CMGS: ")
                    .and_then(|s| s.split('\n').next())
                    .ok_or(anyhow!("Modem response is malformed"))?
                    .trim()
                    .parse()
                    .map_err(|_| anyhow!("Invalid CMGS message reference number"))?;

                Ok(ModemResponse::SendResult { reference_id })
            },
            _ => {
                Ok(ModemResponse::Error { message: response.to_string() })
            }
        }
    }
}