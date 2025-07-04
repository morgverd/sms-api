
#[macro_export]
macro_rules! http_modem_handler {
    ($fn_name:ident, $modem_req:expr) => {
        pub async fn $fn_name(
            State(state): State<AppState>
        ) -> crate::http::types::JsonResult<crate::modem::types::ModemResponse> {
            get_modem_json_result(state, $modem_req).await
        }
    };
}

#[macro_export]
macro_rules! http_post_handler {
    (
        $fn_name:ident,
        Option<$request_type:ty>,
        $response_type:ty,
        |$state:ident, $payload:ident| $db_call:block
    ) => {
        pub async fn $fn_name(
            axum::extract::State($state): axum::extract::State<AppState>,
            payload: Option<axum::Json<$request_type>>
        ) -> crate::http::types::JsonResult<$response_type> {
            async fn inner(
                $state: AppState,
                $payload: Option<$request_type>,
            ) -> anyhow::Result<$response_type> {
                $db_call
            }

            let $payload = payload.map(|json| json.0);

            match inner($state, $payload).await {
                Ok(data) => Ok(axum::Json(HttpResponse {
                    success: true,
                    response: Some(data),
                    error: None
                })),
                Err(e) => Err((
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(HttpResponse {
                        success: false,
                        response: None,
                        error: Some(e.to_string())
                    })
                ))
            }
        }
    };

    (
        $fn_name:ident,
        $request_type:ty,
        $response_type:ty,
        |$state:ident, $payload:ident| $db_call:block
    ) => {
        pub async fn $fn_name(
            axum::extract::State($state): axum::extract::State<AppState>,
            axum::Json($payload): axum::Json<$request_type>
        ) -> crate::http::types::JsonResult<$response_type> {
            async fn inner(
                $state: AppState,
                $payload: $request_type,
            ) -> anyhow::Result<$response_type> {
                $db_call
            }

            match inner($state, $payload).await {
                Ok(data) => Ok(axum::Json(HttpResponse {
                    success: true,
                    response: Some(data),
                    error: None
                })),
                Err(e) => Err((
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(HttpResponse {
                        success: false,
                        response: None,
                        error: Some(e.to_string())
                    })
                ))
            }
        }
    };
}
