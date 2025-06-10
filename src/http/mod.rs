pub mod types;
mod routes;

use axum::{
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use crate::http::routes::*;
use crate::http::types::{HttpResponse, AppState};
use crate::modem::types::{ModemRequest, ModemResponse};

type ModemJsonResult = Result<Json<HttpResponse<ModemResponse>>, StatusCode>;

async fn get_modem_json_result(
    mut state: AppState,
    request: ModemRequest
) -> ModemJsonResult {
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
        .route("/sms/send", post(send_sms))
        .route("/sms/network-status", get(get_network_status))
        .route("/sms/signal-strength", get(get_signal_strength))
        .route("/sms/network-operator", get(get_network_operator))
        .route("/sms/service-provider", get(get_service_provider))
        .route("/sms/battery-level", get(get_battery_level))
        .layer(
            ServiceBuilder::new()
                .layer(CorsLayer::permissive())
        )
        .with_state(state)
}