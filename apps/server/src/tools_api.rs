use std::sync::Arc;

use alh_storage::{NewTool, StorageError, ToolPatch, ToolRow};
use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::error::ApiError;
use crate::state::AppState;
use crate::tool_catalog::row_to_spec;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/tools", get(list_tools).post(create_tool))
        .route(
            "/api/tools/{id}",
            axum::routing::get(get_tool)
                .patch(patch_tool)
                .delete(delete_tool),
        )
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    #[serde(default)]
    include_source: bool,
}

async fn list_tools(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(query): Query<ListQuery>,
) -> Response {
    match state.store.list_tools_for_user(user).await {
        Ok(rows) => Json(json!({
            "data": rows
                .iter()
                .map(|row| tool_json(row, query.include_source))
                .collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(err) => ApiError::database(err).response(),
    }
}

async fn get_tool(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
    Query(query): Query<ListQuery>,
) -> Response {
    match state.store.get_tool(user, id).await {
        Ok(Some(row)) => Json(tool_json(&row, query.include_source)).into_response(),
        Ok(None) => ApiError::not_found("tool not found").response(),
        Err(err) => ApiError::database(err).response(),
    }
}

async fn create_tool(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<Value>,
) -> Response {
    let tool = match tool_from_create(user, &body) {
        Ok(tool) => tool,
        Err(err) => return err.response(),
    };
    // Validate script compiles by converting and checking kind.
    if let Err(err) = row_to_spec(ToolRow {
        id: tool.id,
        owner_user_id: Some(tool.owner_user_id),
        slug: tool.slug.clone(),
        name: tool.name.clone(),
        description: tool.description.clone(),
        parameters: tool.parameters.clone(),
        kind: "rhai".into(),
        source: Some(tool.source.clone()),
        enabled: tool.enabled,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }) {
        return ApiError::invalid(err).response();
    }
    let id = tool.id;
    match state.store.insert_user_tool(&tool).await {
        Ok(()) => match state.store.get_tool(user, id).await {
            Ok(Some(row)) => (StatusCode::CREATED, Json(tool_json(&row, true))).into_response(),
            Ok(None) => ApiError::not_found("tool not found").response(),
            Err(err) => ApiError::database(err).response(),
        },
        Err(StorageError::Duplicate) => {
            ApiError::conflict_msg("tool slug already exists").response()
        }
        Err(err) => ApiError::database(err).response(),
    }
}

async fn patch_tool(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
    Json(body): Json<Value>,
) -> Response {
    let patch = match tool_patch(&body) {
        Ok(patch) => patch,
        Err(err) => return err.response(),
    };
    match state.store.update_user_tool(user, id, patch).await {
        Ok(Some(row)) => Json(tool_json(&row, true)).into_response(),
        Ok(None) => ApiError::not_found("tool not found").response(),
        Err(StorageError::NotFound) => ApiError::not_found("tool not found").response(),
        Err(err) => ApiError::database(err).response(),
    }
}

async fn delete_tool(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> Response {
    match state.store.delete_user_tool(user, id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => ApiError::not_found("tool not found").response(),
        Err(err) => ApiError::database(err).response(),
    }
}

fn tool_json(row: &ToolRow, include_source: bool) -> Value {
    let mut value = json!({
        "id": row.id,
        "owner_user_id": row.owner_user_id,
        "slug": row.slug,
        "name": row.name,
        "description": row.description,
        "parameters": row.parameters,
        "kind": row.kind,
        "enabled": row.enabled,
        "model_name": model_tool_name(row),
        "created_at": row.created_at,
        "updated_at": row.updated_at,
    });
    if include_source {
        value["source"] = json!(row.source);
    }
    value
}

fn model_tool_name(row: &ToolRow) -> String {
    if row.owner_user_id.is_some() {
        format!("user__{}", row.slug)
    } else {
        row.slug.clone()
    }
}

fn tool_from_create(user: Uuid, body: &Value) -> Result<NewTool, ApiError> {
    let slug = required_slug(body)?;
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(slug)
        .to_string();
    let description = body
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let parameters = body
        .get("parameters")
        .cloned()
        .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
    if !parameters.is_object() {
        return Err(ApiError::invalid("parameters must be a json object"));
    }
    let source = body
        .get("source")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|source| !source.is_empty())
        .ok_or_else(|| ApiError::invalid("source is required"))?
        .to_string();
    if source.chars().count() > 200_000 {
        return Err(ApiError::invalid("source is too long"));
    }
    let enabled = body.get("enabled").and_then(Value::as_bool).unwrap_or(true);
    Ok(NewTool {
        id: Uuid::new_v4(),
        owner_user_id: user,
        slug: slug.to_string(),
        name,
        description,
        parameters,
        source,
        enabled,
    })
}

fn tool_patch(body: &Value) -> Result<ToolPatch, ApiError> {
    let mut patch = ToolPatch::default();
    if let Some(name) = body.get("name") {
        let name = name
            .as_str()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| ApiError::invalid("name must be a non-empty string"))?;
        patch.name = Some(name.to_string());
    }
    if let Some(description) = body.get("description") {
        let description = description
            .as_str()
            .ok_or_else(|| ApiError::invalid("description must be a string"))?;
        patch.description = Some(description.to_string());
    }
    if let Some(parameters) = body.get("parameters") {
        if !parameters.is_object() {
            return Err(ApiError::invalid("parameters must be a json object"));
        }
        patch.parameters = Some(parameters.clone());
    }
    if let Some(source) = body.get("source") {
        let source = source
            .as_str()
            .map(str::trim)
            .filter(|source| !source.is_empty())
            .ok_or_else(|| ApiError::invalid("source must be a non-empty string"))?;
        if source.chars().count() > 200_000 {
            return Err(ApiError::invalid("source is too long"));
        }
        patch.source = Some(source.to_string());
    }
    if let Some(enabled) = body.get("enabled") {
        let enabled = enabled
            .as_bool()
            .ok_or_else(|| ApiError::invalid("enabled must be a boolean"))?;
        patch.enabled = Some(enabled);
    }
    Ok(patch)
}

fn required_slug(body: &Value) -> Result<&str, ApiError> {
    let slug = body
        .get("slug")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|slug| !slug.is_empty())
        .ok_or_else(|| ApiError::invalid("slug is required"))?;
    if slug.len() > 64 {
        return Err(ApiError::invalid("slug is too long"));
    }
    if !slug
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Err(ApiError::invalid(
            "slug must be ascii alphanumeric, '_' or '-'",
        ));
    }
    if slug.starts_with("user__") {
        return Err(ApiError::invalid("slug must not start with user__"));
    }
    Ok(slug)
}
