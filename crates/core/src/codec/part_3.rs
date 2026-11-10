impl AnthropicEncoder {
    pub fn new(id: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            model: model.into(),
            acc: CompletionAccum::default(),
            text_open: false,
            thinking_open: false,
            tool_open: None,
            next_index: 0,
            started: false,
        }
    }

    pub fn push(&mut self, event: &CompletionEvent) -> Vec<SseFrame> {
        self.acc.apply(event);
        let mut frames = Vec::new();
        self.start(&mut frames);
        match event {
            CompletionEvent::ThinkingDelta(text) => {
                self.close_text(&mut frames);
                self.close_tool(&mut frames);
                if !self.thinking_open {
                    frames.push(SseFrame::named(
                        "content_block_start",
                        json!({
                            "type": "content_block_start",
                            "index": self.next_index,
                            "content_block": {"type": "thinking", "thinking": ""},
                        }),
                    ));
                    self.thinking_open = true;
                    self.next_index += 1;
                }
                frames.push(SseFrame::named(
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": self.next_index - 1,
                        "delta": {"type": "thinking_delta", "thinking": text},
                    }),
                ));
            }
            CompletionEvent::Delta(text) => {
                self.close_thinking(&mut frames);
                self.close_tool(&mut frames);
                if !self.text_open {
                    frames.push(SseFrame::named(
                        "content_block_start",
                        json!({
                            "type": "content_block_start",
                            "index": self.next_index,
                            "content_block": {"type": "text", "text": ""},
                        }),
                    ));
                    self.text_open = true;
                    self.next_index += 1;
                }
                frames.push(SseFrame::named(
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": self.next_index - 1,
                        "delta": {"type": "text_delta", "text": text},
                    }),
                ));
            }
            CompletionEvent::ToolCallDelta {
                index,
                id,
                name,
                arguments,
            } => {
                self.close_thinking(&mut frames);
                self.close_text(&mut frames);
                if self.tool_open != Some(*index) {
                    self.close_tool(&mut frames);
                    frames.push(SseFrame::named(
                        "content_block_start",
                        json!({
                            "type": "content_block_start",
                            "index": self.next_index,
                            "content_block": {
                                "type": "tool_use",
                                "id": id.clone().unwrap_or_default(),
                                "name": name.clone().unwrap_or_default(),
                                "input": {},
                            },
                        }),
                    ));
                    self.tool_open = Some(*index);
                    self.next_index += 1;
                }
                if !arguments.is_empty() {
                    frames.push(SseFrame::named(
                        "content_block_delta",
                        json!({
                            "type": "content_block_delta",
                            "index": self.next_index - 1,
                            "delta": {"type": "input_json_delta", "partial_json": arguments},
                        }),
                    ));
                }
            }
            CompletionEvent::Done {
                finish_reason,
                usage,
            } => {
                self.close_thinking(&mut frames);
                self.close_text(&mut frames);
                self.close_tool(&mut frames);
                frames.push(SseFrame::named(
                    "message_delta",
                    json!({
                        "type": "message_delta",
                        "delta": {"stop_reason": finish_reason.anthropic(), "stop_sequence": null},
                        "usage": {"output_tokens": usage.map(|usage| usage.completion_tokens).unwrap_or(0)},
                    }),
                ));
                frames.push(SseFrame::named(
                    "message_stop",
                    json!({"type": "message_stop"}),
                ));
            }
        }
        frames
    }

    pub fn json(&self) -> Value {
        let mut content = Vec::new();
        if !self.acc.thinking.is_empty() {
            content.push(json!({"type": "thinking", "thinking": self.acc.thinking}));
        }
        if !self.acc.text.is_empty() || self.acc.tool_calls.is_empty() {
            content.push(json!({"type": "text", "text": self.acc.text}));
        }
        for call in &self.acc.tool_calls {
            content.push(json!({
                "type": "tool_use",
                "id": call.id,
                "name": call.name,
                "input": parse_arguments(&call.arguments),
            }));
        }
        json!({
            "id": self.id,
            "type": "message",
            "role": "assistant",
            "model": self.model,
            "content": content,
            "stop_reason": self.acc.finish_reason().anthropic(),
            "stop_sequence": null,
            "usage": {
                "input_tokens": self.acc.usage.map(|usage| usage.prompt_tokens).unwrap_or(0),
                "output_tokens": self.acc.usage.map(|usage| usage.completion_tokens).unwrap_or(0),
            },
        })
    }

    fn start(&mut self, frames: &mut Vec<SseFrame>) {
        if self.started {
            return;
        }
        self.started = true;
        frames.push(SseFrame::named(
            "message_start",
            json!({
                "type": "message_start",
                "message": {
                    "id": self.id,
                    "type": "message",
                    "role": "assistant",
                    "model": self.model,
                    "content": [],
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": {"input_tokens": 0, "output_tokens": 0},
                },
            }),
        ));
    }

    fn close_text(&mut self, frames: &mut Vec<SseFrame>) {
        if self.text_open {
            self.close_block(frames);
            self.text_open = false;
        }
    }

    fn close_thinking(&mut self, frames: &mut Vec<SseFrame>) {
        if self.thinking_open {
            self.close_block(frames);
            self.thinking_open = false;
        }
    }

    fn close_tool(&mut self, frames: &mut Vec<SseFrame>) {
        if self.tool_open.take().is_some() {
            self.close_block(frames);
        }
    }

    fn close_block(&self, frames: &mut Vec<SseFrame>) {
        frames.push(SseFrame::named(
            "content_block_stop",
            json!({
                "type": "content_block_stop",
                "index": self.next_index.saturating_sub(1),
            }),
        ));
    }
}

pub struct ResponsesEncoder {
    id: String,
    model: String,
    acc: CompletionAccum,
    text_started: bool,
    tool_started: Option<u32>,
    output_index: u32,
}

