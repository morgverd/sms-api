use std::collections::HashSet;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use crate::events::EventType;

pub type JsonResult<T> = Result<Json<HttpResponse<T>>, (StatusCode, Json<HttpResponse<T>>)>;

#[derive(Serialize)]
pub struct HttpResponse<T> {
    pub success: bool,
    pub response: Option<T>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct PhoneNumberFetchRequest {
    pub phone_number: String,

    #[serde(default)]
    pub limit: Option<u64>,

    #[serde(default)]
    pub offset: Option<u64>,

    #[serde(default)]
    pub reverse: bool
}

#[derive(Deserialize)]
pub struct MessageIdFetchRequest {
    pub message_id: i64,

    #[serde(default)]
    pub limit: Option<u64>,

    #[serde(default)]
    pub offset: Option<u64>,

    #[serde(default)]
    pub reverse: bool
}

#[derive(Deserialize)]
pub struct GlobalFetchRequest {

    #[serde(default)]
    pub limit: Option<u64>,

    #[serde(default)]
    pub offset: Option<u64>,

    #[serde(default)]
    pub reverse: bool
}

#[derive(Deserialize)]
pub struct SendSmsRequest {
    pub to: String,
    pub content: String,

    #[serde(default)]
    pub flash: bool,

    #[serde(default)]
    pub validity_period: Option<u8>
}

#[derive(Deserialize)]
pub struct SetLogLevelRequest {
    pub level: String
}

#[derive(Serialize)]
pub struct SendSmsResponse {
    pub message_id: i64,
    pub reference_id: u8
}

#[derive(Deserialize)]
pub struct SetFriendlyNameRequest {
    pub phone_number: String,
    pub friendly_name: Option<String>
}

#[derive(Deserialize)]
pub struct GetFriendlyNameRequest {
    pub phone_number: String
}

#[derive(Deserialize)]
pub struct WebSocketQuery {
    pub events: Option<String>
}
impl WebSocketQuery {
    pub fn get_event_types(&self) -> Option<Vec<EventType>> {
        match &self.events {
            Some(events_str) if events_str == "*" => None, // Accept all events
            Some(events_str) => {
                let events: Vec<EventType> = events_str
                    .split(',')
                    .filter_map(|s| EventType::try_from(s.trim()).ok())
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect();

                if events.is_empty() {
                    None // If no valid events parsed, accept all
                } else {
                    Some(events)
                }
            },
            None => None // No filter specified, accept all events
        }
    }
}