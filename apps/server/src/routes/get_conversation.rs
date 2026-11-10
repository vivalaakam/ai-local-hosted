async fn get_conversation(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> Response {
    let conversation = match state.store.get_conversation(user, id).await {
        Ok(Some(conversation)) => conversation,
        Ok(None) => return ApiError::not_found("conversation not found").response(),
        Err(err) => return ApiError::database(err).response(),
    };
    let messages = match state.store.list_messages(user, id).await {
        Ok(messages) => messages,
        Err(err) => return ApiError::database(err).response(),
    };
    Json(conversation_json(&conversation, Some(&messages))).into_response()
}

