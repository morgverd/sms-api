use std::sync::Arc;
use anyhow::{anyhow, Result};
use log::{debug, warn};
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
        content: &str
    ) -> Result<Option<ModemIncomingMessage>> {
        match message_type {
            UnsolicitedMessageType::IncomingSMS => {

                // Decode SMS_DELIVER PDU into an IncomingSMS.
                let content_hex = hex::decode(content).map_err(|e| anyhow!(e))?;
                let deliver_pdu = DeliverPdu::try_from(content_hex.as_slice()).map_err(|e| anyhow!(e))?;

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
            },
            UnsolicitedMessageType::ShuttingDown => {

                // TODO: Some IS_OFFLINE state should be updated, which blocks new API requests and creates a task
                //  that sends periodic AT requests to check for a connection. Once re-connected the modem must be
                //  re-initialized as it will echo requests etc.
                warn!("The modem is shutting down!");
                Ok(None)
            }
        }
    }

    pub async fn command_responder(
        request: &ModemRequest,
        response: &String
    ) -> Result<ModemResponse> {
        debug!("Command response: {:?} -> {:?}", request, response);

        // Validate response ends with OK
        if !response.trim_end().ends_with("OK") {
            return Err(anyhow!("Modem response does not end with OK"));
        }

        match request {
            ModemRequest::SendSMS { .. } => {
                // Find the CMGS response line in the buffer
                let cmgs_line = response
                    .lines()
                    .find(|line| line.trim().starts_with("+CMGS:"))
                    .ok_or(anyhow!("No CMGS response found in buffer"))?;

                let reference_id: u8 = cmgs_line
                    .trim()
                    .strip_prefix("+CMGS:")
                    .ok_or(anyhow!("Malformed CMGS response"))?
                    .trim()
                    .parse()
                    .map_err(|_| anyhow!("Invalid CMGS message reference number"))?;

                Ok(ModemResponse::SendResult { reference_id })
            },
            ModemRequest::GetNetworkStatus => {
                // Find the CREG response line in the buffer
                let creg_line = response
                    .lines()
                    .find(|line| line.trim().starts_with("+CREG:"))
                    .ok_or(anyhow!("No CREG response found in buffer"))?;

                let data = creg_line
                    .trim()
                    .strip_prefix("+CREG:")
                    .ok_or(anyhow!("Malformed CREG response"))?
                    .trim();

                let mut parts = data.split(',');
                let registration: u8 = parts
                    .next()
                    .ok_or(anyhow!("Missing registration status"))?
                    .trim()
                    .parse()
                    .map_err(|_| anyhow!("Invalid registration status"))?;

                let technology: u8 = parts
                    .next()
                    .ok_or(anyhow!("Missing technology status"))?
                    .trim()
                    .parse()
                    .map_err(|_| anyhow!("Invalid technology status"))?;

                Ok(ModemResponse::NetworkStatus { registration, technology })
            },
            ModemRequest::GetSignalStrength => {
                // Find the CSQ response line in the buffer
                let csq_line = response
                    .lines()
                    .find(|line| line.trim().starts_with("+CSQ:"))
                    .ok_or(anyhow!("No CSQ response found in buffer"))?;

                let data = csq_line
                    .trim()
                    .strip_prefix("+CSQ:")
                    .ok_or(anyhow!("Malformed CSQ response"))?
                    .trim();

                let mut parts = data.split(',');
                let rssi: i32 = parts
                    .next()
                    .ok_or(anyhow!("Missing RSSI value"))?
                    .trim()
                    .parse()
                    .map_err(|_| anyhow!("Invalid RSSI value"))?;

                let ber: i32 = parts
                    .next()
                    .ok_or(anyhow!("Missing BER value"))?
                    .trim()
                    .parse()
                    .map_err(|_| anyhow!("Invalid BER value"))?;

                Ok(ModemResponse::SignalStrength { rssi, ber })
            },
            ModemRequest::GetNetworkOperator => {
                // Find the COPS response line in the buffer
                let cops_line = response
                    .lines()
                    .find(|line| line.trim().starts_with("+COPS:"))
                    .ok_or(anyhow!("No COPS response found in buffer"))?;

                let data = cops_line
                    .trim()
                    .strip_prefix("+COPS:")
                    .ok_or(anyhow!("Malformed COPS response"))?
                    .trim();

                let mut parts = data.split(',');
                let status: u8 = parts
                    .next()
                    .ok_or(anyhow!("Missing operator status"))?
                    .trim()
                    .parse()
                    .map_err(|_| anyhow!("Invalid operator status"))?;

                let format: u8 = parts
                    .next()
                    .ok_or(anyhow!("Missing operator format"))?
                    .trim()
                    .parse()
                    .map_err(|_| anyhow!("Invalid operator format"))?;

                let operator = parts
                    .next()
                    .ok_or(anyhow!("Missing operator name"))?
                    .trim()
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .ok_or(anyhow!("Operator name not properly quoted"))?
                    .to_string();

                Ok(ModemResponse::NetworkOperator { status, format, operator })
            },
            ModemRequest::GetServiceProvider => {
                // Find the CSPN response line in the buffer
                let cspn_line = response
                    .lines()
                    .find(|line| line.trim().starts_with("+CSPN:"))
                    .ok_or(anyhow!("No CSPN response found in buffer"))?;

                let data = cspn_line
                    .trim()
                    .strip_prefix("+CSPN:")
                    .ok_or(anyhow!("Malformed CSPN response"))?
                    .trim();

                // Find the quoted operator name
                let quote_start = data.find('"').ok_or(anyhow!("Missing opening quote for operator name"))?;
                let quote_end = data.rfind('"').ok_or(anyhow!("Missing closing quote for operator name"))?;

                if quote_start >= quote_end {
                    return Err(anyhow!("Invalid quoted operator name"));
                }

                let operator = data[quote_start + 1..quote_end].to_string();

                Ok(ModemResponse::ServiceProvider { operator })
            },
            ModemRequest::GetBatteryLevel => {
                // Find the CBC response line in the buffer
                let cbc_line = response
                    .lines()
                    .find(|line| line.trim().starts_with("+CBC:"))
                    .ok_or(anyhow!("No CBC response found in buffer"))?;

                let data = cbc_line
                    .trim()
                    .strip_prefix("+CBC:")
                    .ok_or(anyhow!("Malformed CBC response"))?
                    .trim();

                let mut parts = data.split(',');
                let status: u8 = parts
                    .next()
                    .ok_or(anyhow!("Missing battery status"))?
                    .trim()
                    .parse()
                    .map_err(|_| anyhow!("Invalid battery status"))?;

                let charge: u8 = parts
                    .next()
                    .ok_or(anyhow!("Missing battery charge"))?
                    .trim()
                    .parse()
                    .map_err(|_| anyhow!("Invalid battery charge"))?;

                let voltage_raw: u32 = parts
                    .next()
                    .ok_or(anyhow!("Missing battery voltage"))?
                    .trim()
                    .parse()
                    .map_err(|_| anyhow!("Invalid battery voltage"))?;

                // Convert milli-volts to volts.
                let voltage: f32 = voltage_raw as f32 / 1000.0;
                Ok(ModemResponse::BatteryLevel { status, charge, voltage })
            }
        }
    }
}