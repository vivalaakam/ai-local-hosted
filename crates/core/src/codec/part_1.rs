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

