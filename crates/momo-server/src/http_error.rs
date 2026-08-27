use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use momo_core::ResponseError;
use serde_json::json;

use super::MAX_PUBLIC_ERROR_BYTES;

pub(super) struct ApiError {
    pub(super) status: StatusCode,
    pub(super) error: ResponseError,
}

impl ApiError {
    pub(super) fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request", message, false)
    }

    pub(super) fn internal(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "orchestration_error",
            message,
            false,
        )
    }

    pub(super) fn bad_gateway(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, "upstream_error", message, true)
    }

    pub(super) fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "idempotency_conflict", message, false)
    }

    pub(super) fn gateway_timeout(message: impl Into<String>) -> Self {
        Self::new(StatusCode::GATEWAY_TIMEOUT, "timeout", message, true)
    }

    pub(super) fn rate_limited(message: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limit", message, true)
    }

    pub(super) fn cancelled() -> Self {
        Self::new(
            StatusCode::from_u16(499).expect("valid non-standard client closed status"),
            "cancelled",
            "response request was cancelled",
            false,
        )
    }

    pub(super) fn new(
        status: StatusCode,
        code: &'static str,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            status,
            error: ResponseError {
                error_type: code.to_owned(),
                code: code.to_owned(),
                message: sanitize_error_message(&message.into()),
                retryable,
                upstream_status: None,
                request_id: None,
            },
        }
    }

    pub(super) fn with_request_id(mut self, request_id: &str) -> Self {
        self.error.request_id = Some(request_id.to_owned());
        self
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "error": self.error,
            })),
        )
            .into_response()
    }
}

pub(super) fn sanitize_error_message(message: &str) -> String {
    let mut output = message
        .chars()
        .take(MAX_PUBLIC_ERROR_BYTES)
        .collect::<String>();
    for marker in ["Bearer ", "api_key=", "api-key=", "x-api-key="] {
        if let Some(start) = output
            .to_ascii_lowercase()
            .find(&marker.to_ascii_lowercase())
        {
            let value_start = start + marker.len();
            let value_end = output[value_start..]
                .find(|character: char| {
                    character.is_whitespace() || matches!(character, ',' | '"' | '}')
                })
                .map_or(output.len(), |offset| value_start + offset);
            output.replace_range(value_start..value_end, "[REDACTED]");
        }
    }
    output
        .split_whitespace()
        .map(|token| {
            if token.starts_with("sk-") && token.len() > 8 {
                "[REDACTED]"
            } else {
                token
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
