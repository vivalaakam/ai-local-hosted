use std::convert::Infallible;
use std::time::Instant;

use alh_core::{
    AnthropicEncoder, CompletionAccum, CompletionEvent, CompletionMessage, CompletionStream,
    FinishReason, OpenAiEncoder, ResponsesEncoder, SseFrame, TokenUsage,
};
use axum::response::sse::Event;
use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use alh_storage::{MessageFinal, NewResponseState, RequestLogEntry, Store};

use crate::error::provider_code;

pub struct LogMeta {
    pub user_id: Uuid,
    pub source: &'static str,
    pub protocol: &'static str,
    pub conversation_id: Option<Uuid>,
    pub message_id: Option<Uuid>,
    pub provider_id: Uuid,
    pub client_model: String,
    pub upstream_model: String,
    pub started: Instant,
}

pub fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}

pub fn sse_event(frame: &SseFrame) -> Event {
    let mut event = Event::default().data(frame.data.clone());
    if let Some(name) = &frame.event {
        event = event.event(name);
    }
    event
}

pub async fn forward(tx: &mpsc::Sender<Result<Event, Infallible>>, frames: Vec<SseFrame>) -> bool {
    for frame in frames {
        if tx.send(Ok(sse_event(&frame))).await.is_err() {
            return false;
        }
    }
    true
}

pub struct Outcome {
    pub accum: CompletionAccum,
    pub cancelled: bool,
    pub error_code: Option<String>,
}

pub async fn drive<F>(
    first: Option<CompletionEvent>,
    mut stream: CompletionStream,
    tx: &mpsc::Sender<Result<Event, Infallible>>,
    mut encode: F,
) -> Outcome
where
    F: FnMut(&CompletionEvent) -> Vec<SseFrame>,
{
    let mut accum = CompletionAccum::default();
    let mut cancelled = false;
    let mut error_code = None;
    if let Some(event) = first {
        accum.apply(&event);
        if !forward(tx, encode(&event)).await {
            cancelled = true;
        }
    }
    if !cancelled {
        loop {
            tokio::select! {
                _ = tx.closed() => {
                    cancelled = true;
                    break;
                }
                item = stream.next() => {
                    match item {
                        None => break,
                        Some(Ok(event)) => {
                            accum.apply(&event);
                            if !forward(tx, encode(&event)).await {
                                cancelled = true;
                                break;
                            }
                        }
                        Some(Err(err)) => {
                            error_code = Some(provider_code(&err).to_string());
                            if let Some(detail) = err.detail() {
                                tracing::warn!(error = %err, detail, "provider stream failed");
                            }
                            break;
                        }
                    }
                }
            }
        }
    }
    drop(stream);
    Outcome {
        accum,
        cancelled,
        error_code,
    }
}

type SseSender = mpsc::Sender<Result<Event, Infallible>>;
type SseReceiver = ReceiverStream<Result<Event, Infallible>>;

pub fn channel() -> (SseSender, SseReceiver) {
    let (tx, rx) = mpsc::channel(32);
    (tx, ReceiverStream::new(rx))
}

pub async fn read_until_done(
    mut stream: CompletionStream,
) -> Result<CompletionAccum, alh_core::ProviderError> {
    let mut accum = CompletionAccum::default();
    while let Some(item) = stream.next().await {
        accum.apply(&item?);
    }
    if accum.finish.is_none() {
        accum.finish = Some(if accum.tool_calls.is_empty() {
            FinishReason::Stop
        } else {
            FinishReason::ToolCalls
        });
    }
    Ok(accum)
}

pub fn replay_openai(model: &str, accum: &CompletionAccum) -> Value {
    let mut encoder = OpenAiEncoder::new(new_id("chatcmpl"), model, chrono::Utc::now().timestamp());
    push_accum(&mut |event| encoder.push(event), accum);
    encoder.json()
}

pub fn replay_anthropic(model: &str, accum: &CompletionAccum) -> Value {
    let mut encoder = AnthropicEncoder::new(new_id("msg"), model);
    push_accum(&mut |event| encoder.push(event), accum);
    encoder.json()
}

