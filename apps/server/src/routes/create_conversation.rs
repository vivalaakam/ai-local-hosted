async fn create_conversation(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<Value>,
) -> Response {
    let providers = match state.store.list_providers(user).await {
        Ok(providers) => providers,
        Err(err) => return ApiError::database(err).response(),
    };
    let provider_id = match body
        .get("provider_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        Some(id) => match Uuid::parse_str(id) {
            Ok(id) => id,
            Err(_) => return ApiError::invalid("provider id is invalid").response(),
        },
        None => match providers
            .iter()
            .find(|provider| provider.is_default && provider.enabled)
        {
            Some(provider) => provider.id,
            None => return ApiError::invalid("no provider is configured").response(),
        },
    };
    if !providers
        .iter()
        .any(|provider| provider.id == provider_id && provider.enabled)
    {
        return ApiError::not_found(format!("unknown provider {provider_id}")).response();
    }
    let conversation = NewConversation {
        user_id: user,
        id: Uuid::new_v4(),
        title: body
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string(),
        provider_id,
        model: body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string(),
        system_prompt: body
            .get("system_prompt")
            .and_then(Value::as_str)
            .map(str::to_string),
    };
    match state.store.create_conversation(conversation).await {
        Ok(conversation) => (
            StatusCode::CREATED,
            Json(conversation_json(&conversation, None)),
        )
            .into_response(),
        Err(err) => ApiError::database(err).response(),
    }
}

