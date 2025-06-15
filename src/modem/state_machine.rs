use std::mem::take;
use std::sync::Arc;
use std::time::Instant;
use log::{debug, error, warn};
use tokio::sync::{mpsc, Mutex};
use tokio_serial::SerialStream;
use anyhow::{bail, Result};
use crate::modem::buffer::LineEvent;
use crate::modem::commands::{CommandContext, CommandState, OutgoingCommand};
use crate::modem::handlers::ModemEventHandlers;
use crate::modem::types::{
    ModemEvent,
    ModemResponse,
    ModemIncomingMessage,
    UnsolicitedMessageType
};

#[derive(Debug)]
struct CommandExecution {
    context: CommandContext,
    command: OutgoingCommand,
    timeout_at: Instant
}
impl CommandExecution {
    fn new(command: OutgoingCommand, command_state: CommandState) -> Self {
        let timeout = command.request.get_timeout();
        let context = CommandContext {
            sequence: command.sequence,
            state: command_state,
            response_buffer: String::new()
        };

        Self {
            context,
            command,
            timeout_at: Instant::now() + timeout,
        }
    }

    fn is_timed_out(&self) -> bool {
        Instant::now() >= self.timeout_at
    }
}

#[derive(Debug, Default)]
enum StateMachineState {
    #[default] Idle,
    Command(CommandExecution),
    UnsolicitedMessage {
        message_type: UnsolicitedMessageType,
        header: String,
        interrupted_command: Option<CommandExecution>,
    }
}

#[derive(Default)]
pub struct ModemStateMachine {
    state: StateMachineState
}
impl ModemStateMachine {
    pub fn can_accept_command(&self) -> bool {
        matches!(self.state, StateMachineState::Idle)
    }

    pub fn reset_to_idle(&mut self) {
        self.state = StateMachineState::Idle;
    }

    pub fn start_command(&mut self, cmd: OutgoingCommand, command_state: CommandState) {
        debug!("Starting command: {:?} with state -> {:?}", cmd, command_state);
        let execution = CommandExecution::new(cmd, command_state);
        self.state = StateMachineState::Command(execution);
    }

    pub async fn handle_command_timeout(&mut self) -> Result<()> {
        let execution = match &self.state {
            StateMachineState::Command(execution) => execution,
            _ => return Ok(())
        };
        if !execution.is_timed_out() {
            return Ok(());
        }
        
        // Remove the CommandExecution from state to get OutgoingCommand.
        let mut command = match take(&mut self.state) {
            StateMachineState::Command(execution) => {
                self.state = StateMachineState::Idle;
                execution.command
            }
            _ => unreachable!(),
        };
        
        warn!("Command {} timed out!", command.sequence);
        command.respond(ModemResponse::Error {
            message: "Command timed out!".to_string()
        }).await
    }

    pub async fn transition_state(
        &mut self,
        line_event: LineEvent,
        main_tx: &mpsc::UnboundedSender<ModemIncomingMessage>,
        port: &Arc<Mutex<SerialStream>>
    ) -> Result<()> {
        
        // FIXME: REMOVE THESE LOGS!
        debug!("ModemStateMachine transition_state: LineEvent: {:?}", line_event);
        let modem_event = match line_event {
            LineEvent::Line(content) => self.classify_line(&content),
            LineEvent::Prompt(content) => ModemEvent::Prompt(content),
        };
        debug!("ModemStateMachine transition_state: ModemEvent: {:?}, State: {:?}", modem_event, self.state);

        let new_state = self.process_event(port, modem_event, main_tx).await?;
        debug!("ModemStateMachine transition_state: {:?} -> {:?}", self.state, new_state);
        self.state = new_state;

        Ok(())
    }

