use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ModemConfig {
    pub device: &'static str,
    pub baud: u32,

    /// The read_interval is basically the key indicator of HTTP response speed.
    /// On average the modem responds within 20-30ms to a basic query.
    /// Lower value = more reads = higher CPU usage.
    pub read_interval_duration: Duration,

    /// The size of Command bounded mpsc sender, should be low. eg: 32
    pub cmd_channel_buffer_size: usize
}
