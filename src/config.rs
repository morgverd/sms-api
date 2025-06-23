use std::collections::HashMap;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use anyhow::{Context, Result};
use axum::http::HeaderValue;
use base64::Engine;
use base64::engine::general_purpose;
use reqwest::header::{HeaderMap, HeaderName};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub database: DatabaseConfig,
    pub sentry: Option<SentryConfig>,

    #[serde(default)]
    pub modem: ModemConfig,

    #[serde(default)]
    pub http: HTTPConfig,

    #[serde(default)]
    pub webhooks: Option<Vec<ConfiguredWebhook>>
}

impl AppConfig {
    pub fn load(config_filepath: Option<PathBuf>) -> Result<Self> {
        let config_path = config_filepath
            .unwrap_or_else(|| PathBuf::from("config.toml"));

        let config_content = fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config file: {:?}", config_path))?;

        let config: AppConfig = toml::from_str(&config_content)
            .with_context(|| format!("Failed to parse TOML config file: {:?}", config_path))?;

        Ok(config)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModemConfig {
    #[serde(default = "default_modem_device")]
    pub device: String,

    #[serde(default = "default_modem_baud")]
    pub baud: u32,

    /// The read_interval is basically the key indicator of HTTP response speed.
    /// On average the modem responds within 20-30ms to a basic query.
    /// Lower value = more reads = higher CPU usage.
    #[serde(default = "default_modem_read_interval")]
    #[serde(deserialize_with = "deserialize_duration_from_millis")]
    pub read_interval_duration: Duration,

    /// The size of Command bounded mpsc sender, should be low. eg: 32
    #[serde(default = "default_modem_cmd_buffer_size")]
    pub cmd_channel_buffer_size: usize
}
impl Default for ModemConfig {
    fn default() -> Self {
        Self {
            device: default_modem_device(),
            baud: default_modem_baud(),
            read_interval_duration: default_modem_read_interval(),
            cmd_channel_buffer_size: default_modem_cmd_buffer_size()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    pub database_url: String,

    #[serde(deserialize_with = "deserialize_encryption_key")]
    pub encryption_key: [u8; 32]
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConfiguredWebhook {
    pub url: String,
    pub expected_status: Option<u16>,

    /// By default, this is only IncomingMessage.
    #[serde(default = "default_webhook_events")]
    pub events: Vec<ConfiguredWebhookEvent>,

    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
}
impl ConfiguredWebhook {
    pub fn get_header_map(&self) -> Result<Option<HeaderMap>> {
        let map = if let Some(headers) = &self.headers { headers } else { return Ok(None); };

        let mut out = HeaderMap::with_capacity(map.len());
        for (k, v) in map {
            out.insert(
                HeaderName::from_str(k)?,
                HeaderValue::from_str(v)?
            );
        }

        Ok(Some(out))
    }
}

#[derive(Eq, PartialEq, Hash, Debug, Clone, Copy, Deserialize)]
pub enum ConfiguredWebhookEvent {
    #[serde(rename = "incoming")]
    IncomingMessage,

    #[serde(rename = "outgoing")]
    OutgoingMessage,

    #[serde(rename = "delivery")]
    DeliveryReport
}

#[derive(Debug, Deserialize)]
pub struct SentryConfig {
    pub dsn: String
}

#[derive(Debug, Deserialize)]
pub struct HTTPConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "default_http_address")]
    pub address: SocketAddr
}
impl Default for HTTPConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            address: default_http_address()
        }
    }
}

fn default_modem_device() -> String { "/dev/ttyS0".to_string() }
fn default_modem_baud() -> u32 { 115200 }
fn default_modem_read_interval() -> Duration { Duration::from_millis(30) }
fn default_modem_cmd_buffer_size() -> usize { 32 }
fn default_webhook_events() -> Vec<ConfiguredWebhookEvent> { vec![ConfiguredWebhookEvent::IncomingMessage] }
fn default_http_address() -> SocketAddr { SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 3000) }

fn deserialize_duration_from_millis<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let millis = u64::deserialize(deserializer)?;
    Ok(Duration::from_millis(millis))
}

fn deserialize_encryption_key<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    let decoded = general_purpose::STANDARD.decode(&s)
        .map_err(|e| serde::de::Error::custom(format!("Failed to decode base64 encryption key: {}", e)))?;

    if decoded.len() != 32 {
        return Err(serde::de::Error::custom(format!("Encryption key must be 32 bytes, got {}", decoded.len())));
    }

    let mut key = [0u8; 32];
    key.copy_from_slice(&decoded);
    Ok(key)
}
