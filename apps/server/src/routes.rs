use std::sync::Arc;
use std::time::Instant;

use alh_core::{
    AnthropicEncoder, ChatProvider, CompletionMessage, CompletionRequest, OpenAiEncoder,
    ResponsesEncoder, estimate_request_tokens, parse_anthropic_message, parse_chat_completion,
    parse_responses, select_context,
};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Extension, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::Sse;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Router, middleware};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use alh_storage::{ConversationPatch, ImportCounts, NewConversation, StorageError, parse_cursor};

use crate::auth::{CurrentUser, require_auth};
use crate::error::{ApiError, provider_code};
use crate::state::{AppState, apply_provider_header, model_ids, resolve_route};
use crate::turn::{
    LogMeta, channel, drive, finalize_chat, forward, new_id, read_until_done, replay_anthropic,
    replay_openai, replay_responses, save_response, status_of, write_log,
};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/oauth/register", post(crate::oauth::register))
        .route("/oauth/token", post(crate::oauth::token))
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/responses", post(responses))
        .route("/v1/messages", post(messages))
        .route("/v1/messages/count_tokens", post(count_tokens))
        .route(
            "/api/system-prompt",
            get(get_own_system_prompt).put(put_own_system_prompt),
        )
        .route(
            "/api/conversations",
            get(list_conversations).post(create_conversation),
        )
        .route(
            "/api/conversations/{id}",
            get(get_conversation)
                .patch(patch_conversation)
                .delete(delete_conversation),
        )
        .route("/api/conversations/{id}/messages", post(post_message))
        .route(
            "/api/imports/claude",
            post(import_claude).layer(DefaultBodyLimit::max(512 * 1024 * 1024)),
        )
        .merge(crate::providers::routes())
        .layer(middleware::from_fn_with_state(state.clone(), require_auth))
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(state)
}

async fn health(State(state): State<Arc<AppState>>) -> Response {
    let database_ok = state.store.ping().await.is_ok();
    let status = if database_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(json!({
            "status": if database_ok { "ok" } else { "down" },
            "database": if database_ok { "ok" } else { "down" },
            "provider": "unknown",
        })),
    )
        .into_response()
}

async fn models(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    headers: HeaderMap,
) -> Response {
    let ids = match model_ids(&state, user).await {
        Ok(ids) => ids,
        Err(err) => return err.response(),
    };
    if headers.get("anthropic-version").is_some() {
        Json(json!({
            "data": ids.iter().map(|id| json!({
                "type": "model",
                "id": id,
                "display_name": id,
                "created_at": "2026-01-01T00:00:00Z",
            })).collect::<Vec<_>>(),
            "has_more": false,
        }))
        .into_response()
    } else {
        Json(json!({
            "object": "list",
            "data": ids.iter().map(|id| json!({
                "id": id,
                "object": "model",
                "owned_by": "alh",
            })).collect::<Vec<_>>(),
        }))
        .into_response()
    }
}

