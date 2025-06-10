use std::sync::Arc;
use log::{debug, error, info, warn};
use tokio::sync::{mpsc, Mutex};
use tokio_serial::SerialStream;
use anyhow::Result;
use crate::modem::buffer::LineEvent;
use crate::modem::commands::CommandTracker;
use crate::modem::handlers::{command_responder, handle_unsolicited_message, prompt_handler};
use crate::modem::ModemManager;
use crate::modem::types::{
    ModemEvent,
    ModemReadState,
    ModemResponse,
    ModemIncomingMessage,
    UnsolicitedMessageType
};

impl ModemManager {

    pub async fn process_modem_event(
        read_state: ModemReadState,
        line_event: LineEvent,
        main_tx: &mpsc::UnboundedSender<ModemIncomingMessage>,
        port: &Arc<Mutex<SerialStream>>,
        command_tracker: &mut CommandTracker
    ) -> Result<ModemReadState> {
        let modem_event = match line_event {
            LineEvent::Line(content) => Self::classify_line(&content, &read_state),
            LineEvent::Prompt(content) => ModemEvent::Prompt(content),
        };
        debug!("process_modem_event: ModemReadState {:?} | ModemEvent {:?}", read_state, modem_event);

        if let Some(expired_cmd) = command_tracker.force_timeout_active_command() {
            error!("Command sequence {} timed out: {:?}", expired_cmd.sequence, expired_cmd.request);
            expired_cmd.respond(ModemResponse::Error {
                message: format!("Command sequence {} timed out", expired_cmd.sequence)
            }).await;
        }

        match (read_state, modem_event) {

            // Receive data for an unsolicited message, completing the state and returning
            (ModemReadState::UnsolicitedMessage { message_type, header, active_command }, ModemEvent::Data(content)) => {

                // Handle the unsolicited message data, sending the parsed ModemReceivedMessage back to main_tx.
                match handle_unsolicited_message(&message_type, &header, &content).await {
                    Ok(message) => if let Some(message) = message {
                        let _ = main_tx.send(message);
                    },
                    Err(e) => error!("Couldn't handle incoming SMS message with error: {:?}", e)
                }

                // Restore previous command context if present.
                Ok(match active_command {
                    Some(ctx) => ModemReadState::Command(ctx),
                    None => ModemReadState::Idle,
                })
            },

            // Handle unsolicited messages when in command state - DON'T change command state.
            // TODO: Possibly queue this to be read again when available?
            (ModemReadState::Command(ctx), ModemEvent::Data(content))
            if Self::looks_like_unsolicited_content(&content) => {
                error!("Received unsolicited content during command execution: {:?}", content);
                Ok(ModemReadState::Command(ctx))
            },

            // Handle the start of an unsolicited modem event, during command or idle states.
            (read_state @ (ModemReadState::Command(_) | ModemReadState::Idle), ModemEvent::UnsolicitedNotification(content)) => {
                match UnsolicitedMessageType::from_header(&content) {
                    Some(message_type) => {
                        let (active_command, context_info) = match read_state {
                            ModemReadState::Command(ctx) => {
                                let sequence = ctx.cmd.sequence;
                                (Some(ctx), format!("during command {}", sequence))
                            },
                            ModemReadState::Idle => (None, "while idle".to_string()),
                            _ => unreachable!()
                        };

                        info!("Unsolicited message header received {}: {:?}", context_info, content);
                        Ok(ModemReadState::UnsolicitedMessage {
                            message_type,
                            header: content,
                            active_command
                        })
                    },
                    None => {
                        let context_info = match &read_state {
                            ModemReadState::Command(ctx) => {
                                let sequence = ctx.cmd.sequence;
                                format!("during command {}", sequence)
                            },
                            ModemReadState::Idle => "while idle".to_string(),
                            _ => unreachable!()
                        };

                        info!("Unsolicited notification {}: {:?}", context_info, content);
                        if let Err(e) = Self::handle_unsolicited_response(&content).await {
                            error!("Error handling unsolicited response: {}", e);
                        }
                        Ok(read_state) // Return the original state
                    }
                }
            },

            // Handle prompts only when expecting them.
            (ModemReadState::Command(mut ctx), ModemEvent::Prompt(content)) => {
                debug!("Processing prompt: {:?}", content);
                ctx.response_buffer.push_str(&content);

                match prompt_handler(port, &ctx.cmd.request).await {
                    Ok(Some(new_state)) => {
                        ctx.state = new_state;
                        Ok(ModemReadState::Command(ctx))
                    }
                    Ok(None) => Ok(ModemReadState::Idle),
                    Err(e) => {
                        error!("Prompt handler error: {e}");
                        ctx.cmd.respond(ModemResponse::Error {
                            message: format!("Prompt handler error: {e}")
                        }).await;
                        Ok(ModemReadState::Idle)
                    }
                }
            },

            // Handle command responses and other data when in command state.
            (ModemReadState::Command(mut ctx), ModemEvent::CommandResponse(content) | ModemEvent::Data(content)) => {
                debug!("Processing command response/data: {:?}", content);
                ctx.response_buffer.push_str(&content);
                ctx.response_buffer.push('\n');

                if ctx.state.is_complete(&content) {
                    let response = command_responder(&ctx.cmd.request, &ctx.response_buffer).await
                        .unwrap_or_else(|e| ModemResponse::Error { message: e.to_string() });

                    ctx.cmd.respond(response).await;
                    if let Some(_) = command_tracker.complete_command() {
                        debug!("Command completed and removed from tracker");
                    }

                    Ok(ModemReadState::Idle)
                } else {
                    Ok(ModemReadState::Command(ctx))
                }
            },

            // Ignore unexpected events when idle.
            (ModemReadState::Idle, ModemEvent::Prompt(content)) => {
                warn!("Received unexpected prompt when idle: {:?}", content);
                Ok(ModemReadState::Idle)
            }
            (ModemReadState::Idle, ModemEvent::CommandResponse(content) | ModemEvent::Data(content)) => {
                warn!("Received unexpected response when idle: {:?}", content);
                Ok(ModemReadState::Idle)
            },
            (read_state, modem_event) => {
                unreachable!("Got to an invalid state! Read: {:?}, Event: {:?}", read_state, modem_event);
            }
        }
    }

