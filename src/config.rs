use std::time::Duration;
use anyhow::Result;
use dotenv::dotenv;

macro_rules! env_config {
    // Required field
    ($field_name:ident: $field_type:ty, $env_var:literal) => {
        (|| -> anyhow::Result<$field_type> {
            std::env::var($env_var)
                .map_err(|_| anyhow::anyhow!("Required environment variable '{}' is missing", $env_var))
                .and_then(|v| v.parse::<$field_type>()
                    .map_err(|e| anyhow::anyhow!("Failed to parse '{}' as {}: {}", $env_var, stringify!($field_type), e)))
        })()
    };

    // Optional field with default
    ($field_name:ident: $field_type:ty, $env_var:literal, $default:expr) => {
        (|| -> anyhow::Result<$field_type> {
            match std::env::var($env_var) {
                Ok(v) => v.parse::<$field_type>()
                    .map_err(|e| anyhow::anyhow!("Failed to parse '{}' as {}: {}", $env_var, stringify!($field_type), e)),
                Err(_) => Ok($default)
            }
        })()
    };
}

pub struct AppConfig {
    pub modem: ModemConfig,
    pub sms: SMSConfig
}
impl AppConfig {
    pub fn load_from_env() -> Result<Self> {
        dotenv().ok();
        Ok(Self {
            modem: ModemConfig::from_env()?,
            sms: SMSConfig::from_env()?
        })
    }
}

#[derive(Debug, Clone)]
pub struct ModemConfig {
    pub device: String,
    pub baud: u32,

    /// The read_interval is basically the key indicator of HTTP response speed.
    /// On average the modem responds within 20-30ms to a basic query.
    /// Lower value = more reads = higher CPU usage.
    pub read_interval_duration: Duration,

    /// The size of Command bounded mpsc sender, should be low. eg: 32
    pub cmd_channel_buffer_size: usize
}
impl ModemConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            device: env_config!(device: String, "SMS_MODEM_DEVICE", "/dev/ttyS0".to_string())?,
            baud: env_config!(baud: u32, "SMS_MODEM_BAUD", 115200)?,
            read_interval_duration: {
                let millis: u64 = env_config!(read_interval_ms: u64, "SMS_MODEM_READ_INTERVAL_MS", 25)?;
                Duration::from_millis(millis)
            },
            cmd_channel_buffer_size: env_config!(cmd_channel_buffer_size: usize, "SMS_MODEM_CMD_CHANNEL_BUFFER_SIZE", 12)?
        })
    }
}

#[derive(Debug)]
pub struct SMSConfig {
    pub webhooks: Vec<ConfiguredWebhook>,
    pub database_url: String,
    pub encryption_key: [u8; 32]
}
impl SMSConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            webhooks: Vec::new(),
            database_url: env_config!(database_url: String, "SMS_DATABASE_URL")?,

            // TODO: Load from some local key file instead of this hardcoded test key.
            encryption_key: [
                147, 203, 89, 45, 12, 178, 234, 67, 91, 156, 23, 88, 201, 142, 76, 39,
                165, 118, 95, 212, 33, 184, 157, 72, 109, 246, 58, 131, 194, 85, 167, 29
            ]
        })
    }
}

#[derive(Debug)]
pub struct ConfiguredWebhook {
    pub url: String,
    pub header_name: String,
    pub header_value: String
}