use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use crate::modem::types::ModemResponse;

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
    pub content: String
}

#[derive(Serialize)]
pub struct SendSmsResponse {
    pub message_id: i64,
    pub response: ModemResponse
}