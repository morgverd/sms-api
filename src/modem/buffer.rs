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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_line_processing() {
        let mut buffer = LineBuffer::with_max_size(1024);

        let events = buffer.process_data(b"hello world\n");
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], LineEvent::Line(s) if s == "hello world"));

        let events = buffer.process_data(b"first\nsecond\nthird\n");
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[0], LineEvent::Line(s) if s == "first"));
        assert!(matches!(&events[1], LineEvent::Line(s) if s == "second"));
        assert!(matches!(&events[2], LineEvent::Line(s) if s == "third"));
    }

    #[test]
    fn test_prompt_detection() {
        let mut buffer = LineBuffer::with_max_size(1024);

        let events = buffer.process_data(b">");
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], LineEvent::Prompt(s) if s == ">"));

        let events = buffer.process_data(b"output\n>");
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], LineEvent::Line(s) if s == "output"));
        assert!(matches!(&events[1], LineEvent::Prompt(s) if s == ">"));

        let events = buffer.process_data(b"test>data\n");
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], LineEvent::Line(s) if s == "test>data"));
    }

    #[test]
    fn test_mixed_events_sequence() {
        let mut buffer = LineBuffer::with_max_size(1024);

        let events = buffer.process_data(b"command output\n>user input\n>");
        assert_eq!(events.len(), 4);
        assert!(matches!(&events[0], LineEvent::Line(s) if s == "command output"));
        assert!(matches!(&events[1], LineEvent::Prompt(s) if s == ">"));
        assert!(matches!(&events[2], LineEvent::Line(s) if s == "user input"));
        assert!(matches!(&events[3], LineEvent::Prompt(s) if s == ">"));
    }

    #[test]
    fn test_incremental_processing() {
        let mut buffer = LineBuffer::with_max_size(1024);

        assert_eq!(buffer.process_data(b"partial").len(), 0);
        assert_eq!(buffer.process_data(b" data").len(), 0);

        let events = buffer.process_data(b" here\n");
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], LineEvent::Line(s) if s == "partial data here"));

        assert_eq!(buffer.process_data(b"line").len(), 0);
        assert_eq!(buffer.process_data(b" two").len(), 0);
        let events = buffer.process_data(b"\n>");
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], LineEvent::Line(s) if s == "line two"));
        assert!(matches!(&events[1], LineEvent::Prompt(s) if s == ">"));

        let events = buffer.process_data(b"command\n");
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], LineEvent::Line(s) if s == "command"));
    }

    #[test]
    fn test_line_endings() {
        let mut buffer = LineBuffer::with_max_size(1024);

        let events = buffer.process_data(b"unix\nwindows\r\nmac\rend\n");
        assert_eq!(events.len(), 4);
        assert!(matches!(&events[0], LineEvent::Line(s) if s == "unix"));
        assert!(matches!(&events[1], LineEvent::Line(s) if s == "windows"));
        assert!(matches!(&events[2], LineEvent::Line(s) if s == "mac"));
        assert!(matches!(&events[3], LineEvent::Line(s) if s == "end"));

        let events = buffer.process_data(b"output\r>");
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], LineEvent::Line(s) if s == "output"));
        assert!(matches!(&events[1], LineEvent::Prompt(s) if s == ">"));
    }

    #[test]
    fn test_whitespace_handling() {
        let mut buffer = LineBuffer::with_max_size(1024);

        let events = buffer.process_data(b"  hello world  \n");
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], LineEvent::Line(s) if s == "hello world"));

        let events = buffer.process_data(b"\n\n   \n\t\t\n");
        assert_eq!(events.len(), 0);

        let events = buffer.process_data(b"line1\n\n\nline2\n");
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], LineEvent::Line(s) if s == "line1"));
        assert!(matches!(&events[1], LineEvent::Line(s) if s == "line2"));
    }

    #[test]
    fn test_buffer_size_limits() {
        let mut buffer = LineBuffer::with_max_size(10);

        buffer.process_data(b"0123456789ABCDEFGHIJ");
        assert!(buffer.buffer.len() <= 10);

        buffer.clear();
        buffer.process_data(b"0123456789");
        buffer.process_data(b"ABCDE");
        assert!(buffer.buffer.len() <= 10);
        let buffer_str = String::from_utf8_lossy(&buffer.buffer);
        assert!(buffer_str.ends_with("ABCDE") || buffer.buffer.len() == 10);
    }

    #[test]
    fn test_invalid_utf8_recovery() {
        let mut buffer = LineBuffer::with_max_size(1024);

        let events = buffer.process_data(&[0xFF, 0xFE, 0xFD, b'\n']);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], LineEvent::Line(_)));
    }

    #[test]
    fn test_clear_buffer() {
        let mut buffer = LineBuffer::with_max_size(1024);

        buffer.process_data(b"some data");
        assert!(!buffer.buffer.is_empty());

        buffer.clear();
        assert!(buffer.buffer.is_empty());

        let events = buffer.process_data(b"new line\n");
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], LineEvent::Line(s) if s == "new line"));
    }
}