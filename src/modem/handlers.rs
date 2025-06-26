use anyhow::{anyhow, Result};
use log::{debug, warn};
use pdu_rs::pdu::{DeliverPdu, StatusReportPdu};
use tokio::sync::mpsc;
use crate::sms::types::{SMSIncomingDeliveryReport, SMSIncomingMessage};
use crate::modem::commands::CommandState;
use crate::modem::worker::{WorkerEvent, ModemStatus};
use crate::modem::types::{
    ModemRequest,
    ModemResponse,
    ModemIncomingMessage,
    UnsolicitedMessageType
};

pub struct ModemEventHandlers {
    worker_event_tx: mpsc::UnboundedSender<WorkerEvent>,
}
impl ModemEventHandlers {
    pub fn new(worker_event_tx: mpsc::UnboundedSender<WorkerEvent>) -> Self {
        Self { worker_event_tx }
    }

    pub async fn command_sender(&self, request: &ModemRequest) -> Result<CommandState> {
        match request {
            ModemRequest::SendSMS { len, .. } => {
                let command = format!("AT+CMGS={}\r\n", len);
                self.write(command.as_bytes()).await?;
                Ok(CommandState::WaitingForPrompt)
            }
            ModemRequest::GetNetworkStatus => {
                self.write(b"AT+CREG?\r\n").await?;
                Ok(CommandState::WaitingForData)
            }
            ModemRequest::GetSignalStrength => {
                self.write(b"AT+CSQ\r\n").await?;
                Ok(CommandState::WaitingForData)
            },
            ModemRequest::GetNetworkOperator => {
                self.write(b"AT+COPS?\r\n").await?;
                Ok(CommandState::WaitingForData)
            },
            ModemRequest::GetServiceProvider => {
                self.write(b"AT+CSPN?\r\n").await?;
                Ok(CommandState::WaitingForData)
            },
            ModemRequest::GetBatteryLevel => {
                self.write(b"AT+CBC\r\n").await?;
                Ok(CommandState::WaitingForData)
            }
        }
    }

    pub async fn prompt_handler(&self, request: &ModemRequest) -> Result<Option<CommandState>> {
        if let ModemRequest::SendSMS { len, pdu } = request {
            debug!("Sending PDU: len = {}", len);

            // Push CTRL+Z to end of PDU to submit.
            let mut buf = Vec::with_capacity(pdu.as_bytes().len() + 1);
            buf.extend_from_slice(pdu.as_bytes());
            buf.push(0x1A);
            self.write(&buf).await?;

            return Ok(Some(CommandState::WaitingForOk));
        }

        Ok(None)
    }

    pub async fn handle_unsolicited_message(
        &self,
        message_type: &UnsolicitedMessageType,
        content: &str
    ) -> Result<Option<ModemIncomingMessage>> {
        debug!("UnsolicitedMessage: {:?} -> {:?}", &message_type, &content);

        match message_type {
            UnsolicitedMessageType::IncomingSMS => {
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
                warn!("The modem is shutting down!");
                self.set_status(ModemStatus::ShuttingDown).await?;
                Ok(None)
            }
        }
    }

    pub async fn command_responder(
        &self,
        request: &ModemRequest,
        response: &String
    ) -> Result<ModemResponse> {
        debug!("Command response: {:?} -> {:?}", request, response);
        if !response.trim_end().ends_with("OK") {
            return Err(anyhow!("Modem response does not end with OK"));
        }

        match request {
            ModemRequest::SendSMS { .. } => {
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
                let cspn_line = response
                    .lines()
                    .find(|line| line.trim().starts_with("+CSPN:"))
                    .ok_or(anyhow!("No CSPN response found in buffer"))?;

                let data = cspn_line
                    .trim()
                    .strip_prefix("+CSPN:")
                    .ok_or(anyhow!("Malformed CSPN response"))?
                    .trim();

                // Find the quoted operator name.
                let quote_start = data.find('"').ok_or(anyhow!("Missing opening quote for operator name"))?;
                let quote_end = data.rfind('"').ok_or(anyhow!("Missing closing quote for operator name"))?;

                if quote_start >= quote_end {
                    return Err(anyhow!("Invalid quoted operator name"));
                }

                let operator = data[quote_start + 1..quote_end].to_string();

                Ok(ModemResponse::ServiceProvider { operator })
            },
            ModemRequest::GetBatteryLevel => {
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

                let voltage: f32 = voltage_raw as f32 / 1000.0;
                Ok(ModemResponse::BatteryLevel { status, charge, voltage })
            }
        }
    }

    async fn write(&self, data: &[u8]) -> Result<()> {
        self.worker_event_tx
            .send(WorkerEvent::WriteCommand(data.to_vec()))
            .map_err(|_| anyhow!("Failed to send write command event"))?;
        Ok(())
    }

    async fn set_status(&self, status: ModemStatus) -> Result<()> {
        self.worker_event_tx
            .send(WorkerEvent::SetStatus(status))
            .map_err(|_| anyhow!("Failed to send status change event"))?;
        Ok(())
    }
}