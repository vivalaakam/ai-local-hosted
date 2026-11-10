async fn post_message(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
    Json(body): Json<Value>,
) -> Response {
    let content = match body
        .get("content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        Some(content) => content.to_string(),
        None => return ApiError::invalid("content is required").response(),
    };
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string);
    let begun = match state
        .store
        .begin_turn(user, id, &content, model.as_deref())
        .await
    {
        Ok(begun) => begun,
        Err(StorageError::NotFound) => {
            return ApiError::not_found("conversation not found").response();
        }
        Err(StorageError::Conflict) => return ApiError::conflict().response(),
        Err(err) => return ApiError::database(err).response(),
    };
    if begun.model.trim().is_empty() {
        return fail_message(
            &state,
            begun.assistant_message_id,
            "invalid_request",
            ApiError::invalid("model is required"),
        )
        .await;
    }
    let route = match resolve_route(&state, user, &begun.model).await {
        Ok(route) => route,
        Err(err) => {
            let code = err.code;
            return fail_message(&state, begun.assistant_message_id, code, err).await;
        }
    };
    if let Err(err) = state
        .store
        .set_conversation_route(user, id, route.provider_id, &route.client_model)
        .await
    {
        tracing::warn!(%err, "conversation route update failed");
    }
    let history = select_context(begun.system_prompt.as_deref(), &begun.history, 40);
    let mut request = CompletionRequest {
        model: route.upstream_model.clone(),
        messages: history,
        tools: Vec::new(),
        tool_choice: None,
        temperature: None,
        top_p: None,
        max_tokens: None,
        stop: Vec::new(),
    };
    if let Err(err) = prepend_server_prompts(&state, user, &mut request).await {
        return fail_message(&state, begun.assistant_message_id, err.code, err).await;
    }
    let meta = log_meta(
        user,
        "chat",
        "chat",
        &route,
        Some(begun.conversation_id),
        Some(begun.assistant_message_id),
    );
    let mut stream = route.provider.stream(request);
    let first = match next_event(&mut stream).await {
        Ok(first) => first,
        Err(err) => {
            let code = provider_code(&err);
            write_log(&state.store, &meta, "error", Some(code), None).await;
            return fail_message(
                &state,
                begun.assistant_message_id,
                code,
                ApiError::from_provider(err),
            )
            .await;
        }
    };
    let (tx, rx) = channel();
    let store = state.store.clone();
    let model = route.client_model.clone();
    let assistant_id = begun.assistant_message_id;
    tokio::spawn(async move {
        let mut encoder =
            OpenAiEncoder::new(new_id("chatcmpl"), model, chrono::Utc::now().timestamp());
        let outcome = drive(first, stream, &tx, |event| encoder.push(event)).await;
        finish_encoder(&tx, &outcome, |event| encoder.push(event)).await;
        let status = status_of(&outcome);
        finalize_chat(
            &store,
            assistant_id,
            status,
            &outcome.accum,
            outcome.error_code.as_deref(),
        )
        .await;
        write_log(
            &store,
            &meta,
            status,
            outcome.error_code.as_deref(),
            outcome.accum.usage,
        )
        .await;
    });
    with_chat_headers(
        Sse::new(rx).into_response(),
        begun.conversation_id,
        begun.user_message_id,
        begun.assistant_message_id,
    )
}

