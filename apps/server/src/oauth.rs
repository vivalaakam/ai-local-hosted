use std::sync::Arc;
use std::time::Duration;

use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use axum::Form;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{Duration as ChronoDuration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use alh_storage::StorageError;

use crate::auth::token_hash;
use crate::error::ApiError;
use crate::state::AppState;

const ACCESS_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const REFRESH_TTL: Duration = Duration::from_secs(90 * 24 * 60 * 60);
const INVALID_LOGIN: &str = "invalid username or password";

#[derive(Deserialize)]
pub(crate) struct RegisterBody {
    username: String,
    password: String,
}

#[derive(Deserialize)]
pub(crate) struct TokenForm {
    grant_type: String,
    username: Option<String>,
    password: Option<String>,
    refresh_token: Option<String>,
}

pub(crate) async fn register(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RegisterBody>,
) -> Response {
    let username = body.username.trim().to_string();
    if !valid_username(&username) {
        return ApiError::invalid("username must be 1-64 letters, digits, _ or -").response();
    }
    if body.password.len() < 8 {
        return ApiError::invalid("password must be at least 8 characters").response();
    }
    let password_hash = match hash_password(&body.password).await {
        Ok(hash) => hash,
        Err(err) => return ApiError::database(err).response(),
    };
    let user_id = Uuid::new_v4();
    match state
        .store
        .register_user(user_id, &username, &password_hash)
        .await
    {
        Ok(()) => {}
        Err(StorageError::Duplicate) => {
            return ApiError::conflict_msg("username already exists").response();
        }
        Err(err) => return ApiError::database(err).response(),
    }
    match issue(&state, user_id).await {
        Ok(body) => (StatusCode::CREATED, Json(body)).into_response(),
        Err(err) => err.response(),
    }
}

pub(crate) async fn token(
    State(state): State<Arc<AppState>>,
    Form(form): Form<TokenForm>,
) -> Response {
    match form.grant_type.as_str() {
        "password" => password_grant(&state, form.username, form.password).await,
        "refresh_token" => refresh_grant(&state, form.refresh_token).await,
        _ => ApiError::invalid("unsupported grant_type").response(),
    }
}

async fn password_grant(
    state: &AppState,
    username: Option<String>,
    password: Option<String>,
) -> Response {
    let username = username.unwrap_or_default();
    let password = password.unwrap_or_default();
    let username = username.trim();
    if !valid_username(username) || password.len() < 8 {
        return ApiError::invalid_grant(INVALID_LOGIN).response();
    }
    let account = match state.store.user_by_name(username).await {
        Ok(account) => account,
        Err(err) => return ApiError::database(err).response(),
    };
    let Some(account) = account else {
        return ApiError::invalid_grant(INVALID_LOGIN).response();
    };
    if !verify_password(&password, &account.password_hash).await {
        return ApiError::invalid_grant(INVALID_LOGIN).response();
    }
    match issue(state, account.id).await {
        Ok(body) => Json(body).into_response(),
        Err(err) => err.response(),
    }
}

async fn refresh_grant(state: &AppState, refresh_token: Option<String>) -> Response {
    let Some(refresh_token) = refresh_token.filter(|token| !token.trim().is_empty()) else {
        return ApiError::invalid_grant("invalid refresh token").response();
    };
    let hash = token_hash(refresh_token.trim());
    let user_id = match state.store.token_user(&hash, "refresh").await {
        Ok(Some(user_id)) => user_id,
        Ok(None) => return ApiError::invalid_grant("invalid refresh token").response(),
        Err(err) => return ApiError::database(err).response(),
    };
    if let Err(err) = state.store.delete_token(&hash).await {
        return ApiError::database(err).response();
    }
    match issue(state, user_id).await {
        Ok(body) => Json(body).into_response(),
        Err(err) => err.response(),
    }
}

async fn issue(state: &AppState, user_id: Uuid) -> Result<Value, ApiError> {
    let access = new_token();
    let refresh = new_token();
    let now = Utc::now();
    state
        .store
        .put_token(
            &token_hash(&access),
            user_id,
            "access",
            now + ChronoDuration::seconds(ACCESS_TTL.as_secs() as i64),
        )
        .await
        .map_err(ApiError::database)?;
    state
        .store
        .put_token(
            &token_hash(&refresh),
            user_id,
            "refresh",
            now + ChronoDuration::seconds(REFRESH_TTL.as_secs() as i64),
        )
        .await
        .map_err(ApiError::database)?;
    Ok(json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": ACCESS_TTL.as_secs(),
        "refresh_token": refresh,
    }))
}

fn new_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn valid_username(username: &str) -> bool {
    !username.is_empty()
        && username.len() <= 64
        && username
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

async fn hash_password(password: &str) -> Result<String, String> {
    let password = password.to_string();
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|hash| hash.to_string())
            .map_err(|err| err.to_string())
    })
    .await
    .map_err(|err| err.to_string())?
}

async fn verify_password(password: &str, password_hash: &str) -> bool {
    let password = password.to_string();
    let password_hash = password_hash.to_string();
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .verify_password(password.as_bytes(), password_hash.as_str())
            .is_ok()
    })
    .await
    .unwrap_or(false)
}
