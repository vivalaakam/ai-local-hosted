use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use alh_core::{
    ChatProvider, CompletionEvent, CompletionMessage, CompletionRequest, FinishReason,
    ProviderErrorKind,
};
use alh_providers::{OpenAiProvider, ProviderConfig};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::response::sse::{Event, Sse};
use axum::routing::post;
use axum::{Json, Router};
use futures::StreamExt;
use serde_json::{Value, json};
use tokio::net::TcpListener;

struct Stub {
    base: String,
    cancelled: Arc<AtomicBool>,
}

async fn stub() -> Stub {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let app = Router::new()
        .route("/v1/chat/completions", post(completions))
        .with_state(flag);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    Stub {
        base: format!("http://{addr}/v1"),
        cancelled,
    }
}

fn provider(base: &str) -> OpenAiProvider {
    OpenAiProvider::new(ProviderConfig {
        id: "stub".into(),
        base_url: base.into(),
        api_key: None,
        header_timeout: Duration::from_secs(2),
        idle_timeout: Duration::from_secs(2),
    })
}

fn request(model: &str) -> CompletionRequest {
    CompletionRequest {
        model: model.into(),
        messages: vec![CompletionMessage::user("hi")],
        tools: Vec::new(),
        tool_choice: None,
        temperature: None,
        top_p: None,
        max_tokens: None,
        stop: Vec::new(),
    }
}

async fn collect(base: &str, model: &str) -> Vec<CompletionEvent> {
    let mut stream = provider(base).stream(request(model));
    let mut events = Vec::new();
    while let Some(item) = stream.next().await {
        events.push(item.expect("stream event"));
    }
    events
}

#[tokio::test]
async fn plain_text_two_deltas_and_tool_call() {
    let stub = stub().await;

    let plain = collect(&stub.base, "plain").await;
    assert!(matches!(plain[0], CompletionEvent::Delta(ref text) if text == "hello"));
    match plain.last() {
        Some(CompletionEvent::Done {
            finish_reason,
            usage,
        }) => {
            assert_eq!(finish_reason, &FinishReason::Stop);
            assert_eq!(usage.unwrap().prompt_tokens, 2);
        }
        other => panic!("expected done, got {other:?}"),
    }

    let two = collect(&stub.base, "two").await;
    assert!(matches!(two[0], CompletionEvent::Delta(ref text) if text == "ab"));
    assert!(matches!(two[1], CompletionEvent::Delta(ref text) if text == "cd"));

    let tool = collect(&stub.base, "tool").await;
    match &tool[0] {
        CompletionEvent::ToolCallDelta {
            index,
            id,
            name,
            arguments,
        } => {
            assert_eq!(*index, 0);
            assert_eq!(id.as_deref(), Some("call_1"));
            assert_eq!(name.as_deref(), Some("echo"));
            assert_eq!(arguments, "");
        }
        other => panic!("expected tool delta, got {other:?}"),
    }
    match &tool[1] {
        CompletionEvent::ToolCallDelta { arguments, .. } => assert_eq!(arguments, "{}"),
        other => panic!("expected arguments, got {other:?}"),
    }
    match tool.last() {
        Some(CompletionEvent::Done { finish_reason, .. }) => {
            assert_eq!(finish_reason, &FinishReason::ToolCalls);
        }
        other => panic!("expected tool done, got {other:?}"),
    }
}

#[tokio::test]
async fn upstream_404_and_broken_sse() {
    let stub = stub().await;
    let missing = provider(&stub.base).stream(request("missing"));
    let err = missing
        .into_future()
        .await
        .0
        .expect("error event")
        .expect_err("404");
    assert_eq!(err.kind, ProviderErrorKind::NotFound);

    let broken = provider(&stub.base).stream(request("broken"));
    let err = broken
        .into_future()
        .await
        .0
        .expect("error event")
        .expect_err("decode");
    assert_eq!(err.kind, ProviderErrorKind::Decode);
}

#[tokio::test]
async fn dropping_the_reader_cancels_the_upstream_request() {
    let stub = stub().await;
    let mut stream = provider(&stub.base).stream(request("hang"));
    let first = stream.next().await.expect("delta").unwrap();
    assert!(matches!(first, CompletionEvent::Delta(ref text) if text == "partial"));
    drop(stream);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !stub.cancelled.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        stub.cancelled.load(Ordering::SeqCst),
        "upstream handler was not dropped"
    );
}

async fn completions(
    State(cancelled): State<Arc<AtomicBool>>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    assert_eq!(
        body["stream"],
        json!(true),
        "upstream must be asked to stream"
    );
    let model = body["model"].as_str().unwrap_or("");
    match model {
        "missing" => (axum::http::StatusCode::NOT_FOUND, "missing").into_response(),
        "broken" => Sse::new(futures::stream::iter([Ok::<_, Infallible>(
            Event::default().data("{"),
        )]))
        .into_response(),
        "hang" => hang(cancelled).into_response(),
        "two" => sse(vec![
            chunk(json!({"content": "ab"}), None, None),
            chunk(json!({"content": "cd"}), Some("stop"), None),
            Event::default().data("[DONE]"),
        ])
        .into_response(),
        "tool" => sse(vec![
            chunk(
                json!({"tool_calls": [{
                    "index": 0,
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": "echo", "arguments": ""}
                }]}),
                None,
                None,
            ),
            chunk(
                json!({"tool_calls": [{
                    "index": 0,
                    "function": {"arguments": "{}"}
                }]}),
                Some("tool_calls"),
                None,
            ),
            chunk(
                json!({}),
                None,
                Some(json!({"prompt_tokens": 3, "completion_tokens": 1})),
            ),
            Event::default().data("[DONE]"),
        ])
        .into_response(),
        _ => sse(vec![
            chunk(json!({"content": "hello"}), None, None),
            chunk(
                json!({}),
                Some("stop"),
                Some(json!({"prompt_tokens": 2, "completion_tokens": 1})),
            ),
            Event::default().data("[DONE]"),
        ])
        .into_response(),
    }
}

fn sse(events: Vec<Event>) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    Sse::new(futures::stream::iter(events.into_iter().map(Ok)))
}

fn chunk(delta: Value, finish: Option<&str>, usage: Option<Value>) -> Event {
    let mut body = json!({
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
    });
    if let Some(usage) = usage {
        body["usage"] = usage;
    }
    Event::default().data(body.to_string())
}

fn hang(flag: Arc<AtomicBool>) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    struct Guard(Arc<AtomicBool>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let guard = Guard(flag);
    Sse::new(futures::stream::unfold((guard, 0u8), |state| async move {
        let (guard, step) = state;
        if step == 0 {
            let event = chunk(json!({"content": "partial"}), None, None);
            Some((Ok(event), (guard, 1)))
        } else {
            tokio::time::sleep(Duration::from_secs(60)).await;
            None
        }
    }))
}
