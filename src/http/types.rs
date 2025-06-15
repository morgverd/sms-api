use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use crate::modem::types::ModemResponse;
use crate::sms::types::SMSMessage;

pub type JsonResult<T> = Result<Json<HttpResponse<T>>, (StatusCode, Json<HttpResponse<T>>)>;

#[derive(Serialize)]
pub struct HttpResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct SendSmsRequest {
    pub to: String,
    pub content: String
}

#[derive(Serialize)]
pub struct SendSmsResponse {
    pub message_id: i64,
    pub response: ModemResponse
}

#[derive(Deserialize)]
pub struct FetchSmsRequest {
    pub phone_number: String,
    pub limit: u32,
    pub offset: u32
}

#[derive(Serialize)]
pub struct FetchSmsResponse {
    pub messages: Vec<SMSMessage>
}

#[derive(Deserialize)]
pub struct FetchLatestNumbersRequest {
    pub limit: u32,
    pub offset: u32
}

#[derive(Serialize)]
pub struct FetchLatestNumbersResponse {
    pub numbers: Vec<String>
}