async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let parsed = match parse_chat_completion(&body) {
        Ok(parsed) => parsed,
        Err(err) => return ApiError::invalid(err.to_string()).response(),
    };
    let requested = apply_provider_header(&parsed.client_model, header_str(&headers, "x-provider"));
    let route = match resolve_route(&state, user, &requested).await {
        Ok(route) => route,
        Err(err) => return err.response(),
    };
    let mut request = parsed.request;
    if let Err(err) = prepend_server_prompts(&state, user, &mut request).await {
        return err.response();
    }
    request.model = route.upstream_model.clone();
    let meta = log_meta(user, "agent", "chat_completions", &route, None, None);
    let mut stream = route.provider.stream(request);
    if !parsed.stream {
        return match read_until_done(stream).await {
            Ok(accum) => {
                let body = replay_openai(&route.client_model, &accum);
                write_log(&state.store, &meta, "done", None, accum.usage).await;
                Json(body).into_response()
            }
            Err(err) => {
                write_log(
                    &state.store,
                    &meta,
                    "error",
                    Some(provider_code(&err)),
                    None,
                )
                .await;
                ApiError::from_provider(err).response()
            }
        };
    }
    let first = match next_event(&mut stream).await {
        Ok(first) => first,
        Err(err) => {
            write_log(
                &state.store,
                &meta,
                "error",
                Some(provider_code(&err)),
                None,
            )
            .await;
            return ApiError::from_provider(err).response();
        }
    };
    let (tx, rx) = channel();
    let store = state.store.clone();
    let model = route.client_model.clone();
    tokio::spawn(async move {
        let mut encoder =
            OpenAiEncoder::new(new_id("chatcmpl"), model, chrono::Utc::now().timestamp());
        let outcome = drive(first, stream, &tx, |event| encoder.push(event)).await;
        finish_encoder(&tx, &outcome, |event| encoder.push(event)).await;
        let status = status_of(&outcome);
        write_log(
            &store,
            &meta,
            status,
            outcome.error_code.as_deref(),
            outcome.accum.usage,
        )
        .await;
    });
    Sse::new(rx).into_response()
}

