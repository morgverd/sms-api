mod routes;
mod types;
mod macros;

use axum::{
    response::Json,
    routing::{get, post},
    Router
};
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use crate::http::routes::*;
use crate::app::HttpState;
use crate::http::types::{HttpResponse, JsonResult};
use crate::modem::types::{ModemRequest, ModemResponse};

#[cfg(feature = "sentry")]
use {
    sentry::integrations::tower::{NewSentryLayer, SentryHttpLayer},
    axum::{body::Body, http::Request}
};

async fn get_modem_json_result(
    state: HttpState,
    request: ModemRequest
) -> JsonResult<ModemResponse> {
    let response = match state.sms_manager.send_command(request).await {
        Ok(response) => response,
        Err(e) => {
            return Ok(Json(HttpResponse {
                success: false,
                response: None,
                error: Some(e.to_string())
            }))
        }
    };

    Ok(Json(HttpResponse {
        success: true,
        response: Some(response),
        error: None,
    }))
}

pub fn create_app(state: HttpState, _sentry: bool) -> Router {
    let router = Router::new()
        .route("/db/sms", post(db_sms))
        .route("/db/latest-numbers", post(db_latest_numbers))
        .route("/db/delivery-reports", post(db_delivery_reports))
        .route("/sms/send", post(sms_send))
        .route("/sms/network-status", get(sms_get_network_status))
        .route("/sms/signal-strength", get(sms_get_signal_strength))
        .route("/sms/network-operator", get(sms_get_network_operator))
        .route("/sms/service-provider", get(sms_get_service_provider))
        .route("/sms/battery-level", get(sms_get_battery_level))
        .layer(
            ServiceBuilder::new().layer(CorsLayer::permissive())
        );

    #[cfg(feature = "sentry")]
    let router = if _sentry {
        log::debug!("Adding Sentry Axum layer");
        router
            .layer(
                ServiceBuilder::new().layer(NewSentryLayer::<Request<Body>>::new_from_top())
            )
            .layer(
                ServiceBuilder::new().layer(SentryHttpLayer::new().enable_transaction())
            )
    } else {
        router
    };

    router.with_state(state)
}