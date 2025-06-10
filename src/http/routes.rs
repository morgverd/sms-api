use axum::extract::State;
use axum::Json;
use crate::http::{get_modem_json_result, ModemJsonResult};
use crate::http::types::{AppState, HttpResponse, SendSmsRequest};
use crate::modem::types::ModemRequest;

pub async fn send_sms(
    State(mut state): State<AppState>,
    Json(payload): Json<SendSmsRequest>,
) -> ModemJsonResult {
    let response = match state.sender.send_sms(payload.to, &*payload.content).await {
        Ok(response) => response,
        Err(e) => {
            return Ok(Json(HttpResponse {
                success: false,
                data: None,
                error: Some(e.to_string())
            }))
        }
    };

    Ok(Json(HttpResponse {
        success: true,
        data: Some(response),
        error: None,
    }))
}

macro_rules! modem_get_handler {
    ($fn_name:ident, $modem_req:expr) => {
        pub async fn $fn_name(
            State(state): State<AppState>
        ) -> ModemJsonResult {
            get_modem_json_result(state, $modem_req).await
        }
    };
}

modem_get_handler!(get_network_status, ModemRequest::GetNetworkStatus);
modem_get_handler!(get_signal_strength, ModemRequest::GetSignalStrength);
modem_get_handler!(get_network_operator, ModemRequest::GetNetworkOperator);
modem_get_handler!(get_service_provider, ModemRequest::GetServiceProvider);
modem_get_handler!(get_battery_level, ModemRequest::GetBatteryLevel);