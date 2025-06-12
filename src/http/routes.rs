use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use crate::AppState;
use crate::http::get_modem_json_result;
use crate::http::types::{FetchSmsRequest, FetchSmsResponse, HttpResponse, ModemJsonResult, SendSmsRequest};
use crate::modem::types::ModemRequest;
use crate::sms::types::SMSOutgoingMessage;

pub async fn fetch_sms(
    State(state): State<AppState>,
    Json(payload): Json<FetchSmsRequest>
) -> Result<Json<HttpResponse<FetchSmsResponse>>, StatusCode> {
    let result = state.sms_manager.borrow_database()
        .get_messages(&payload.phone_number, i64::from(payload.limit), i64::from(payload.offset))
        .await;
    
    let json = match result {
        Ok(messages) => Json(HttpResponse {
            success: true,
            data: Some(FetchSmsResponse { messages }),
            error: None
        }),
        Err(e) => Json(HttpResponse {
            success: false,
            data: None,
            error: Some(e.to_string())
        })
    };
    Ok(json)
}

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