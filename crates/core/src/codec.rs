use serde_json::{Value, json};

use crate::error::TranslateError;
use crate::message::{
    CompletionMessage, Role, ToolCall, ToolDefinition, estimate_tokens, require_str, text_and_parts,
};
use crate::provider::{CompletionAccum, CompletionEvent, CompletionRequest};

#[derive(Debug, Clone, PartialEq)]
pub struct IncomingCompletion {
    pub client_model: String,
    pub request: CompletionRequest,
    pub stream: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResponsesParsed {
    pub incoming: IncomingCompletion,
    pub previous_response_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseFrame {
    pub event: Option<String>,
    pub data: String,
}

impl SseFrame {
    fn data(data: impl Into<String>) -> Self {
        Self {
            event: None,
            data: data.into(),
        }
    }

    fn named(event: &str, data: Value) -> Self {
        Self {
            event: Some(event.to_string()),
            data: data.to_string(),
        }
    }
}

pub fn parse_chat_completion(body: &Value) -> Result<IncomingCompletion, TranslateError> {
    let client_model = require_str(body, "model")?.to_string();
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| TranslateError::new("messages is required"))?;
    let mut parsed = Vec::new();
    for message in messages {
        parsed.push(openai_message(message)?);
    }
    Ok(IncomingCompletion {
        client_model: client_model.clone(),
        stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
        request: CompletionRequest {
            model: client_model,
            messages: parsed,
            tools: openai_tools(body.get("tools")),
            tool_choice: body.get("tool_choice").cloned(),
            temperature: number(body, "temperature"),
            top_p: number(body, "top_p"),
            max_tokens: u32_field(body, "max_completion_tokens")
                .or_else(|| u32_field(body, "max_tokens")),
            stop: stop_list(body.get("stop")),
        },
    })
}

pub fn parse_anthropic_message(body: &Value) -> Result<IncomingCompletion, TranslateError> {
    let client_model = require_str(body, "model")?.to_string();
    let mut messages = Vec::new();
    if let Some(system) = body.get("system") {
        let text = anthropic_system(system);
        if !text.is_empty() {
            messages.push(CompletionMessage::system(text));
        }
    }
    let raw_messages = body
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| TranslateError::new("messages is required"))?;
    for message in raw_messages {
        messages.extend(anthropic_message(message)?);
    }
    Ok(IncomingCompletion {
        client_model: client_model.clone(),
        stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
        request: CompletionRequest {
            model: client_model,
            messages,
            tools: anthropic_tools(body.get("tools")),
            tool_choice: body.get("tool_choice").cloned(),
            temperature: number(body, "temperature"),
            top_p: number(body, "top_p"),
            max_tokens: u32_field(body, "max_tokens"),
            stop: string_array(body.get("stop_sequences")),
        },
    })
}

pub fn parse_responses(
    body: &Value,
    previous: Option<&[CompletionMessage]>,
) -> Result<ResponsesParsed, TranslateError> {
    let client_model = require_str(body, "model")?.to_string();
    let previous_response_id = body
        .get("previous_response_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let mut messages = if let Some(previous) = previous {
        previous.to_vec()
    } else {
        Vec::new()
    };
    if let Some(instructions) = body.get("instructions").and_then(Value::as_str) {
        let instructions = instructions.trim();
        if !instructions.is_empty() {
            if matches!(messages.first(), Some(message) if message.role == Role::System) {
                messages[0] = CompletionMessage::system(instructions);
            } else {
                messages.insert(0, CompletionMessage::system(instructions));
            }
        }
    }
    let input = body
        .get("input")
        .ok_or_else(|| TranslateError::new("input is required"))?;
    messages.extend(responses_input(input)?);
    if messages.iter().all(|message| message.role == Role::System) {
        return Err(TranslateError::new("input is required"));
    }
    Ok(ResponsesParsed {
        previous_response_id,
        incoming: IncomingCompletion {
            client_model: client_model.clone(),
            stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
            request: CompletionRequest {
                model: client_model,
                messages,
                tools: responses_tools(body.get("tools")),
                tool_choice: body.get("tool_choice").cloned(),
                temperature: number(body, "temperature"),
                top_p: number(body, "top_p"),
                max_tokens: u32_field(body, "max_output_tokens"),
                stop: Vec::new(),
            },
        },
    })
}

pub fn estimate_request_tokens(request: &CompletionRequest) -> u32 {
    estimate_tokens(&request.messages, &request.tools)
}

pub struct OpenAiEncoder {
    id: String,
    model: String,
    created: i64,
    acc: CompletionAccum,
    finished: bool,
}

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

fn openai_call(fallback_index: u32, value: &Value) -> Option<ToolCall> {
    let function = value.get("function")?;
    Some(ToolCall {
        index: value
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|index| u32::try_from(index).ok())
            .unwrap_or(fallback_index),
        id: value
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        name: function
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        arguments: json_string(function.get("arguments")),
    })
}

fn openai_tools(value: Option<&Value>) -> Vec<ToolDefinition> {
    value
        .and_then(Value::as_array)
        .map(|tools| tools.iter().filter_map(openai_tool).collect())
        .unwrap_or_default()
}

