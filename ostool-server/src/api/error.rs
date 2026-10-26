use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::api::models::ErrorResponse;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub retry_after: Option<u32>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        let message = message.into();
        let message = if status.is_server_error() {
            log::error!("API {code}: {message}");
            "服务暂时不可用，请稍后重试；如持续失败，请联系管理员并提供请求 ID".into()
        } else {
            message
        };
        Self {
            status,
            code,
            message,
            retry_after: None,
        }
    }

    /// Preserve expected validation/conflict statuses, but never send underlying
    /// storage or process diagnostics to a client (including SSE projections).
    pub fn operation(error: anyhow::Error, status: StatusCode, message: &'static str) -> Self {
        log::warn!("API operation failed: {error:#}");
        if error.is::<sqlx::Error>() || error.is::<std::io::Error>() {
            return Self::internal("底层存储或系统调用失败");
        }
        Self::new(status, "operation_failed", message)
    }

    pub fn with_retry_after(mut self, seconds: u32) -> Self {
        self.retry_after = Some(seconds);
        self
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }

    pub fn payload_too_large(message: impl Into<String>) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }

    pub fn mac_already_bound(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "mac_already_bound", message)
    }

    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "service_unavailable",
            message,
        )
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_server_error",
            message,
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let retry = self.retry_after;
        let mut response = (
            self.status,
            Json(ErrorResponse {
                code: self.code.to_string(),
                message: self.message,
                details: retry.map(|seconds| serde_json::json!({"retry_after_seconds":seconds})),
            }),
        )
            .into_response();
        if let Some(seconds) = retry {
            response.headers_mut().insert(
                "retry-after",
                seconds.to_string().parse().expect("numeric retry interval"),
            );
        }
        response
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(value: anyhow::Error) -> Self {
        log::error!("request failed: {value:#}");
        Self::internal("服务暂时不可用，请稍后重试")
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        log::error!("security database operation failed: {error}");
        Self::service_unavailable("服务暂时不可用，请稍后重试")
    }
}
