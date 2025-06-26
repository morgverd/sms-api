#[derive(Debug)]
pub enum LineEvent {
    Line(String),
    Prompt(String),
}

pub struct LineBuffer {
    buffer: Vec<u8>,
    max_buffer_size: usize,
}
impl LineBuffer {
    pub fn with_max_size(size: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(512),
            max_buffer_size: size,
        }
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    pub fn process_data(&mut self, data: &[u8]) -> Vec<LineEvent> {
        self.buffer.extend_from_slice(data);

        // Prevent unbounded growth.
        if self.buffer.len() > self.max_buffer_size {

            // Trim the oldest data, keeping only the most recent max_buffer_size bytes.
            let keep_from = self.buffer.len().saturating_sub(self.max_buffer_size);
            self.buffer.drain(..keep_from);
        }

        let mut events = Vec::new();
        let mut start = 0;
        let mut i = 0;

        while i < self.buffer.len() {
            match self.buffer[i] {
                b'\r' | b'\n' => {
                    if i > start {
                        if let Some(line_event) = self.try_create_event(&self.buffer[start..i], LineEvent::Line) {
                            events.push(line_event);
                        }
                    }

                    // Skip all consecutive newlines.
                    while i < self.buffer.len() && (self.buffer[i] == b'\r' || self.buffer[i] == b'\n') {
                        i += 1;
                    }
                    start = i;
                }
                b'>' => {
                    // Only treat as prompt if it's at start of line or after whitespace.
                    let is_prompt = start == i
                        || (i > 0 && (self.buffer[i - 1] == b'\n' || self.buffer[i - 1] == b'\r'));

                    if is_prompt {
                        if let Some(prompt_event) = self.try_create_event(&self.buffer[start..=i], LineEvent::Prompt) {
                            events.push(prompt_event);
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

    fn try_create_event<F>(&self, data: &[u8], constructor: F) -> Option<LineEvent>
    where
        F: FnOnce(String) -> LineEvent,
    {
        // Ignore if empty or whitespace only.
        if data.is_empty() || data.iter().all(|&b| b.is_ascii_whitespace()) {
            return None;
        }

        let content = match std::str::from_utf8(data) {
            Ok(content) => content.trim(),
            Err(_) => {
                // Handle invalid UTF-8 gracefully - convert with replacement chars
                return match String::from_utf8_lossy(data).trim() {
                    trimmed if !trimmed.is_empty() => Some(constructor(trimmed.to_string())),
                    _ => None,
                };
            }
        };

        if !content.is_empty() {
            Some(constructor(content.to_string()))
        } else {
            None
        }
    }
}