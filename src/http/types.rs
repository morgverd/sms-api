use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use crate::modem::types::ModemResponse;

pub type ModemJsonResult = Result<Json<HttpResponse<ModemResponse>>, StatusCode>;

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