async fn responses(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let previous_id = body
        .get("previous_response_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let previous = if let Some(id) = &previous_id {
        match state.store.get_response_state(user, id).await {
            Ok(Some(stored)) => Some(stored.messages),
            Ok(None) => return ApiError::not_found("unknown previous_response_id").response(),
            Err(err) => return ApiError::database(err).response(),
        }
    } else {
        None
    };
    let parsed = match parse_responses(&body, previous.as_deref()) {
        Ok(parsed) => parsed,
        Err(err) => return ApiError::invalid(err.to_string()).response(),
    };
    let requested = apply_provider_header(
        &parsed.incoming.client_model,
        header_str(&headers, "x-provider"),
    );
    let route = match resolve_route(&state, user, &requested).await {
        Ok(route) => route,
        Err(err) => return err.response(),
    };
    let mut request = parsed.incoming.request;
    let transcript = request.messages.clone();
    if let Err(err) = prepend_server_prompts(&state, user, &mut request).await {
        return err.response();
    }
    request.model = route.upstream_model.clone();
    let response_id = new_id("resp");
    let meta = log_meta(user, "agent", "responses", &route, None, None);
    let mut stream = route.provider.stream(request);
    if !parsed.incoming.stream {
        return match read_until_done(stream).await {
            Ok(accum) => {
                let body = replay_responses(&response_id, &route.client_model, &accum);
                save_response(
                    &state.store,
                    &response_id,
                    previous_id,
                    &meta,
                    transcript,
                    &accum,
                    body.get("output").cloned().unwrap_or(Value::Null),
                )
                .await;
                write_log(&state.store, &meta, "done", None, accum.usage).await;
                Json(body).into_response()
            }
            Err(err) => {
                write_log(
                    &state.store,
                    &meta,
                    "error",
                    Some(provider_code(&err)),
                    None,
                )
                .await;
                ApiError::from_provider(err).response()
            }
        };
    }
    let first = match next_event(&mut stream).await {
        Ok(first) => first,
        Err(err) => {
            write_log(
                &state.store,
                &meta,
                "error",
                Some(provider_code(&err)),
                None,
            )
            .await;
            return ApiError::from_provider(err).response();
        }
    };
    let (tx, rx) = channel();
    let store = state.store.clone();
    let model = route.client_model.clone();
    let id = response_id.clone();
    tokio::spawn(async move {
        let mut encoder = ResponsesEncoder::new(id.clone(), model);
        if !forward(&tx, vec![encoder.created_frame()]).await {
            save_response(
                &store,
                &id,
                previous_id,
                &meta,
                transcript,
                &alh_core::CompletionAccum::default(),
                Value::Null,
            )
            .await;
            write_log(&store, &meta, "cancelled", None, None).await;
            return;
        }
        let outcome = drive(first, stream, &tx, |event| encoder.push(event)).await;
        finish_encoder(&tx, &outcome, |event| encoder.push(event)).await;
        let output = encoder.output_items();
        let status = status_of(&outcome);
        save_response(
            &store,
            &id,
            previous_id,
            &meta,
            transcript,
            &outcome.accum,
            output,
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
    Sse::new(rx).into_response()
}

async fn messages(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let parsed = match parse_anthropic_message(&body) {
        Ok(parsed) => parsed,
        Err(err) => return ApiError::invalid(err.to_string()).anthropic().response(),
    };
    let requested = apply_provider_header(&parsed.client_model, header_str(&headers, "x-provider"));
    let route = match resolve_route(&state, user, &requested).await {
        Ok(route) => route,
        Err(err) => return err.anthropic().response(),
    };
    let mut request = parsed.request;
    if let Err(err) = prepend_server_prompts(&state, user, &mut request).await {
        return err.anthropic().response();
    }
    request.model = route.upstream_model.clone();
    let meta = log_meta(user, "agent", "anthropic_messages", &route, None, None);
    let mut stream = route.provider.stream(request);
    if !parsed.stream {
        return match read_until_done(stream).await {
            Ok(accum) => {
                let body = replay_anthropic(&route.client_model, &accum);
                write_log(&state.store, &meta, "done", None, accum.usage).await;
                Json(body).into_response()
            }
            Err(err) => {
                write_log(
                    &state.store,
                    &meta,
                    "error",
                    Some(provider_code(&err)),
                    None,
                )
                .await;
                ApiError::from_provider(err).anthropic().response()
            }
        };
    }
    let first = match next_event(&mut stream).await {
        Ok(first) => first,
        Err(err) => {
            write_log(
                &state.store,
                &meta,
                "error",
                Some(provider_code(&err)),
                None,
            )
            .await;
            return ApiError::from_provider(err).anthropic().response();
        }
    };
    let (tx, rx) = channel();
    let store = state.store.clone();
    let model = route.client_model.clone();
    tokio::spawn(async move {
        let mut encoder = AnthropicEncoder::new(new_id("msg"), model);
        let outcome = drive(first, stream, &tx, |event| encoder.push(event)).await;
        finish_encoder(&tx, &outcome, |event| encoder.push(event)).await;
        let status = status_of(&outcome);
        write_log(
            &store,
            &meta,
            status,
            outcome.error_code.as_deref(),
            outcome.accum.usage,
        )
        .await;
    });
    Sse::new(rx).into_response()
}

async fn count_tokens(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<Value>,
) -> Response {
    match parse_anthropic_message(&body) {
        Ok(mut parsed) => match prepend_server_prompts(&state, user, &mut parsed.request).await {
            Ok(()) => Json(json!({
                "input_tokens": estimate_request_tokens(&parsed.request),
            }))
            .into_response(),
            Err(err) => err.anthropic().response(),
        },
        Err(err) => ApiError::invalid(err.to_string()).anthropic().response(),
    }
}

#[derive(Deserialize)]
struct OwnSystemPromptBody {
    content: Option<String>,
}

async fn get_own_system_prompt(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> Response {
    match state.store.system_prompt_layers(user).await {
        Ok(layers) => Json(json!({ "content": layers.own })).into_response(),
        Err(err) => ApiError::database(err).response(),
    }
}

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

#[derive(Deserialize)]
struct ListQuery {
    cursor: Option<String>,
    limit: Option<i64>,
}

async fn list_conversations(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(query): Query<ListQuery>,
) -> Response {
    let cursor = match query.cursor.as_deref() {
        None => None,
        Some(cursor) => match parse_cursor(cursor) {
            Some(cursor) => Some(cursor),
            None => return ApiError::invalid("invalid cursor").response(),
        },
    };
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    match state.store.list_conversations(user, cursor, limit).await {
        Ok(page) => Json(json!({
            "data": page.items.iter().map(|item| conversation_json(item, None)).collect::<Vec<_>>(),
            "next_cursor": page.next_cursor,
        }))
        .into_response(),
        Err(err) => ApiError::database(err).response(),
    }
}

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

fn import_json(counts: ImportCounts) -> Value {
    json!({
        "conversations": counts.conversations,
        "skipped": counts.skipped,
        "messages": counts.messages,
    })
}

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

async fn fail_message(state: &AppState, message_id: Uuid, code: &str, error: ApiError) -> Response {
    finalize_chat(
        &state.store,
        message_id,
        "error",
        &alh_core::CompletionAccum::default(),
        Some(code),
    )
    .await;
    error.response()
}

fn log_meta(
    user_id: Uuid,
    source: &'static str,
    protocol: &'static str,
    route: &crate::state::ResolvedRoute,
    conversation_id: Option<Uuid>,
    message_id: Option<Uuid>,
) -> LogMeta {
    LogMeta {
        user_id,
        source,
        protocol,
        conversation_id,
        message_id,
        provider_id: route.provider_id,
        client_model: route.client_model.clone(),
        upstream_model: route.upstream_model.clone(),
        started: Instant::now(),
    }
}

async fn next_event(
    stream: &mut alh_core::CompletionStream,
) -> Result<Option<alh_core::CompletionEvent>, alh_core::ProviderError> {
    match stream.next().await {
        None => Ok(None),
        Some(result) => result.map(Some),
    }
}

async fn finish_encoder<F>(
    tx: &tokio::sync::mpsc::Sender<Result<axum::response::sse::Event, std::convert::Infallible>>,
    outcome: &crate::turn::Outcome,
    mut encode: F,
) where
    F: FnMut(&alh_core::CompletionEvent) -> Vec<alh_core::SseFrame>,
{
    if outcome.cancelled || outcome.error_code.is_some() || outcome.accum.finish.is_some() {
        return;
    }
    let _ = forward(
        tx,
        encode(&alh_core::CompletionEvent::Done {
            finish_reason: alh_core::FinishReason::Stop,
            usage: outcome.accum.usage,
        }),
    )
    .await;
}

fn with_chat_headers(
    mut response: Response,
    conversation: Uuid,
    user_message: Uuid,
    message: Uuid,
) -> Response {
    insert_header(
        &mut response,
        "x-conversation-id",
        &conversation.to_string(),
    );
    insert_header(
        &mut response,
        "x-user-message-id",
        &user_message.to_string(),
    );
    insert_header(&mut response, "x-message-id", &message.to_string());
    response
}

fn insert_header(response: &mut Response, name: &'static str, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        response
            .headers_mut()
            .insert(axum::http::header::HeaderName::from_static(name), value);
    }
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn conversation_json(
    conversation: &alh_storage::Conversation,
    messages: Option<&[alh_storage::MessageRow]>,
) -> Value {
    let mut value = json!({
        "id": conversation.id,
        "title": conversation.title,
        "provider_id": conversation.provider_id,
        "model": conversation.model,
        "system_prompt": conversation.system_prompt,
        "created_at": conversation.created_at.to_rfc3339(),
        "updated_at": conversation.updated_at.to_rfc3339(),
        "archived_at": conversation.archived_at.map(|value| value.to_rfc3339()),
    });
    if let Some(messages) = messages {
        value["messages"] = json!(messages.iter().map(message_json).collect::<Vec<_>>());
    }
    value
}

fn message_json(message: &alh_storage::MessageRow) -> Value {
    json!({
        "id": message.id,
        "position": message.position,
        "role": message.role,
        "content": message.content,
        "tool_calls": message.tool_calls,
        "tool_call_id": message.tool_call_id,
        "thinking": message.thinking,
        "status": message.status,
        "error_code": message.error_code,
    })
}
