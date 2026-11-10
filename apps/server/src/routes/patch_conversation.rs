async fn patch_conversation(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
    Json(body): Json<Value>,
) -> Response {
    let patch = ConversationPatch {
        title: match body.get("title") {
            Some(Value::String(title)) => Some(title.clone()),
            _ => None,
        },
        system_prompt: match body.get("system_prompt") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(Value::String(prompt)) => Some(Some(prompt.clone())),
            Some(_) => None,
        },
        model: body
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string),
        archived: body.get("archived").and_then(Value::as_bool),
    };
    match state.store.patch_conversation(user, id, patch).await {
        Ok(Some(conversation)) => Json(conversation_json(&conversation, None)).into_response(),
        Ok(None) => ApiError::not_found("conversation not found").response(),
        Err(err) => ApiError::database(err).response(),
    }
}