fn openai_tool(value: &Value) -> Option<ToolDefinition> {
    let function = value.get("function").unwrap_or(value);
    Some(ToolDefinition {
        name: function.get("name")?.as_str()?.to_string(),
        description: function
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        parameters: function
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| json!({"type": "object"})),
    })
}

fn anthropic_system(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn anthropic_message(value: &Value) -> Result<Vec<CompletionMessage>, TranslateError> {
    let role = value
        .get("role")
        .and_then(Value::as_str)
        .and_then(Role::parse)
        .ok_or_else(|| TranslateError::new("message role is invalid"))?;
    let content = value.get("content").unwrap_or(&Value::Null);
    if let Value::String(text) = content {
        return Ok(vec![CompletionMessage::text(role, text.clone())]);
    }
    let Some(blocks) = content.as_array() else {
        return Ok(vec![CompletionMessage::text(role, String::new())]);
    };
    let mut text = String::new();
    let mut thinking = String::new();
    let mut tool_calls = Vec::new();
    let mut tool_results = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(piece) = block.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(piece);
                }
            }
            Some("thinking") => {
                if let Some(piece) = block.get("thinking").and_then(Value::as_str) {
                    thinking.push_str(piece);
                }
            }
            Some("tool_use") => tool_calls.push(ToolCall {
                index: u32::try_from(tool_calls.len()).unwrap_or(u32::MAX),
                id: block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                name: block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                arguments: block
                    .get("input")
                    .map(|input| input.to_string())
                    .unwrap_or_else(|| "{}".into()),
            }),
            Some("tool_result") => {
                let (result, _) = block
                    .get("content")
                    .map(text_and_parts)
                    .unwrap_or_else(|| (String::new(), None));
                tool_results.push(CompletionMessage {
                    role: Role::Tool,
                    content: result,
                    parts: None,
                    tool_calls: Vec::new(),
                    tool_call_id: block
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    thinking: None,
                });
            }
            _ => {}
        }
    }
    let mut messages = Vec::new();
    if !text.is_empty() || !tool_calls.is_empty() || tool_results.is_empty() {
        messages.push(CompletionMessage {
            role,
            content: text,
            parts: None,
            tool_calls,
            tool_call_id: None,
            thinking: if thinking.is_empty() {
                None
            } else {
                Some(thinking)
            },
        });
    }
    messages.extend(tool_results);
    Ok(messages)
}

fn anthropic_tools(value: Option<&Value>) -> Vec<ToolDefinition> {
    value
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| {
                    Some(ToolDefinition {
                        name: tool.get("name")?.as_str()?.to_string(),
                        description: tool
                            .get("description")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        parameters: tool
                            .get("input_schema")
                            .cloned()
                            .unwrap_or_else(|| json!({"type": "object"})),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn responses_input(value: &Value) -> Result<Vec<CompletionMessage>, TranslateError> {
    match value {
        Value::String(text) => Ok(vec![CompletionMessage::user(text.clone())]),
        Value::Array(items) => {
            let mut messages = Vec::new();
            for item in items {
                if let Some(text) = item.as_str() {
                    messages.push(CompletionMessage::user(text.to_string()));
                    continue;
                }
                match item.get("type").and_then(Value::as_str) {
                    Some("message") | None if item.get("role").is_some() => {
                        messages.push(responses_message(item)?);
                    }
                    Some("function_call") => messages.push(CompletionMessage {
                        role: Role::Assistant,
                        content: String::new(),
                        parts: None,
                        tool_calls: vec![ToolCall {
                            index: 0,
                            id: item
                                .get("call_id")
                                .or_else(|| item.get("id"))
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                            name: item
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                            arguments: json_string(item.get("arguments")),
                        }],
                        tool_call_id: None,
                        thinking: None,
                    }),
                    Some("function_call_output") => messages.push(CompletionMessage {
                        role: Role::Tool,
                        content: json_string(item.get("output")),
                        parts: None,
                        tool_calls: Vec::new(),
                        tool_call_id: item
                            .get("call_id")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        thinking: None,
                    }),
                    Some("input_text") => {
                        if let Some(text) = item.get("text").and_then(Value::as_str) {
                            messages.push(CompletionMessage::user(text.to_string()));
                        }
                    }
                    _ => {}
                }
            }
            Ok(messages)
        }
        _ => Err(TranslateError::new("input is invalid")),
    }
}

fn responses_message(value: &Value) -> Result<CompletionMessage, TranslateError> {
    let role = value
        .get("role")
        .and_then(Value::as_str)
        .and_then(Role::parse)
        .ok_or_else(|| TranslateError::new("message role is invalid"))?;
    let (content, parts) = value
        .get("content")
        .map(text_and_parts)
        .unwrap_or_else(|| (String::new(), None));
    Ok(CompletionMessage {
        role,
        content,
        parts,
        tool_calls: Vec::new(),
        tool_call_id: None,
        thinking: None,
    })
}

fn responses_tools(value: Option<&Value>) -> Vec<ToolDefinition> {
    value
        .and_then(Value::as_array)
        .map(|tools| tools.iter().filter_map(openai_tool).collect())
        .unwrap_or_default()
}

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
