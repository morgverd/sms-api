use axum::extract::State;
use axum::Json;
use crate::AppState;
use crate::http::get_modem_json_result;
use crate::http::types::{HttpResponse, ModemJsonResult, SendSmsRequest};
use crate::modem::types::ModemRequest;
use crate::sms::types::SMSOutgoingMessage;

pub async fn send_sms(
    State(state): State<AppState>,
    Json(payload): Json<SendSmsRequest>,
) -> ModemJsonResult {
    let message = SMSOutgoingMessage {
        phone_number: payload.to,
        content: payload.content,
    };
    let response = match state.sms_manager.send_sms(message).await {
        Ok((_, response)) => response,
        Err(e) => {
            return Ok(Json(HttpResponse {
                success: false,
                data: None,
                error: Some(e.to_string())
            }));
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