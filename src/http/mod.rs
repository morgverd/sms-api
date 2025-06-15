mod routes;
mod types;
mod macros;

use axum::{
    response::Json,
    routing::{get, post},
    Router,
};
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use crate::AppState;
use crate::http::routes::*;
use crate::http::types::{HttpResponse, JsonResult};
use crate::modem::types::{ModemRequest, ModemResponse};

async fn get_modem_json_result(
    state: AppState,
    request: ModemRequest
) -> JsonResult<ModemResponse> {
    let response = match state.sms_manager.send_command(request).await {
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
        .route("/db/sms", post(db_sms))
        .route("/db/latest-numbers", post(db_latest_numbers))
        .route("/sms/send", post(sms_send))
        .route("/sms/network-status", get(sms_get_network_status))
        .route("/sms/signal-strength", get(sms_get_signal_strength))
        .route("/sms/network-operator", get(sms_get_network_operator))
        .route("/sms/service-provider", get(sms_get_service_provider))
        .route("/sms/battery-level", get(sms_get_battery_level))
        .layer(
            ServiceBuilder::new()
                .layer(CorsLayer::permissive())
        )
        .with_state(state)
}