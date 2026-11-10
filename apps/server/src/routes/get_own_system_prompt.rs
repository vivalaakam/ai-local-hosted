async fn get_own_system_prompt(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> Response {
    match state.store.system_prompt_layers(user).await {
        Ok(layers) => Json(json!({ "content": layers.own })).into_response(),
        Err(err) => ApiError::database(err).response(),
    }
}

