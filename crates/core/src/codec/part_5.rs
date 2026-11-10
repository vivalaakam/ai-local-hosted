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

