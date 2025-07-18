use std::mem::take;
use std::time::Instant;
use log::{debug, error, warn};
use tokio::sync::mpsc;
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
use crate::modem::worker::WorkerEvent;

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
        interrupted_command: Option<CommandExecution>,
    }
}

pub struct ModemStateMachine {
    main_tx: mpsc::UnboundedSender<ModemIncomingMessage>,
    state: StateMachineState,
    handlers: ModemEventHandlers
}
impl ModemStateMachine {
    pub fn new(
        main_tx: mpsc::UnboundedSender<ModemIncomingMessage>,
        worker_event_tx: mpsc::UnboundedSender<WorkerEvent>
    ) -> Self {
        Self {
            main_tx,
            state: StateMachineState::Idle,
            handlers: ModemEventHandlers::new(worker_event_tx)
        }
    }

    pub fn can_accept_command(&self) -> bool {
        matches!(self.state, StateMachineState::Idle)
    }

    pub fn reset_to_idle(&mut self) {
        self.state = StateMachineState::Idle;
    }

    pub async fn start_command(&mut self, cmd: OutgoingCommand) -> Result<()> {
        debug!("Starting command: {:?}", cmd);

        let command_state = self.handlers.command_sender(&cmd.request).await?;
        let execution = CommandExecution::new(cmd, command_state);
        self.state = StateMachineState::Command(execution);

        Ok(())
    }

    pub async fn handle_command_timeout(&mut self) -> Result<bool> {
        let execution = match &self.state {
            StateMachineState::Command(execution) => execution,
            _ => return Ok(false)
        };

        if !execution.is_timed_out() {
            return Ok(false);
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
        }).await.map(|_| true)
    }

    pub async fn transition_state(&mut self, line_event: LineEvent) -> Result<()> {
        debug!("ModemStateMachine transition_state: LineEvent: {:?}", line_event);

        let modem_event = match line_event {
            LineEvent::Line(content) => self.classify_line(&content),
            LineEvent::Prompt(content) => ModemEvent::Prompt(content),
        };

        debug!("ModemStateMachine transition_state: ModemEvent: {:?}, State: {:?}", modem_event, self.state);

        let new_state = self.process_event(modem_event).await?;
        debug!("ModemStateMachine transition_state: {:?} -> {:?}", self.state, new_state);
        self.state = new_state;

        Ok(())
    }

    async fn process_event(
        &mut self,
        modem_event: ModemEvent
    ) -> Result<StateMachineState> {
        match (take(&mut self.state), modem_event) {
            // Handle unsolicited message completion
            (StateMachineState::UnsolicitedMessage { message_type, interrupted_command, .. }, ModemEvent::Data(content)) => {
                self.handle_unsolicited(&message_type, &content).await;
                Ok(match interrupted_command {
                    Some(execution) => StateMachineState::Command(execution),
                    None => StateMachineState::Idle,
                })
            },

            // Handle unsolicited message start
            (StateMachineState::Command(execution), ModemEvent::UnsolicitedMessage { message_type, header }) => {
                let sequence = execution.context.sequence;
                debug!("Unsolicited message header received during command {}: {:?}", sequence, header);

                if !message_type.has_next_line() {
                    self.handle_unsolicited(&message_type, &header).await;
                    Ok(StateMachineState::Command(execution))
                } else {
                    Ok(StateMachineState::UnsolicitedMessage {
                        message_type,
                        interrupted_command: Some(execution),
                    })
                }
            },
            (StateMachineState::Idle, ModemEvent::UnsolicitedMessage { message_type, header }) => {
                debug!("Unsolicited message header received while idle: {:?}", header);

                if !message_type.has_next_line() {
                    self.handle_unsolicited(&message_type, &header).await;
                    Ok(StateMachineState::Idle)
                } else {
                    Ok(StateMachineState::UnsolicitedMessage {
                        message_type,
                        interrupted_command: None,
                    })
                }
            },

            // Process command responses
            (StateMachineState::Command(execution), event) => {
                self.process_command(execution, event).await
            }

            // Ignore unexpected events when idle
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
        mut execution: CommandExecution,
        event: ModemEvent
    ) -> Result<StateMachineState> {
        match event {
            ModemEvent::Prompt(content) => {
                debug!("Processing prompt: {:?}", content);
                // Don't add prompt to response buffer - it's not part of the command response

                match self.handlers.prompt_handler(&execution.command.request).await {
                    Ok(Some(new_state)) => {
                        execution.context.state = new_state;
                        Ok(StateMachineState::Command(execution))
                    }
                    Ok(None) => {
                        let response = ModemResponse::Error {
                            message: "Command completed during prompt handling".to_string()
                        };
                        execution.command.respond(response).await?;
                        Ok(StateMachineState::Idle)
                    },
                    Err(e) => {
                        error!("Prompt handler error: {e}");
                        let response = ModemResponse::Error {
                            message: format!("Prompt handler error: {e}")
                        };
                        execution.command.respond(response).await?;
                        Ok(StateMachineState::Idle)
                    }
                }
            },

            ModemEvent::CommandResponse(content) | ModemEvent::Data(content) => {
                debug!("Processing command response/data: {:?}", content);
                execution.context.response_buffer.push_str(&content);
                execution.context.response_buffer.push('\n');

                if execution.context.state.is_complete(&content) {
                    match self.handlers.command_responder(&execution.command.request, &execution.context.response_buffer).await {
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

    async fn handle_unsolicited(&self, message_type: &UnsolicitedMessageType, content: &str) {
        match self.handlers.handle_unsolicited_message(message_type, content).await {
            Ok(message) => if let Some(message) = message {
                let _ = self.main_tx.send(message);
            },
            Err(e) => error!("Couldn't handle incoming SMS message with error: {:?}", e)
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
                trimmed.starts_with("+CMGS:") ||
                trimmed.starts_with("+CSQ:") ||
                trimmed.starts_with("+CREG:") {
                return ModemEvent::CommandResponse(trimmed.to_string());
            }
        }

        ModemEvent::Data(trimmed.to_string())
    }
}