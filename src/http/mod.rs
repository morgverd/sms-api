mod routes;
mod types;
mod macros;

use anyhow::{bail, Result};
use axum::routing::{get, post};
use log::{info, warn};
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use crate::http::routes::*;
use crate::app::HttpState;
use crate::http::types::{HttpResponse, JsonResult};
use crate::modem::types::{ModemRequest, ModemResponse};

#[cfg(feature = "sentry")]
use sentry::integrations::tower::{NewSentryLayer, SentryHttpLayer};

async fn get_modem_json_result(
    state: HttpState,
    request: ModemRequest
) -> JsonResult<ModemResponse> {
    let response = match state.sms_manager.send_command(request).await {
        Ok(response) => response,
        Err(e) => {
            return Ok(axum::response::Json(HttpResponse {
                success: false,
                response: None,
                error: Some(e.to_string())
            }))
        }
    };

    Ok(axum::response::Json(HttpResponse {
        success: true,
        response: Some(response),
        error: None,
    }))
}

async fn auth_middleware(
    axum::extract::State(expected_token): axum::extract::State<String>,
    headers: axum::http::HeaderMap,
    request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    let auth_header = headers
        .get("authorization")
        .ok_or(axum::http::StatusCode::UNAUTHORIZED)?;

    let auth_str = auth_header
        .to_str()
        .map_err(|_| axum::http::StatusCode::BAD_REQUEST)?
        .trim();

    let token = if auth_str.starts_with("Bearer ") {
        &auth_str[7..]
    } else {
        auth_str
    };

    if token != expected_token {
        return Err(axum::http::StatusCode::UNAUTHORIZED);
    }

    Ok(next.run(request).await)
}

pub fn create_app(state: HttpState, _sentry: bool) -> Result<axum::Router> {
    let mut router = axum::Router::new()
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

    // Add optional authentication middleware.
    let auth_token = std::env::var("SMS_API_HTTP_TOKEN");
    if state.config.require_authentication && auth_token.is_err() {
        bail!("Missing required SMS_API_HTTP_TOKEN environment variable, and require_authentication is enabled");
    }
    if let Ok(token) = auth_token {
        info!("Adding HTTP authentication middleware!");
        router = router.layer(
            axum::middleware::from_fn_with_state(token, auth_middleware)
        );
    } else {
        warn!("Serving HTTP without authentication middleware due to missing/invalid SMS_API_HTTP_TOKEN");
    }

    // If Sentry is enabled, include axum integration layers.
    #[cfg(feature = "sentry")]
    let router = if _sentry {
        log::debug!("Adding Sentry Axum layer");
        router
            .layer(
                ServiceBuilder::new().layer(NewSentryLayer::<axum::http::Request<axum::body::Body>>::new_from_top())
            )
            .layer(
                ServiceBuilder::new().layer(SentryHttpLayer::new().enable_transaction())
            )
    } else {
        router
    };

    Ok(router.with_state(state))
}