use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use alh_core::{ProviderError, ProviderErrorKind};

pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    anthropic: bool,
}

impl ApiError {
    pub fn response(self) -> Response {
        if self.anthropic {
            let kind = match self.code {
                "unauthorized" => "authentication_error",
                "invalid_request" | "conflict" => "invalid_request_error",
                "not_found" | "model_not_found" => "not_found_error",
                "provider_unreachable" => "overloaded_error",
                _ => "api_error",
            };
            (
                self.status,
                Json(json!({
                    "type": "error",
                    "error": {"type": kind, "message": self.message},
                })),
            )
                .into_response()
        } else {
            (
                self.status,
                Json(json!({"error": {"code": self.code, "message": self.message}})),
            )
                .into_response()
        }
    }

    pub fn anthropic(mut self) -> Self {
        self.anthropic = true;
        self
    }

    pub fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthorized",
            message: "missing or invalid access token".into(),
            anthropic: false,
        }
    }

    pub fn invalid_grant(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_grant",
            message: message.into(),
            anthropic: false,
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_request",
            message: message.into(),
            anthropic: false,
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: message.into(),
            anthropic: false,
        }
    }

    pub fn model_not_found(model: impl Into<String>) -> Self {
        let model = model.into();
        Self {
            status: StatusCode::NOT_FOUND,
            code: "model_not_found",
            message: format!("model not found: {model}"),
            anthropic: false,
        }
    }

    pub fn conflict() -> Self {
        Self::conflict_msg("a response is already streaming for this conversation")
    }

    pub fn conflict_msg(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "conflict",
            message: message.into(),
            anthropic: false,
        }
    }

    pub fn database(err: impl std::fmt::Display) -> Self {
        tracing::error!(error = %err, "database request failed");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "upstream",
            message: "database error".into(),
            anthropic: false,
        }
    }

    pub fn from_provider(err: ProviderError) -> Self {
        if let Some(detail) = err.detail() {
            tracing::warn!(error = %err, detail, "provider request failed");
        } else {
            tracing::warn!(error = %err, "provider request failed");
        }
        let (status, code) = match err.kind {
            ProviderErrorKind::NotFound => (StatusCode::NOT_FOUND, "model_not_found"),
            ProviderErrorKind::Unavailable | ProviderErrorKind::Timeout => {
                (StatusCode::BAD_GATEWAY, "provider_unreachable")
            }
            ProviderErrorKind::Cancelled => (StatusCode::BAD_GATEWAY, "interrupted"),
            ProviderErrorKind::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "upstream"),
            ProviderErrorKind::Invalid => (StatusCode::BAD_GATEWAY, "invalid_request"),
            ProviderErrorKind::Auth | ProviderErrorKind::Decode => {
                (StatusCode::BAD_GATEWAY, "upstream")
            }
        };
        Self {
            status,
            code,
            message: err.kind.public_message().into(),
            anthropic: false,
        }
    }
}

pub fn provider_code(err: &ProviderError) -> &'static str {
    match err.kind {
        ProviderErrorKind::NotFound => "model_not_found",
        ProviderErrorKind::Unavailable | ProviderErrorKind::Timeout => "provider_unreachable",
        ProviderErrorKind::Cancelled => "interrupted",
        ProviderErrorKind::Invalid => "invalid_request",
        _ => "upstream",
    }
}
