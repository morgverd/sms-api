use std::sync::Arc;
use anyhow::{anyhow, Result};
use log::debug;
use pdu_rs::pdu::{DeliverPdu, StatusReportPdu};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio_serial::SerialStream;
use crate::modem::commands::CommandState;
use crate::modem::types::{
    ModemRequest,
    ModemResponse,
    ModemIncomingMessage,
    UnsolicitedMessageType
};
use crate::sms::types::{SMSIncomingDeliveryReport, SMSIncomingMessage};

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

                // Decode SMS_DELIVER PDU into an IncomingSMS.
                let content_hex = hex::decode(content).map_err(|e| anyhow!(e))?;
                let deliver_pdu = DeliverPdu::try_from(content_hex.as_slice()).map_err(|e| anyhow!(e))?;
                // TODO: Validate that the user_data len matches header specified size for a sanity check?

                let incoming = SMSIncomingMessage {
                    phone_number: deliver_pdu.originating_address.to_string(),
                    content: deliver_pdu.get_message_data()
                        .decode_message()
                        .map_err(|e| anyhow!(e))?.text
                };
                Ok(Some(ModemIncomingMessage::IncomingSMS(incoming)))
            },
            UnsolicitedMessageType::DeliveryReport => {

                // Decode SMS_STATUS_REPORT PDU into a DeliveryReport.
                let content_hex = hex::decode(content).map_err(|e| anyhow!(e))?;
                let status_report_pdu = StatusReportPdu::try_from(content_hex.as_slice()).map_err(|e| anyhow!(e))?;

                let report = SMSIncomingDeliveryReport {
                    status: status_report_pdu.status,
                    phone_number: status_report_pdu.recipient_address.to_string(),
                    reference_id: status_report_pdu.message_reference,
                };
                Ok(Some(ModemIncomingMessage::DeliveryReport(report)))
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
        debug!("Command response: {:?} -> {:?}", request, response);
        
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