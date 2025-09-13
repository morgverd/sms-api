mod routes;
mod types;
pub mod websocket;

use anyhow::{bail, Result};
use axum::http::{HeaderName, HeaderValue};
use axum::routing::{get, post};
use tracing::log::{info, warn};
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use crate::TracingReloadHandle;
use crate::http::types::{HttpResponse, JsonResult};
use crate::modem::types::{ModemRequest, ModemResponse};
use crate::config::HTTPConfig;
use crate::sms::SMSManager;
use crate::http::websocket::WebSocketManager;
use crate::http::routes::*;

#[cfg(feature = "sentry")]
use sentry::integrations::tower::{NewSentryLayer, SentryHttpLayer};

#[derive(Clone)]
pub struct HttpState {
    pub sms_manager: SMSManager,
    pub config: HTTPConfig,
    pub tracing_reload: TracingReloadHandle,
    pub websocket: Option<WebSocketManager>
}

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

pub fn create_app(
    config: HTTPConfig,
    websocket: Option<WebSocketManager>,
    sms_manager: SMSManager,
    tracing_reload: TracingReloadHandle,
    _sentry: bool
) -> Result<axum::Router> {
    let mut router = axum::Router::new()
        .route("/db/sms", post(db_sms))
        .route("/db/latest-numbers", post(db_latest_numbers))
        .route("/db/delivery-reports", post(db_delivery_reports))
        .route("/db/friendly-names/set", post(friendly_names_set))
        .route("/db/friendly-names/get", post(friendly_names_get))
        .route("/sms/send", post(sms_send))
        .route("/sms/network-status", get(sms_get_network_status))
        .route("/sms/signal-strength", get(sms_get_signal_strength))
        .route("/sms/network-operator", get(sms_get_network_operator))
        .route("/sms/service-provider", get(sms_get_service_provider))
        .route("/sms/battery-level", get(sms_get_battery_level))
        .route("/gnss/status", get(gnss_get_status))
        .route("/gnss/location", get(gnss_get_location))
        .route("/sys/phone-number", get(sys_phone_number))
        .route("/sys/version", get(sys_version))
        .route("/sys/set-log-level", post(sys_set_log_level))
        .layer(
            SetResponseHeaderLayer::overriding(
                HeaderName::from_static("x-version"),
                HeaderValue::from_static(crate::VERSION)
            )
        )
        .layer(
            ServiceBuilder::new().layer(CorsLayer::permissive())
        );

    // Add optional websocket route if there is a manager.
    if websocket.is_some() {
        info!("Adding WebSocket broadcaster HTTP route!");
        router = router.route("/ws", get(websocket_upgrade));
    }

    // Add optional authentication middleware.
    if config.require_authentication {
        match std::env::var("SMS_HTTP_AUTH_TOKEN") {
            Ok(token) => {
                info!("Adding HTTP authentication middleware!");
                router = router.layer(
                    axum::middleware::from_fn_with_state(token, auth_middleware)
                );
            },
            Err(_) => bail!("Missing required SMS_HTTP_AUTH_TOKEN environment variable, and require_authentication is enabled!")
        }
    } else {
        warn!("Serving HTTP without authentication middleware, as require_authentication is disabled!");
    }

    // If Sentry is enabled, include axum integration layers.
    #[cfg(feature = "sentry")]
    if _sentry {
        info!("Adding Sentry HTTP layer!");
        router = router
            .layer(
                ServiceBuilder::new().layer(NewSentryLayer::<axum::http::Request<axum::body::Body>>::new_from_top())
            )
            .layer(
                ServiceBuilder::new().layer(SentryHttpLayer::new().enable_transaction())
            )
    }

    // Shared HTTP route state.
    let state = HttpState {
        sms_manager,
        config,
        tracing_reload,
        websocket
    };
    Ok(router.with_state(state))
}