use serde::{Deserialize, Serialize};
use crate::modem::ModemManager;

#[derive(Clone)]
pub struct AppState {
    pub modem: ModemManager
}

#[derive(Serialize)]
pub struct HttpResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
pub struct SendSmsRequest {
    pub len: u64,
    pub pdu: String,
}