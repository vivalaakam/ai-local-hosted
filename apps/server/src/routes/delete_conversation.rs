async fn delete_conversation(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> Response {
    match state.store.delete_conversation(user, id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => ApiError::not_found("conversation not found").response(),
        Err(err) => ApiError::database(err).response(),
    }
}

