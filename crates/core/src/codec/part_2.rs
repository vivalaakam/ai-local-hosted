impl OpenAiEncoder {
    pub fn new(id: impl Into<String>, model: impl Into<String>, created: i64) -> Self {
        Self {
            id: id.into(),
            model: model.into(),
            created,
            acc: CompletionAccum::default(),
            finished: false,
        }
    }

    pub fn push(&mut self, event: &CompletionEvent) -> Vec<SseFrame> {
        self.acc.apply(event);
        match event {
            CompletionEvent::Delta(text) => vec![SseFrame::data(
                self.chunk(json!({"content": text}), None, None).to_string(),
            )],
            CompletionEvent::ThinkingDelta(text) => vec![SseFrame::data(
                self.chunk(json!({"reasoning_content": text}), None, None)
                    .to_string(),
            )],
            CompletionEvent::ToolCallDelta {
                index,
                id,
                name,
                arguments,
            } => {
                let mut function = serde_json::Map::new();
                if let Some(name) = name {
                    function.insert("name".into(), json!(name));
                }
                function.insert("arguments".into(), json!(arguments));
                let mut call = serde_json::Map::new();
                call.insert("index".into(), json!(index));
                if let Some(id) = id {
                    call.insert("id".into(), json!(id));
                }
                call.insert("type".into(), json!("function"));
                call.insert("function".into(), Value::Object(function));
                vec![SseFrame::data(
                    self.chunk(json!({"tool_calls": [Value::Object(call)]}), None, None)
                        .to_string(),
                )]
            }
            CompletionEvent::Done {
                finish_reason,
                usage,
            } => {
                self.finished = true;
                vec![
                    SseFrame::data(
                        self.chunk(json!({}), Some(finish_reason.openai()), *usage)
                            .to_string(),
                    ),
                    SseFrame::data("[DONE]"),
                ]
            }
        }
    }

    pub fn json(&self) -> Value {
        let mut message = json!({"role": "assistant", "content": self.acc.text});
        if !self.acc.tool_calls.is_empty() {
            message["tool_calls"] = json!(
                self.acc
                    .tool_calls
                    .iter()
                    .map(openai_tool_call)
                    .collect::<Vec<_>>()
            );
        }
        let mut body = json!({
            "id": self.id,
            "object": "chat.completion",
            "created": self.created,
            "model": self.model,
            "choices": [{
                "index": 0,
                "message": message,
                "finish_reason": self.acc.finish_reason().openai(),
            }],
        });
        if let Some(usage) = self.acc.usage {
            body["usage"] = usage_json(usage);
        }
        body
    }

    fn chunk(
        &self,
        delta: Value,
        finish_reason: Option<&str>,
        usage: Option<crate::message::TokenUsage>,
    ) -> Value {
        let mut body = json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
            "choices": [{
                "index": 0,
                "delta": delta,
                "finish_reason": finish_reason,
            }],
        });
        if let Some(usage) = usage {
            body["usage"] = usage_json(usage);
        }
        body
    }
}

pub struct AnthropicEncoder {
    id: String,
    model: String,
    acc: CompletionAccum,
    text_open: bool,
    thinking_open: bool,
    tool_open: Option<u32>,
    next_index: u32,
    started: bool,
}

