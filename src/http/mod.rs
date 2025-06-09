pub mod types;

use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use crate::http::types::{HttpResponse, AppState, SendSmsRequest};
use crate::modem::types::{ModemRequest, ModemResponse};

type ModemJsonResult = Result<Json<HttpResponse<ModemResponse>>, StatusCode>;

async fn get_signal_strength(
    State(mut state): State<AppState>
) -> ModemJsonResult {
    let response = match state.modem.send_command(ModemRequest::GetSignalStrength).await {
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

async fn send_sms(
    State(mut state): State<AppState>,
    Json(payload): Json<SendSmsRequest>,
) -> ModemJsonResult {
    let request = ModemRequest::SendSMS {
        len: payload.len,
        pdu: payload.pdu.to_string(),
    };

    let response = match state.modem.send_command(request).await {
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

pub fn create_app(state: AppState) -> Router {
    Router::new()
        .route("/sms/signal-strength", get(get_signal_strength))
        .route("/sms/send", post(send_sms))
        .layer(
            ServiceBuilder::new()
                .layer(CorsLayer::permissive())
        )
        .with_state(state)
}