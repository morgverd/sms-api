#[derive(Debug)]
pub enum LineEvent {
    Line(String),
    Prompt(String),
}

pub struct LineBuffer {
    buffer: String,
    max_buffer_size: usize,
}

impl LineBuffer {
    pub fn new() -> Self {
        Self::with_max_size(4096)
    }

    pub fn with_max_size(size: usize) -> Self {
        Self {
            buffer: String::new(),
            max_buffer_size: size,
        }
    }

    /// Clears the internal buffer. Should be called after timeouts.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// Consumes new data, splits it into lines and prompts, and returns events.
    pub fn process_data(&mut self, data: &str) -> Vec<LineEvent> {
        self.buffer.push_str(data);

        // Prevent unbounded growth.
        if self.buffer.len() > self.max_buffer_size {
            // Trim the oldest data, keeping only the most recent max_buffer_size bytes.
            // This will probably break the message anyway, but better than a total reset?
            let keep_from = self.buffer.len().saturating_sub(self.max_buffer_size);
            self.buffer = self.buffer[keep_from..].to_string();
        }

        let mut events = Vec::new();
        let mut start = 0;
        let bytes = self.buffer.as_bytes();
        let mut i = 0;

        while i < bytes.len() {
            match bytes[i] {
                b'\r' | b'\n' => {
                    if i > start {
                        let content = &self.buffer[start..i];
                        let trimmed = content.trim();
                        if !trimmed.is_empty() {
                            events.push(LineEvent::Line(trimmed.to_string()));
                        }
                    }
                    // Skip all consecutive newlines.
                    while i < bytes.len() && (bytes[i] == b'\r' || bytes[i] == b'\n') {
                        i += 1;
                    }
                    start = i;
                }
                b'>' => {

                    // Only treat as prompt if it's at start of line or after whitespace.
                    let is_prompt = start == i
                        || (i > 0 && (bytes[i - 1] == b'\n' || bytes[i - 1] == b'\r'));
                    if is_prompt {
                        let content = &self.buffer[start..=i];
                        let trimmed = content.trim();
                        if !trimmed.is_empty() {
                            events.push(LineEvent::Prompt(trimmed.to_string()));
                        }
                        i += 1;
                        start = i;
                        continue;
                    } else {
                        i += 1;
                    }
                }
                _ => i += 1,
            }
        }

        // Retain any partial line at the end.
        if start > 0 {
            self.buffer.drain(..start);
        }

        events
    }
}