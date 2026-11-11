async fn completions(
    State(stub): State<Arc<Record>>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    if body["stream"] != true {
        return (axum::http::StatusCode::BAD_REQUEST, "stream required").into_response();
    }
    let model = body["model"].as_str().unwrap_or("").to_string();
    let transcript = body["messages"]
        .as_array()
        .map(|messages| {
            messages
                .iter()
                .map(|message| {
                    format!(
                        "{}:{}",
                        message["role"].as_str().unwrap_or(""),
                        message["content"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    *stub.last_model.lock().unwrap() = model.clone();
    *stub.last_messages.lock().unwrap() = transcript;
    if model == "missing-upstream" {
        return (axum::http::StatusCode::NOT_FOUND, "missing").into_response();
    }
    let messages = body["messages"].as_array();
    if let Some(messages) = messages
        && let Some(tool) = messages.iter().rev().find(|message| message["role"] == "tool")
    {
        let content = tool["content"].as_str().unwrap_or("");
        return text_response(&format!("tool-result:{content}"));
    }
    let content = messages
        .and_then(|messages| messages.last())
        .and_then(|message| message["content"].as_str())
        .unwrap_or("");
    match content {
        "SLOW" => hang(),
        "TOOL" => sse(vec![
            chunk(
                json!({"tool_calls": [{
                    "index": 0,
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": "echo", "arguments": "{}"}
                }]}),
                Some("tool_calls"),
            ),
            Event::default().data("[DONE]"),
        ]),
        "USE_TIME" => sse(vec![
            chunk(
                json!({"tool_calls": [{
                    "index": 0,
                    "id": "call_time",
                    "type": "function",
                    "function": {"name": "current_time", "arguments": "{}"}
                }]}),
                Some("tool_calls"),
            ),
            Event::default().data("[DONE]"),
        ]),
        "TOOLS" => {
            let name = body["tools"][0]["function"]["name"].as_str().unwrap_or("");
            text_response(&format!("tools:{name}"))
        }
        _ => text_response("ok"),
    }
}

fn text_response(text: &str) -> axum::response::Response {
    sse(vec![
        chunk(json!({"content": text}), None),
        chunk(json!({}), Some("stop")),
        Event::default().data("[DONE]"),
    ])
}

fn sse(events: Vec<Event>) -> axum::response::Response {
    Sse::new(futures::stream::iter(
        events.into_iter().map(Ok::<_, std::convert::Infallible>),
    ))
    .into_response()
}

fn hang() -> axum::response::Response {
    Sse::new(futures::stream::unfold(0u8, |step| async move {
        if step == 0 {
            Some((
                Ok::<_, std::convert::Infallible>(chunk(json!({"content": "partial"}), None)),
                1,
            ))
        } else {
            tokio::time::sleep(Duration::from_secs(30)).await;
            None
        }
    }))
    .into_response()
}

fn chunk(delta: Value, finish: Option<&str>) -> Event {
    Event::default().data(
        json!({
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
        })
        .to_string(),
    )
}
