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

include!("routes/router.rs");
include!("routes/health.rs");
include!("routes/models.rs");
include!("routes/chat_completions.rs");
include!("routes/responses.rs");
include!("routes/messages.rs");
include!("routes/count_tokens.rs");
include!("routes/own_system_prompt_body.rs");
include!("routes/get_own_system_prompt.rs");
include!("routes/put_own_system_prompt.rs");
include!("routes/prepend_server_prompts.rs");
include!("routes/list_query.rs");
include!("routes/list_conversations.rs");
include!("routes/create_conversation.rs");
include!("routes/get_conversation.rs");
include!("routes/patch_conversation.rs");
include!("routes/delete_conversation.rs");
include!("routes/import_claude.rs");
include!("routes/import_json.rs");
include!("routes/post_message.rs");
include!("routes/fail_message.rs");
include!("routes/log_meta.rs");
include!("routes/next_event.rs");
include!("routes/finish_encoder.rs");
include!("routes/with_chat_headers.rs");
include!("routes/insert_header.rs");
include!("routes/header_str.rs");
include!("routes/conversation_json.rs");
include!("routes/message_json.rs");