    fn looks_like_unsolicited_content(content: &str) -> bool {
        !content.starts_with("+") &&
            !content.starts_with("OK") &&
            !content.starts_with("ERROR") &&
            content.len() > 10
    }

    fn classify_line(content: &str, current_state: &ModemReadState) -> ModemEvent {
        let trimmed = content.trim();

        // Always classify these as unsolicited regardless of state.
        if trimmed.starts_with("+CMT:") ||
            trimmed.starts_with("+CDS:") ||
            trimmed.starts_with("+CMTI:") ||
            trimmed.starts_with("+RING") ||
            trimmed.starts_with("+CLIP:") ||
            trimmed.starts_with("+CCWA:") ||
            trimmed.starts_with("+CUSD:") ||
            trimmed.starts_with("+CGEV:") ||
            trimmed.starts_with("+CPIN:") ||
            trimmed.starts_with("^") ||
            trimmed.starts_with("*") {
            return ModemEvent::UnsolicitedNotification(trimmed.to_string());
        }

        // Command completion indicators - only relevant when executing commands.
        if matches!(current_state, ModemReadState::Command { .. }) {
            if trimmed == "OK" ||
                trimmed == "ERROR" ||
                trimmed.starts_with("+CME ERROR:") ||
                trimmed.starts_with("+CMS ERROR:") ||
                trimmed.starts_with("+CMGS:") ||  // SMS send confirmation.
                trimmed.starts_with("+CSQ:") ||   // Signal quality response.
                trimmed.starts_with("+CREG:") {   // Network registration response.
                return ModemEvent::CommandResponse(trimmed.to_string());
            }
        }

        // Handle solicited responses that might look like unsolicited ones.
        if matches!(current_state, ModemReadState::Command { .. }) &&
            (trimmed.starts_with("+CSQ:") || trimmed.starts_with("+CREG:")) {
            return ModemEvent::CommandResponse(trimmed.to_string());
        }

        ModemEvent::Data(trimmed.to_string())
    }

    async fn handle_unsolicited_response(
        content: &str
    ) -> Result<()> {
        debug!("Handling unsolicited response: {:?}", content);

        if content.starts_with("+CMT:") {
            info!("Incoming SMS header: {:?}", content);
        } else if content.starts_with("+RING") {
            info!("Incoming call detected");
        }

        Ok(())
    }
}