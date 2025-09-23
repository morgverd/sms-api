use std::time::Duration;
use anyhow::anyhow;
use tokio::time::Instant;
use tracing::log::debug;
use crate::sms::types::SMSIncomingMessage;
use crate::types::SMSMessage;

const MULTIPART_MESSAGES_STALLED_DURATION: Duration = Duration::from_secs(30 * 60); // 30 minutes

#[derive(Debug, Clone)]
pub struct SMSMultipartHeader {
    pub message_reference: u8,
    pub total: u8,
    pub index: u8
}

#[derive(Debug, Clone)]
pub struct SMSMultipartMessages {
    pub total_size: usize,
    pub last_updated: Instant,
    pub first_message: Option<SMSIncomingMessage>,
    pub text_len: usize,
    pub text_parts: Vec<Option<String>>,
    pub received_count: usize,
}
impl SMSMultipartMessages {
    pub fn with_capacity(total_size: usize) -> Self {
        Self {
            total_size,
            last_updated: Instant::now(),
            first_message: None,
            text_len: 0,
            text_parts: vec![None; total_size],
            received_count: 0
        }
    }

    pub fn add_message(&mut self, message: SMSIncomingMessage, index: u8) -> bool {
        self.last_updated = Instant::now();
        if self.first_message.is_none() {
            self.first_message = Some(message.clone());
        }

        // Make multipart index 0-based.
        let idx = (index as usize).saturating_sub(1);
        if idx < self.text_parts.len() && self.text_parts[idx].is_none() {

            // Dirty fix until I have the time to rewrite the PDU parser.
            let content = if message.content.ends_with("@") {
                message.content.trim_end_matches("@").to_string()
            } else {
                message.content
            };

            self.text_len += content.len();
            self.text_parts[idx] = Some(content);
            self.received_count += 1;
        }

        debug!("Received Multipart SMS Count: {:?} | Max: {:?}", self.received_count, self.total_size);
        self.received_count >= self.total_size
    }

    pub fn compile(&self) -> anyhow::Result<SMSMessage> {
        let first_message = match &self.first_message {
            Some(first_message) => first_message,
            None => return Err(anyhow!("Missing required first message to convert into SMSMessage!"))
        };

        let mut content = String::with_capacity(self.text_len);
        for msg_opt in &self.text_parts {
            if let Some(text) = msg_opt {
                content.push_str(&text);
            }
        }

        let mut message = SMSMessage::from(first_message.clone());
        message.message_content = content;

        Ok(message)
    }

    pub fn is_stalled(&self) -> bool {
        self.last_updated.elapsed() > MULTIPART_MESSAGES_STALLED_DURATION
    }
}
