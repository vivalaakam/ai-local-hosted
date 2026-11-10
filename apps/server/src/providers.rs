use std::sync::Arc;

use alh_storage::{ProviderRow, ProviderUpdate, StorageError, normalize_protocol};
use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::error::ApiError;
use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/providers", get(list_providers).post(create_provider))
        .route(
            "/api/providers/{id}",
            axum::routing::patch(patch_provider).delete(delete_provider),
        )
}

async fn list_providers(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> Response {
    match state.store.list_providers(user).await {
        Ok(rows) => Json(json!({
            "data": rows.iter().map(provider_json).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(err) => ApiError::database(err).response(),
    }
}

async fn create_provider(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<Value>,
) -> Response {
    let row = match provider_from_create(user, &body) {
        Ok(row) => row,
        Err(err) => return err.response(),
    };
    let id = row.id;
    match state.store.insert_provider(&row).await {
        Ok(()) => match state.store.get_provider(user, id).await {
            Ok(Some(row)) => (StatusCode::CREATED, Json(provider_json(&row))).into_response(),
            Ok(None) => ApiError::not_found("provider not found").response(),
            Err(err) => ApiError::database(err).response(),
        },
        Err(StorageError::Duplicate) => {
            ApiError::conflict_msg("provider already exists").response()
        }
        Err(err) => ApiError::database(err).response(),
    }
}

async fn patch_provider(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
    Json(body): Json<Value>,
) -> Response {
    let patch = match provider_patch(&body) {
        Ok(patch) => patch,
        Err(err) => return err.response(),
    };
    match state.store.update_provider(user, id, patch).await {
        Ok(Some(row)) => Json(provider_json(&row)).into_response(),
        Ok(None) => ApiError::not_found("provider not found").response(),
        Err(err) => ApiError::database(err).response(),
    }
}

async fn delete_provider(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> Response {
    match state.store.delete_provider(user, id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => ApiError::not_found("provider not found").response(),
        Err(StorageError::InUse) => {
            ApiError::conflict_msg("provider is used by conversations").response()
        }
        Err(err) => ApiError::database(err).response(),
    }
}

fn provider_json(row: &ProviderRow) -> Value {
    json!({
        "id": row.id,
        "protocol": row.protocol,
        "display_name": row.display_name,
        "url": row.url,
        "has_token": row.token.as_deref().is_some_and(|token| !token.is_empty()),
        "default_model": row.default_model,
        "is_default": row.is_default,
        "enabled": row.enabled,
    })
}

fn provider_from_create(user_id: Uuid, body: &Value) -> Result<ProviderRow, ApiError> {
    let protocol = required_protocol(body)?;
    let url = required_url(body)?;
    let display_name = required_string(body, "display_name")?;
    if display_name.chars().count() > 200 {
        return Err(ApiError::invalid("display_name is too long"));
    }
    Ok(ProviderRow {
        user_id,
        id: Uuid::new_v4(),
        protocol: protocol.to_string(),
        display_name,
        url,
        token: limited_token(optional_string(body, "token")?)?,
        default_model: blank(optional_string(body, "default_model")?),
        is_default: optional_bool(body, "is_default")?.unwrap_or(false),
        enabled: optional_bool(body, "enabled")?.unwrap_or(true),
    })
}

fn provider_patch(body: &Value) -> Result<ProviderUpdate, ApiError> {
    if !body.is_object() {
        return Err(ApiError::invalid("provider body must be an object"));
    }
    let protocol = match present_string(body, "protocol")? {
        Some(protocol) => Some(
            normalize_protocol(&protocol)
                .ok_or_else(|| ApiError::invalid("protocol must be openai or anthropic"))?
                .to_string(),
        ),
        None => None,
    };
    let url = match present_string(body, "url")? {
        Some(url) => Some(checked_url(&url)?),
        None => None,
    };
    let display_name = match present_string(body, "display_name")? {
        Some(name) if !name.trim().is_empty() && name.chars().count() <= 200 => {
            Some(name.trim().to_string())
        }
        Some(_) => return Err(ApiError::invalid("display_name is invalid")),
        None => None,
    };
    Ok(ProviderUpdate {
        protocol,
        display_name,
        url,
        token: match nullable_string(body, "token")? {
            None => None,
            Some(token) => Some(limited_token(token)?),
        },
        default_model: nullable_string(body, "default_model")?.map(blank),
        is_default: optional_bool(body, "is_default")?,
        enabled: optional_bool(body, "enabled")?,
    })
}

fn required_protocol(body: &Value) -> Result<&'static str, ApiError> {
    let protocol = required_string(body, "protocol")?;
    normalize_protocol(&protocol)
        .ok_or_else(|| ApiError::invalid("protocol must be openai or anthropic"))
}

fn required_url(body: &Value) -> Result<String, ApiError> {
    checked_url(&required_string(body, "url")?)
}

fn checked_url(url: &str) -> Result<String, ApiError> {
    let url = url.trim();
    if url.len() > 2000 || !valid_http_url(url) {
        return Err(ApiError::invalid("url must be an http or https address"));
    }
    Ok(url.to_string())
}

fn required_string(body: &Value, key: &str) -> Result<String, ApiError> {
    match body.get(key) {
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(value.trim().to_string()),
        Some(Value::String(_)) => Err(ApiError::invalid(format!("{key} is empty"))),
        _ => Err(ApiError::invalid(format!("{key} is required"))),
    }
}

fn present_string(body: &Value, key: &str) -> Result<Option<String>, ApiError> {
    match body.get(key) {
        None => Ok(None),
        Some(Value::Null) => Err(ApiError::invalid(format!("{key} cannot be null"))),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(ApiError::invalid(format!("{key} must be a string"))),
    }
}

fn optional_string(body: &Value, key: &str) -> Result<Option<String>, ApiError> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(ApiError::invalid(format!("{key} must be a string"))),
    }
}

fn nullable_string(body: &Value, key: &str) -> Result<Option<Option<String>>, ApiError> {
    match body.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(value)) => Ok(Some(Some(value.clone()))),
        Some(_) => Err(ApiError::invalid(format!("{key} must be a string"))),
    }
}

fn optional_bool(body: &Value, key: &str) -> Result<Option<bool>, ApiError> {
    match body.get(key) {
        None => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(ApiError::invalid(format!("{key} must be a boolean"))),
    }
}

fn limited_token(value: Option<String>) -> Result<Option<String>, ApiError> {
    let value = blank(value);
    if value.as_ref().is_some_and(|token| token.len() > 8192) {
        return Err(ApiError::invalid("token is too long"));
    }
    Ok(value)
}

fn blank(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn valid_http_url(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty() && !host.contains(' ') && !host.contains('@')
}
