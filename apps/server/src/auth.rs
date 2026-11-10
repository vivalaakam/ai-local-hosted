use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Clone, Copy, Debug)]
pub struct CurrentUser(pub Uuid);

pub async fn require_auth(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if path == "/health" || path == "/oauth/token" || path == "/oauth/register" {
        return next.run(request).await;
    }
    let presented = bearer(request.headers().get("authorization")).or_else(|| {
        request
            .headers()
            .get("x-api-key")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(str::to_string)
    });
    if let Some(token) = presented {
        match state.store.token_user(&token_hash(&token), "access").await {
            Ok(Some(user_id)) => {
                request.extensions_mut().insert(CurrentUser(user_id));
                return next.run(request).await;
            }
            Ok(None) => {}
            Err(err) => return ApiError::database(err).response(),
        }
    }
    let mut error = ApiError::unauthorized();
    if path.starts_with("/v1/messages") {
        error = error.anthropic();
    }
    error.response()
}

pub fn token_hash(token: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(token.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push(HEX[(byte >> 4) as usize] as char);
        hex.push(HEX[(byte & 0x0f) as usize] as char);
    }
    hex
}

fn bearer(value: Option<&axum::http::HeaderValue>) -> Option<String> {
    let value = value?.to_str().ok()?.trim();
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?;
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}
