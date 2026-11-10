async fn models(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    headers: HeaderMap,
) -> Response {
    let ids = match model_ids(&state, user).await {
        Ok(ids) => ids,
        Err(err) => return err.response(),
    };
    if headers.get("anthropic-version").is_some() {
        Json(json!({
            "data": ids.iter().map(|id| json!({
                "type": "model",
                "id": id,
                "display_name": id,
                "created_at": "2026-01-01T00:00:00Z",
            })).collect::<Vec<_>>(),
            "has_more": false,
        }))
        .into_response()
    } else {
        Json(json!({
            "object": "list",
            "data": ids.iter().map(|id| json!({
                "id": id,
                "object": "model",
                "owned_by": "alh",
            })).collect::<Vec<_>>(),
        }))
        .into_response()
    }
}

