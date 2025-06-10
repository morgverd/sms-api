use std::mem::take;
use std::sync::Arc;
use log::{debug, error, info, warn};
use tokio::sync::{mpsc, Mutex};
use tokio_serial::SerialStream;
use anyhow::{bail, Result};
use crate::modem::buffer::LineEvent;
use crate::modem::commands::{CommandContext, CommandState, CommandTracker, OutgoingCommand};
use crate::modem::handlers::ModemEventHandlers;
use crate::modem::types::{
    ModemEvent,
    ModemReadState,
    ModemResponse,
    ModemIncomingMessage,
    UnsolicitedMessageType
};

#[derive(Default)]
pub struct ModemStateMachine {
    state: ModemReadState
}
impl ModemStateMachine {
    pub fn can_accept_command(&self) -> bool {
        matches!(self.state, ModemReadState::Idle)
    }

    pub fn has_active_command(&self) -> bool {
        matches!(self.state, ModemReadState::Command(_))
    }

    pub fn reset_to_idle(&mut self) {
        self.state = ModemReadState::Idle;
    }

    pub fn start_command(&mut self, sequence: u32, state: CommandState) {
        let ctx = CommandContext {
            sequence,
            state,
            response_buffer: String::new()
        };
        self.state = ModemReadState::Command(ctx);
    }

    pub async fn transition_state(
        &mut self,
        line_event: LineEvent,
        main_tx: &mpsc::UnboundedSender<ModemIncomingMessage>,
        port: &Arc<Mutex<SerialStream>>,
        command_tracker: &mut CommandTracker
    ) -> Result<()> {

        let modem_event = match line_event {
            LineEvent::Line(content) => self.classify_line(&content),
            LineEvent::Prompt(content) => ModemEvent::Prompt(content),
        };

        let new_state = self.process_event(modem_event, main_tx, port, command_tracker).await?;
        self.state = new_state;

        Ok(())
    }

    fn with_validated_command<'a, F, R>(
        &'a mut self,
        sequence: u32,
        command_tracker: &'a CommandTracker,
        handler: F,
    ) -> Result<R>
    where
        F: FnOnce(&'a OutgoingCommand) -> R,
    {
        if let Some(cmd) = command_tracker.get_active_command() {
            if cmd.sequence == sequence {
                Ok(handler(cmd))
            } else {
                bail!("Sequence mismatch: context has {} but tracker has {}!", sequence, cmd.sequence)
            }
        } else {
            bail!("No active command in tracker for sequence {}", sequence)
        }
    }

    async fn process_event(
        &mut self,
        modem_event: ModemEvent,
        main_tx: &mpsc::UnboundedSender<ModemIncomingMessage>,
        port: &Arc<Mutex<SerialStream>>,
        command_tracker: &mut CommandTracker
    ) -> Result<ModemReadState> {

        if let Some(expired_cmd) = command_tracker.force_timeout_active_command() {
            error!("Command sequence {} timed out: {:?}", expired_cmd.sequence, expired_cmd.request);
            expired_cmd.respond(ModemResponse::Error {
                message: format!("Command sequence {} timed out", expired_cmd.sequence)
            }).await;
        }

        match (take(&mut self.state), modem_event) {

            // Receive data for an unsolicited message, completing the state and returning
            (ModemReadState::UnsolicitedMessage { message_type, header, active_command }, ModemEvent::Data(content)) => {

                // Handle the unsolicited message data, sending the parsed ModemReceivedMessage back to main_tx.
                match ModemEventHandlers::handle_unsolicited_message(&message_type, &header, &content).await {
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
            (read_state @ (ModemReadState::Command(_) | ModemReadState::Idle), ModemEvent::UnsolicitedMessage { message_type, header }) => {
                let (active_command, context_info) = match read_state {
                    ModemReadState::Command(ctx) => {
                        let sequence = ctx.sequence;
                        (Some(ctx), format!("during command {}", sequence))
                    },
                    ModemReadState::Idle => (None, "while idle".to_string()),
                    _ => unreachable!()
                };

                info!("Unsolicited message header received {}: {:?}", context_info, header);
                Ok(ModemReadState::UnsolicitedMessage {
                    message_type,
                    header,
                    active_command
                })
            },

            // Handle prompts only when expecting them.
            (ModemReadState::Command(mut ctx), ModemEvent::Prompt(content)) => {
                debug!("Processing prompt: {:?}", content);
                ctx.response_buffer.push_str(&content);

                let request_ref = self.with_validated_command(
                    ctx.sequence,
                    command_tracker,
                    |cmd| &cmd.request
                )?;

                match ModemEventHandlers::prompt_handler(&port, request_ref).await {
                    Ok(Some(new_state)) => {
                        ctx.state = new_state;
                        Ok(ModemReadState::Command(ctx))
                    }
                    Ok(None) => Ok(ModemReadState::Idle),
                    Err(e) => {

                        // If prompt handling fails, send an error back to the command tracker to close it.
                        error!("Prompt handler error: {e}");
                        command_tracker.complete_active_command(ModemResponse::Error {
                            message: format!("Prompt handler error: {e}")
                        }).await?;
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

                    let (request_ref, response_buffer_ref) = self.with_validated_command(
                        ctx.sequence,
                        command_tracker,
                        |cmd| (&cmd.request, &ctx.response_buffer)
                    )?;

                    match ModemEventHandlers::command_responder(request_ref, response_buffer_ref).await {
                        Ok(response) => {
                            command_tracker.complete_active_command(response).await?;
                            Ok(ModemReadState::Idle)
                        },
                        Err(e) => {
                            let error_response = ModemResponse::Error {
                                message: e.to_string()
                            };

                            command_tracker.complete_active_command(error_response).await?;
                            Ok(ModemReadState::Idle)
                        }
                    }
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

    fn classify_line(&self, content: &str) -> ModemEvent {
        let trimmed = content.trim();

        // Prioritise unsolicited messages regardless of current state.
        if let Some(message_type) = UnsolicitedMessageType::from_header(trimmed) {
            return ModemEvent::UnsolicitedMessage { message_type, header: trimmed.to_string() }
        }

        // Command completion indicators - only relevant when executing commands.
        if matches!(self.state, ModemReadState::Command { .. }) {
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
        if matches!(self.state, ModemReadState::Command { .. }) &&
            (trimmed.starts_with("+CSQ:") || trimmed.starts_with("+CREG:")) {
            return ModemEvent::CommandResponse(trimmed.to_string());
        }

        ModemEvent::Data(trimmed.to_string())
    }

    fn looks_like_unsolicited_content(content: &str) -> bool {
        !content.starts_with("+") &&
            !content.starts_with("OK") &&
            !content.starts_with("ERROR") &&
            content.len() > 10
    }
}