pub fn replay_responses(id: &str, model: &str, accum: &CompletionAccum) -> Value {
    let mut encoder = ResponsesEncoder::new(id, model);
    push_accum(&mut |event| encoder.push(event), accum);
    encoder.json()
}

fn push_accum(push: &mut dyn FnMut(&CompletionEvent) -> Vec<SseFrame>, accum: &CompletionAccum) {
    if !accum.thinking.is_empty() {
        push(&CompletionEvent::ThinkingDelta(accum.thinking.clone()));
    }
    if !accum.text.is_empty() {
        push(&CompletionEvent::Delta(accum.text.clone()));
    }
    for call in &accum.tool_calls {
        push(&CompletionEvent::ToolCallDelta {
            index: call.index,
            id: Some(call.id.clone()),
            name: Some(call.name.clone()),
            arguments: call.arguments.clone(),
        });
    }
    push(&CompletionEvent::Done {
        finish_reason: accum.finish_reason(),
        usage: accum.usage,
    });
}

pub async fn write_log(
    store: &Store,
    meta: &LogMeta,
    status: &str,
    error_code: Option<&str>,
    usage: Option<TokenUsage>,
) {
    let latency = i32::try_from(meta.started.elapsed().as_millis()).unwrap_or(i32::MAX);
    let entry = RequestLogEntry {
        user_id: meta.user_id,
        source: meta.source.to_string(),
        protocol: meta.protocol.to_string(),
        conversation_id: meta.conversation_id,
        message_id: meta.message_id,
        provider_id: meta.provider_id,
        client_model: meta.client_model.clone(),
        model: meta.upstream_model.clone(),
        status: status.to_string(),
        error_code: error_code.map(str::to_string),
        latency_ms: latency,
        prompt_tokens: usage.and_then(|usage| i32::try_from(usage.prompt_tokens).ok()),
        completion_tokens: usage.and_then(|usage| i32::try_from(usage.completion_tokens).ok()),
    };
    if let Err(err) = store.insert_request_log(&entry).await {
        tracing::error!(%err, "request log insert failed");
    }
}

pub async fn finalize_chat(
    store: &Store,
    id: Uuid,
    status: &str,
    accum: &CompletionAccum,
    error_code: Option<&str>,
) {
    let update = MessageFinal {
        id,
        status: status.to_string(),
        content: accum.text.clone(),
        thinking: if accum.thinking.is_empty() {
            None
        } else {
            Some(accum.thinking.clone())
        },
        tool_calls: accum.tool_calls.clone(),
        error_code: error_code.map(str::to_string),
        usage: accum.usage,
    };
    if let Err(err) = store.finalize_message(update).await {
        tracing::error!(%err, "message finalize failed");
    }
}

pub async fn save_response(
    store: &Store,
    id: &str,
    previous_id: Option<String>,
    meta: &LogMeta,
    transcript: Vec<CompletionMessage>,
    accum: &CompletionAccum,
    output: Value,
) {
    let mut messages = transcript;
    let mut assistant = CompletionMessage::assistant(accum.text.clone());
    if !accum.thinking.is_empty() {
        assistant.thinking = Some(accum.thinking.clone());
    }
    assistant.tool_calls = accum.tool_calls.clone();
    messages.push(assistant);
    let state = NewResponseState {
        user_id: meta.user_id,
        id: id.to_string(),
        previous_id,
        client_model: meta.client_model.clone(),
        provider_id: meta.provider_id,
        upstream_model: meta.upstream_model.clone(),
        messages,
        output,
    };
    if let Err(err) = store.save_response_state(&state).await {
        tracing::error!(%err, "response state insert failed");
    }
}

pub fn status_of(outcome: &Outcome) -> &'static str {
    if outcome.cancelled {
        "cancelled"
    } else if outcome.error_code.is_some() {
        "error"
    } else {
        "done"
    }
}
