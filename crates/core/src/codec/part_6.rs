fn openai_tool_call(call: &ToolCall) -> Value {
    json!({
        "id": call.id,
        "type": "function",
        "function": {"name": call.name, "arguments": call.arguments},
    })
}

fn usage_json(usage: crate::message::TokenUsage) -> Value {
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.prompt_tokens.saturating_add(usage.completion_tokens),
    })
}

fn number(body: &Value, field: &str) -> Option<f32> {
    body.get(field)
        .and_then(Value::as_f64)
        .map(|value| value as f32)
}

fn u32_field(body: &Value, field: &str) -> Option<u32> {
    body.get(field)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn stop_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(text)) => vec![text.clone()],
        Some(Value::Array(_)) => string_array(value),
        _ => Vec::new(),
    }
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn json_string(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn parse_arguments(arguments: &str) -> Value {
    serde_json::from_str(arguments).unwrap_or(Value::String(arguments.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{FinishReason, TokenUsage};

    #[test]
    fn chat_completion_keeps_tool_calls() {
        let body = json!({
            "model": "qwen",
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "", "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": "read", "arguments": "{\"path\":\"a\"}"}
                }]},
                {"role": "tool", "tool_call_id": "call_1", "content": "ok"}
            ],
            "tools": [{"type": "function", "function": {"name": "read", "parameters": {"type": "object"}}}],
            "stream": true
        });
        let incoming = parse_chat_completion(&body).unwrap();
        assert!(incoming.stream);
        assert_eq!(incoming.request.messages[1].tool_calls[0].name, "read");
        assert_eq!(
            incoming.request.messages[2].tool_call_id.as_deref(),
            Some("call_1")
        );
        assert_eq!(incoming.request.tools[0].name, "read");
    }

    #[test]
    fn anthropic_tool_use_roundtrip() {
        let body = json!({
            "model": "claude-sonnet-4-5",
            "system": [{"type": "text", "text": "be brief"}],
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "run"},
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": "done"}
                ]
            }],
            "tools": [{"name": "run", "input_schema": {"type": "object"}}]
        });
        let incoming = parse_anthropic_message(&body).unwrap();
        assert_eq!(incoming.request.messages[0].content, "be brief");
        assert_eq!(incoming.request.messages[2].role, Role::Tool);
        let mut encoder = AnthropicEncoder::new("msg_1", "claude-sonnet-4-5");
        let frames = encoder.push(&CompletionEvent::ToolCallDelta {
            index: 0,
            id: Some("toolu_2".into()),
            name: Some("run".into()),
            arguments: "{\"a\":1}".into(),
        });
        assert!(frames.iter().any(
            |frame| frame.event.as_deref() == Some("content_block_start")
                && frame.data.contains("tool_use")
        ));
        let _ = encoder.push(&CompletionEvent::Done {
            finish_reason: FinishReason::ToolCalls,
            usage: Some(TokenUsage {
                prompt_tokens: 3,
                completion_tokens: 4,
            }),
        });
        let body = encoder.json();
        assert_eq!(body["stop_reason"], "tool_use");
        assert_eq!(body["content"][0]["name"], "run");
    }

    #[test]
    fn responses_previous_appends_only_the_new_output() {
        let previous = vec![CompletionMessage::user("first")];
        let body = json!({
            "model": "gpt-5",
            "previous_response_id": "resp_old",
            "input": [{"type": "function_call_output", "call_id": "call_1", "output": "ok"}]
        });
        let parsed = parse_responses(&body, Some(&previous)).unwrap();
        assert_eq!(parsed.previous_response_id.as_deref(), Some("resp_old"));
        assert_eq!(parsed.incoming.request.messages.len(), 2);
        assert_eq!(parsed.incoming.request.messages[1].role, Role::Tool);
    }

    #[test]
    fn openai_stream_ends_with_done() {
        let mut encoder = OpenAiEncoder::new("chatcmpl_1", "qwen", 1);
        let frames = encoder.push(&CompletionEvent::Delta("Hi".into()));
        assert!(frames[0].data.contains("Hi"));
        let end = encoder.push(&CompletionEvent::Done {
            finish_reason: FinishReason::Stop,
            usage: None,
        });
        assert_eq!(end.last().unwrap().data, "[DONE]");
    }
}
