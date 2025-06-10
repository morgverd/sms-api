use serde::{Deserialize, Serialize};
use crate::modem::ModemManager;
use crate::modem::sender::ModemSender;

#[derive(Clone)]
pub struct AppState {
    pub sender: ModemSender
}

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