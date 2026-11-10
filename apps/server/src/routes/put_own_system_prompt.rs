async fn put_own_system_prompt(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<OwnSystemPromptBody>,
) -> Response {
    if body
        .content
        .as_ref()
        .is_some_and(|content| content.chars().count() > 100_000)
    {
        return ApiError::invalid("system prompt is too long").response();
    }
    match state
        .store
        .set_own_system_prompt(user, body.content.as_deref())
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => ApiError::database(err).response(),
    }
}