    async fn process_event(
        &mut self,
        port: &Arc<Mutex<SerialStream>>,
        modem_event: ModemEvent,
        main_tx: &mpsc::UnboundedSender<ModemIncomingMessage>
    ) -> Result<StateMachineState> {

        match (take(&mut self.state), modem_event) {

            // Receive data for an unsolicited message, completing the state and returning
            (StateMachineState::UnsolicitedMessage { message_type, header, interrupted_command }, ModemEvent::Data(content)) => {

                // Handle the unsolicited message data, sending the parsed ModemReceivedMessage back to main_tx.
                match ModemEventHandlers::handle_unsolicited_message(&message_type, &header, &content).await {
                    Ok(message) => if let Some(message) = message {
                        let _ = main_tx.send(message);
                    },
                    Err(e) => error!("Couldn't handle incoming SMS message with error: {:?}", e)
                }

                // Restore previous command context if present.
                Ok(match interrupted_command {
                    Some(execution) => StateMachineState::Command(execution),
                    None => StateMachineState::Idle,
                })
            },

            // Handle the start of an unsolicited modem event, during command or idle states.
            (StateMachineState::Command(execution), ModemEvent::UnsolicitedMessage { message_type, header }) => {
                let sequence = execution.context.sequence;
                debug!("Unsolicited message header received during command {}: {:?}", sequence, header);
                Ok(StateMachineState::UnsolicitedMessage {
                    message_type,
                    header,
                    interrupted_command: Some(execution),
                })
            },
            (StateMachineState::Idle, ModemEvent::UnsolicitedMessage { message_type, header }) => {
                debug!( "Unsolicited message header received while idle: {:?}", header);
                Ok(StateMachineState::UnsolicitedMessage {
                    message_type,
                    header,
                    interrupted_command: None,
                })
            },

            // Process command responses.
            (StateMachineState::Command(execution), event) => {
                self.process_command(port, execution, event).await
            }

            // Ignore unexpected events when idle.
            (StateMachineState::Idle, ModemEvent::Prompt(content)) => {
                warn!("Received unexpected prompt when idle: {:?}", content);
                Ok(StateMachineState::Idle)
            }
            (StateMachineState::Idle, ModemEvent::CommandResponse(content) | ModemEvent::Data(content)) => {
                warn!("Received unexpected response when idle: {:?}", content);
                Ok(StateMachineState::Idle)
            },
            (read_state, modem_event) => {
                error!("Got to an invalid state! Read: {:?}, Event: {:?}", read_state, modem_event);
                bail!("Invalid state transition: {:?} with event {:?}", read_state, modem_event);
            }
        }
    }

    async fn process_command(
        &mut self,
        port: &Arc<Mutex<SerialStream>>,
        mut execution: CommandExecution,
        event: ModemEvent
    ) -> Result<StateMachineState> {
        match event {

            // Handle prompts only when expecting them.
            ModemEvent::Prompt(content) => {
                debug!("Processing prompt: {:?}", content);
                execution.context.response_buffer.push_str(&content);

                match ModemEventHandlers::prompt_handler(&port, &execution.command.request).await {
                    Ok(Some(new_state)) => {
                        execution.context.state = new_state;
                        Ok(StateMachineState::Command(execution))
                    }
                    Ok(None) => {
                        // Prompt handler indicates command is complete
                        let response = ModemResponse::Error {
                            message: "Command completed during prompt handling".to_string()
                        };
                        execution.command.respond(response).await?;
                        Ok(StateMachineState::Idle)
                    },
                    Err(e) => {
                        // If prompt handling fails, send an error back to complete the command
                        error!("Prompt handler error: {e}");
                        let response = ModemResponse::Error {
                            message: format!("Prompt handler error: {e}")
                        };
                        execution.command.respond(response).await?;
                        Ok(StateMachineState::Idle)
                    }
                }
            },

            // Handle command responses and other data when in command state.
            ModemEvent::CommandResponse(content) | ModemEvent::Data(content) => {
                debug!("Processing command response/data: {:?}", content);
                execution.context.response_buffer.push_str(&content);
                execution.context.response_buffer.push('\n');

                if execution.context.state.is_complete(&content) {
                    match ModemEventHandlers::command_responder(&execution.command.request, &execution.context.response_buffer).await {
                        Ok(response) => {
                            execution.command.respond(response).await?;
                            Ok(StateMachineState::Idle)
                        },
                        Err(e) => {
                            let error_response = ModemResponse::Error {
                                message: e.to_string()
                            };
                            execution.command.respond(error_response).await?;
                            Ok(StateMachineState::Idle)
                        }
                    }
                } else {
                    Ok(StateMachineState::Command(execution))
                }
            },
            ModemEvent::UnsolicitedMessage { .. } => {
                unreachable!("Unsolicited messages during a command should have already been handled!")
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
        if matches!(self.state, StateMachineState::Command(_)) {
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
        if matches!(self.state, StateMachineState::Command(_)) &&
            (trimmed.starts_with("+CSQ:") || trimmed.starts_with("+CREG:")) {
            return ModemEvent::CommandResponse(trimmed.to_string());
        }

        ModemEvent::Data(trimmed.to_string())
    }
}