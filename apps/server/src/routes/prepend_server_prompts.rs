async fn prepend_server_prompts(
    state: &AppState,
    user: Uuid,
    request: &mut CompletionRequest,
) -> Result<(), ApiError> {
    let layers = state
        .store
        .system_prompt_layers(user)
        .await
        .map_err(ApiError::database)?;
    let prompts = layers.effective();
    request
        .messages
        .splice(0..0, prompts.into_iter().map(CompletionMessage::system));
    Ok(())
}

