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
    let tool_rows = match state.store.list_enabled_tools_for_user(user).await {
        Ok(rows) => rows,
        Err(err) => return ApiError::database(err).response(),
    };
    let mut tool_specs = Vec::new();
    for row in tool_rows {
        match crate::tool_catalog::row_to_spec(row) {
            Ok(spec) => tool_specs.push(spec),
            Err(err) => tracing::warn!(%err, "skipping invalid tool row"),
        }
    }
    let tools_prompt = alh_tools::catalog_prompt(&tool_specs);
    let begun = match state
        .store
        .begin_turn(
            user,
            id,
            &content,
            model.as_deref(),
            tools_prompt.as_deref(),
        )
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
    let tool_defs = alh_tools::definitions_for(&tool_specs);
    let history = select_context(begun.system_prompt.as_deref(), &begun.history, 40);
    let mut seed = CompletionRequest {
        model: route.upstream_model.clone(),
        messages: history,
        tools: tool_defs,
        tool_choice: None,
        temperature: None,
        top_p: None,
        max_tokens: None,
        stop: Vec::new(),
    };
    if let Err(err) = prepend_server_prompts(&state, user, &mut seed).await {
        return fail_message(&state, begun.assistant_message_id, err.code, err).await;
    }
    if let Some(prompt) = tools_prompt.as_deref() {
        let index = seed
            .messages
            .iter()
            .position(|message| message.role != alh_core::Role::System)
            .unwrap_or(seed.messages.len());
        seed.messages
            .insert(index, CompletionMessage::system(prompt.to_string()));
    }
    let meta = log_meta(
        user,
        "chat",
        "chat",
        &route,
        Some(begun.conversation_id),
        Some(begun.assistant_message_id),
    );
    let (tx, rx) = channel();
    let store = state.store.clone();
    let tools_runtime = state.tools.clone();
    let client_model = route.client_model.clone();
    let upstream_model = route.upstream_model.clone();
    let provider = route.provider;
    let conversation_id = begun.conversation_id;
    let initial_assistant_id = begun.assistant_message_id;
    tokio::spawn(async move {
        drive_chat_with_tools(DriveChat {
            store,
            tools_runtime,
            tool_specs,
            provider,
            tx,
            meta,
            client_model,
            upstream_model,
            conversation_id,
            assistant_id: initial_assistant_id,
            messages: seed.messages,
            tools: seed.tools,
        })
        .await;
    });
    with_chat_headers(
        Sse::new(rx).into_response(),
        begun.conversation_id,
        begun.user_message_id,
        begun.assistant_message_id,
        begun.system_message_id,
    )
}

struct DriveChat {
    store: alh_storage::Store,
    tools_runtime: alh_tools::ToolExecutor,
    tool_specs: Vec<alh_tools::ToolSpec>,
    provider: alh_providers::OpenAiProvider,
    tx: tokio::sync::mpsc::Sender<Result<axum::response::sse::Event, std::convert::Infallible>>,
    meta: crate::turn::LogMeta,
    client_model: String,
    upstream_model: String,
    conversation_id: Uuid,
    assistant_id: Uuid,
    messages: Vec<CompletionMessage>,
    tools: Vec<alh_core::ToolDefinition>,
}

async fn drive_chat_with_tools(mut ctx: DriveChat) {
    const MAX_TOOL_ROUNDS: usize = 8;
    let mut assistant_id = ctx.assistant_id;
    let mut final_accum = alh_core::CompletionAccum::default();
    let mut cancelled = false;
    let mut error_code: Option<String> = None;

    for round in 0..=MAX_TOOL_ROUNDS {
        let request = CompletionRequest {
            model: ctx.upstream_model.clone(),
            messages: ctx.messages.clone(),
            tools: ctx.tools.clone(),
            tool_choice: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            stop: Vec::new(),
        };
        let mut stream = ctx.provider.stream(request);
        let first = match next_event(&mut stream).await {
            Ok(first) => first,
            Err(err) => {
                error_code = Some(provider_code(&err).to_string());
                break;
            }
        };
        let mut encoder = OpenAiEncoder::new(
            new_id("chatcmpl"),
            &ctx.client_model,
            chrono::Utc::now().timestamp(),
        );
        // Hold Done until the chat tool loop finishes so the client sees one [DONE].
        let outcome = drive(first, stream, &ctx.tx, |event| {
            if matches!(event, alh_core::CompletionEvent::Done { .. }) {
                Vec::new()
            } else {
                encoder.push(event)
            }
        })
        .await;
        cancelled = outcome.cancelled;
        if let Some(code) = outcome.error_code.clone() {
            error_code = Some(code);
        }
        final_accum = outcome.accum.clone();

        if cancelled || error_code.is_some() {
            break;
        }

        let wants_tools = matches!(
            final_accum.finish_reason(),
            alh_core::FinishReason::ToolCalls
        ) && !final_accum.tool_calls.is_empty();
        if !wants_tools || round == MAX_TOOL_ROUNDS {
            let finish = if wants_tools {
                alh_core::FinishReason::Stop
            } else {
                final_accum.finish_reason()
            };
            let _ = forward(
                &ctx.tx,
                encoder.push(&alh_core::CompletionEvent::Done {
                    finish_reason: finish,
                    usage: final_accum.usage,
                }),
            )
            .await;
            break;
        }

        let mut assistant = CompletionMessage::assistant(final_accum.text.clone());
        if !final_accum.thinking.is_empty() {
            assistant.thinking = Some(final_accum.thinking.clone());
        }
        assistant.tool_calls = final_accum.tool_calls.clone();
        ctx.messages.push(assistant);

        let executed = ctx
            .tools_runtime
            .execute_calls(&ctx.tool_specs, &final_accum.tool_calls)
            .await;
        let mut tool_rows = Vec::new();
        for (call, result) in executed {
            let content = match result {
                Ok(content) => content,
                Err(err) => json!({"error": err.to_string()}).to_string(),
            };
            ctx.messages.push(CompletionMessage {
                role: alh_core::Role::Tool,
                content: content.clone(),
                parts: None,
                tool_calls: Vec::new(),
                tool_call_id: Some(call.id.clone()),
                thinking: None,
            });
            tool_rows.push(alh_storage::ToolResultMessage {
                tool_call_id: call.id,
                content,
            });
        }

        let update = alh_storage::MessageFinal {
            id: assistant_id,
            status: "done".into(),
            content: final_accum.text.clone(),
            thinking: if final_accum.thinking.is_empty() {
                None
            } else {
                Some(final_accum.thinking.clone())
            },
            tool_calls: final_accum.tool_calls.clone(),
            error_code: None,
            usage: final_accum.usage,
        };
        match ctx
            .store
            .append_tool_round(ctx.conversation_id, assistant_id, &update, &tool_rows)
            .await
        {
            Ok(next_id) => assistant_id = next_id,
            Err(err) => {
                tracing::error!(%err, "append tool round failed");
                error_code = Some("upstream".into());
                break;
            }
        }
    }

    let status = if cancelled {
        "cancelled"
    } else if error_code.is_some() {
        "error"
    } else {
        "done"
    };
    finalize_chat(
        &ctx.store,
        assistant_id,
        status,
        &final_accum,
        error_code.as_deref(),
    )
    .await;
    let mut meta = ctx.meta;
    meta.message_id = Some(assistant_id);
    write_log(
        &ctx.store,
        &meta,
        status,
        error_code.as_deref(),
        final_accum.usage,
    )
    .await;
}
