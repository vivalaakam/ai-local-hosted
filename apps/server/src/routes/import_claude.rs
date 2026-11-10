async fn import_claude(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    body: Bytes,
) -> Response {
    let providers = match state.store.list_providers(user).await {
        Ok(providers) => providers,
        Err(err) => return ApiError::database(err).response(),
    };
    let Some(provider_id) = providers
        .iter()
        .find(|provider| provider.enabled && provider.is_default)
        .or_else(|| providers.iter().find(|provider| provider.enabled))
        .map(|provider| provider.id)
    else {
        return ApiError::invalid("Add a provider before importing chats.").response();
    };
    let bytes = body.to_vec();
    let chats =
        match tokio::task::spawn_blocking(move || crate::claude_import::load_claude_chats(&bytes))
            .await
        {
            Ok(Ok(chats)) => chats,
            Ok(Err(message)) => return ApiError::invalid(message).response(),
            Err(_) => return ApiError::invalid("import failed").response(),
        };
    if chats.is_empty() {
        return ApiError::invalid("No Claude conversations found in that file.").response();
    }
    match state.store.import_chats(user, provider_id, &chats).await {
        Ok(counts) => Json(import_json(counts)).into_response(),
        Err(StorageError::NotFound) => {
            ApiError::invalid("Add a provider before importing chats.").response()
        }
        Err(err) => ApiError::database(err).response(),
    }
}

