impl SseParser {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<CompletionEvent>, ProviderError> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.consume_line(&line, &mut out)?;
        }
        Ok(out)
    }

    fn finish(&mut self) -> Vec<CompletionEvent> {
        let mut out = Vec::new();
        if !self.buf.is_empty() {
            let line = std::mem::take(&mut self.buf);
            let _ = self.consume_line(&line, &mut out);
        }
        self.emit_done(&mut out);
        out
    }

    fn consume_line(
        &mut self,
        line: &[u8],
        out: &mut Vec<CompletionEvent>,
    ) -> Result<(), ProviderError> {
        if line.is_empty() || line[0] == b':' {
            return Ok(());
        }
        let text = String::from_utf8(line.to_vec()).map_err(|_| {
            ProviderError::with_detail(ProviderErrorKind::Decode, "sse line is not utf-8")
        })?;
        let Some(data) = text.strip_prefix("data:") else {
            return Ok(());
        };
        let data = data.trim_start();
        if data.is_empty() {
            return Ok(());
        }
        if data == "[DONE]" {
            self.emit_done(out);
            return Ok(());
        }
        let value: Value = serde_json::from_str(data).map_err(|err| {
            ProviderError::with_detail(ProviderErrorKind::Decode, err.to_string())
        })?;
        if let Some(error) = value.get("error") {
            return Err(ProviderError::with_detail(
                ProviderErrorKind::Invalid,
                clip(&error.to_string()),
            ));
        }
        self.consume_chunk(&value, out);
        Ok(())
    }

    fn consume_chunk(&mut self, value: &Value, out: &mut Vec<CompletionEvent>) {
        let choice = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first());
        if let Some(choice) = choice {
            let delta = choice.get("delta").unwrap_or(&Value::Null);
            if let Some(text) = delta.get("content").and_then(Value::as_str)
                && !text.is_empty()
            {
                out.push(CompletionEvent::Delta(text.to_string()));
            }
            let thinking = delta
                .get("reasoning_content")
                .and_then(Value::as_str)
                .or_else(|| delta.get("reasoning").and_then(Value::as_str));
            if let Some(text) = thinking.filter(|text| !text.is_empty()) {
                out.push(CompletionEvent::ThinkingDelta(text.to_string()));
            }
            if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    let index = call.get("index").and_then(json_u32).unwrap_or(0);
                    let function = call.get("function").unwrap_or(&Value::Null);
                    out.push(CompletionEvent::ToolCallDelta {
                        index,
                        id: call.get("id").and_then(Value::as_str).map(str::to_string),
                        name: function
                            .get("name")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        arguments: function
                            .get("arguments")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    });
                }
            }
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                self.pending_finish = Some(FinishReason::from_openai(reason));
            }
        }
        if let Some(usage) = value.get("usage").filter(|usage| usage.is_object()) {
            self.usage = Some(alh_core::TokenUsage {
                prompt_tokens: usage.get("prompt_tokens").and_then(json_u32).unwrap_or(0),
                completion_tokens: usage
                    .get("completion_tokens")
                    .and_then(json_u32)
                    .unwrap_or(0),
            });
            self.emit_done(out);
        }
    }

    fn emit_done(&mut self, out: &mut Vec<CompletionEvent>) {
        if self.emitted_done {
            return;
        }
        self.emitted_done = true;
        out.push(CompletionEvent::Done {
            finish_reason: self.pending_finish.clone().unwrap_or(FinishReason::Stop),
            usage: self.usage,
        });
    }
}

fn openai_body(request: &CompletionRequest) -> Value {
    let mut body = json!({
        "model": request.model,
        "messages": request.messages.iter().map(openai_message).collect::<Vec<_>>(),
        "stream": true,
        "stream_options": {"include_usage": true},
    });
    if !request.tools.is_empty() {
        body["tools"] = json!(
            request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        }
                    })
                })
                .collect::<Vec<_>>()
        );
    }
    if let Some(choice) = &request.tool_choice {
        body["tool_choice"] = choice.clone();
    }
    if let Some(temperature) = request.temperature {
        body["temperature"] = json!(temperature);
    }
    if let Some(top_p) = request.top_p {
        body["top_p"] = json!(top_p);
    }
    if let Some(max_tokens) = request.max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    if !request.stop.is_empty() {
        body["stop"] = json!(request.stop);
    }
    body
}

fn openai_message(message: &alh_core::CompletionMessage) -> Value {
    let mut value = json!({
        "role": message.role.as_str(),
        "content": message.content,
    });
    if !message.tool_calls.is_empty() {
        value["tool_calls"] = json!(
            message
                .tool_calls
                .iter()
                .map(|call| {
                    json!({
                        "id": call.id,
                        "type": "function",
                        "function": {"name": call.name, "arguments": call.arguments},
                    })
                })
                .collect::<Vec<_>>()
        );
    }
    if let Some(id) = &message.tool_call_id {
        value["tool_call_id"] = json!(id);
    }
    value
}

fn json_u32(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|number| u32::try_from(number).ok())
        .or_else(|| value.as_i64().and_then(|number| u32::try_from(number).ok()))
}

fn map_transport(err: reqwest::Error) -> ProviderError {
    let kind = if err.is_timeout() {
        ProviderErrorKind::Timeout
    } else {
        ProviderErrorKind::Unavailable
    };
    ProviderError::with_detail(kind, err.to_string())
}

async fn status_error(response: reqwest::Response) -> ProviderError {
    let status = response.status();
    let kind = match status.as_u16() {
        401 | 403 => ProviderErrorKind::Auth,
        404 => ProviderErrorKind::NotFound,
        429 => ProviderErrorKind::RateLimited,
        408 | 504 => ProviderErrorKind::Timeout,
        400 | 422 => ProviderErrorKind::Invalid,
        _ => ProviderErrorKind::Unavailable,
    };
    let body = response.text().await.unwrap_or_default();
    ProviderError::with_detail(kind, format!("{status} {}", clip(&body)))
}

fn clip(text: &str) -> String {
    let mut end = text.len().min(2_000);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

fn shared_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| reqwest::Client::builder().build().expect("reqwest client"))
        .clone()
}
