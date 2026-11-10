impl ResponsesEncoder {
    pub fn new(id: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            model: model.into(),
            acc: CompletionAccum::default(),
            text_started: false,
            tool_started: None,
            output_index: 0,
        }
    }

    pub fn push(&mut self, event: &CompletionEvent) -> Vec<SseFrame> {
        self.acc.apply(event);
        let mut frames = Vec::new();
        match event {
            CompletionEvent::Delta(text) => {
                self.ensure_text(&mut frames);
                frames.push(SseFrame::named(
                    "response.output_text.delta",
                    json!({
                        "type": "response.output_text.delta",
                        "output_index": 0,
                        "content_index": 0,
                        "delta": text,
                    }),
                ));
            }
            CompletionEvent::ThinkingDelta(_) => {}
            CompletionEvent::ToolCallDelta {
                index,
                id,
                name,
                arguments,
            } => {
                if self.tool_started != Some(*index) {
                    self.tool_started = Some(*index);
                    let item_index = self.output_index;
                    self.output_index += 1;
                    frames.push(SseFrame::named(
                        "response.output_item.added",
                        json!({
                            "type": "response.output_item.added",
                            "output_index": item_index,
                            "item": {
                                "type": "function_call",
                                "id": format!("fc_{}", id.clone().unwrap_or_else(|| index.to_string())),
                                "call_id": id.clone().unwrap_or_default(),
                                "name": name.clone().unwrap_or_default(),
                                "arguments": "",
                                "status": "in_progress",
                            },
                        }),
                    ));
                }
                if !arguments.is_empty() {
                    frames.push(SseFrame::named(
                        "response.function_call_arguments.delta",
                        json!({
                            "type": "response.function_call_arguments.delta",
                            "output_index": self.output_index.saturating_sub(1),
                            "delta": arguments,
                        }),
                    ));
                }
            }
            CompletionEvent::Done { .. } => {
                if self.text_started {
                    frames.push(SseFrame::named(
                        "response.output_text.done",
                        json!({
                            "type": "response.output_text.done",
                            "output_index": 0,
                            "content_index": 0,
                            "text": self.acc.text,
                        }),
                    ));
                    frames.push(SseFrame::named(
                        "response.output_item.done",
                        json!({
                            "type": "response.output_item.done",
                            "output_index": 0,
                            "item": self.message_item(),
                        }),
                    ));
                }
                for (offset, call) in self.acc.tool_calls.iter().enumerate() {
                    let output_index = if self.text_started {
                        offset as u32 + 1
                    } else {
                        offset as u32
                    };
                    frames.push(SseFrame::named(
                        "response.function_call_arguments.done",
                        json!({
                            "type": "response.function_call_arguments.done",
                            "output_index": output_index,
                            "arguments": call.arguments,
                        }),
                    ));
                    frames.push(SseFrame::named(
                        "response.output_item.done",
                        json!({
                            "type": "response.output_item.done",
                            "output_index": output_index,
                            "item": function_item(call),
                        }),
                    ));
                }
                frames.push(SseFrame::named(
                    "response.completed",
                    json!({"type": "response.completed", "response": self.json()}),
                ));
            }
        }
        frames
    }

    pub fn created_frame(&self) -> SseFrame {
        SseFrame::named(
            "response.created",
            json!({"type": "response.created", "response": self.snapshot("in_progress")}),
        )
    }

    pub fn json(&self) -> Value {
        self.snapshot("completed")
    }

    pub fn output_items(&self) -> Value {
        self.json()["output"].clone()
    }

    fn ensure_text(&mut self, frames: &mut Vec<SseFrame>) {
        if self.text_started {
            return;
        }
        self.text_started = true;
        frames.push(SseFrame::named(
            "response.output_item.added",
            json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {
                    "type": "message",
                    "id": format!("msg_{}", self.id),
                    "role": "assistant",
                    "status": "in_progress",
                    "content": [],
                },
            }),
        ));
        frames.push(SseFrame::named(
            "response.content_part.added",
            json!({
                "type": "response.content_part.added",
                "output_index": 0,
                "content_index": 0,
                "part": {"type": "output_text", "text": ""},
            }),
        ));
        if self.output_index == 0 {
            self.output_index = 1;
        }
    }

    fn snapshot(&self, status: &str) -> Value {
        let mut output = Vec::new();
        if self.text_started || !self.acc.text.is_empty() || self.acc.tool_calls.is_empty() {
            output.push(self.message_item());
        }
        for call in &self.acc.tool_calls {
            output.push(function_item(call));
        }
        json!({
            "id": self.id,
            "object": "response",
            "status": status,
            "model": self.model,
            "output": output,
            "output_text": self.acc.text,
        })
    }

    fn message_item(&self) -> Value {
        json!({
            "type": "message",
            "id": format!("msg_{}", self.id),
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": self.acc.text, "annotations": []}],
        })
    }
}

fn function_item(call: &ToolCall) -> Value {
    json!({
        "type": "function_call",
        "id": format!("fc_{}", call.id),
        "call_id": call.id,
        "name": call.name,
        "arguments": call.arguments,
        "status": "completed",
    })
}

fn openai_message(value: &Value) -> Result<CompletionMessage, TranslateError> {
    let role = value
        .get("role")
        .and_then(Value::as_str)
        .and_then(Role::parse)
        .ok_or_else(|| TranslateError::new("message role is invalid"))?;
    let (content, parts) = value
        .get("content")
        .map(text_and_parts)
        .unwrap_or_else(|| (String::new(), None));
    let tool_calls = value
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .enumerate()
                .filter_map(|(index, call)| openai_call(index as u32, call))
                .collect()
        })
        .unwrap_or_default();
    Ok(CompletionMessage {
        role,
        content,
        parts,
        tool_calls,
        tool_call_id: value
            .get("tool_call_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        thinking: None,
    })
}

