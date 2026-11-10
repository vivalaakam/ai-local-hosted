fn message_from(value: &Value, fallback: DateTime<Utc>) -> Option<ImportedMessage> {
    let role = text_field(value, &["sender", "role"])?;
    let role = match role.to_ascii_lowercase().as_str() {
        "human" | "user" => "user",
        "assistant" | "claude" => "assistant",
        _ => return None,
    };
    let (content, thinking) = message_body(value);
    if content.trim().is_empty() && thinking.is_none() {
        return None;
    }
    let created_at = time_of(value, &["created_at", "updated_at", "createdAt"]).unwrap_or(fallback);
    let id = uuid_field(value, &["uuid", "id"])
        .unwrap_or_else(|| stable_uuid(&format!("{role}|{created_at}|{content}")));
    Some(ImportedMessage {
        id,
        role: role.to_string(),
        content,
        thinking,
        created_at,
    })
}

fn message_body(message: &Value) -> (String, Option<String>) {
    let mut texts = Vec::new();
    let mut thinking = Vec::new();
    if let Some(content) = message.get("content").filter(|value| !value.is_null()) {
        push_content(&mut texts, &mut thinking, content);
    }
    if texts.iter().all(|text| text.trim().is_empty()) {
        texts.clear();
        if let Some(text) = message.get("text").and_then(Value::as_str) {
            let text = clean(text);
            if !text.trim().is_empty() {
                texts.push(text);
            }
        }
    }
    append_files(&mut texts, message.get("attachments"));
    append_files(&mut texts, message.get("files"));
    let content = clean(&texts.join("\n\n"));
    let thinking = thinking
        .into_iter()
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let thinking = clean(&thinking);
    let thinking = if thinking.trim().is_empty() {
        None
    } else {
        Some(thinking)
    };
    (content, thinking)
}

fn push_content(texts: &mut Vec<String>, thinking: &mut Vec<String>, value: &Value) {
    match value {
        Value::String(text) => push_text(texts, text),
        Value::Array(items) => {
            for item in items {
                push_content(texts, thinking, item);
            }
        }
        Value::Object(_) => push_block(texts, thinking, value),
        _ => {}
    }
}

fn push_block(texts: &mut Vec<String>, thinking: &mut Vec<String>, value: &Value) {
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "thinking" => push_text(thinking, &first_string(value, &["thinking", "text"])),
        "redacted_thinking" => push_text(thinking, "[thinking redacted]"),
        "tool_use" | "server_tool_use" | "tool_call" => {
            let name = first_string(value, &["name"]);
            let name = if name.is_empty() {
                "tool".to_string()
            } else {
                name
            };
            let input = pretty(value.get("input"));
            texts.push(format!("[tool {name}]\n{input}"));
        }
        "tool_result" => {
            let mut nested = Vec::new();
            let mut nested_thinking = Vec::new();
            if let Some(content) = value.get("content") {
                push_content(&mut nested, &mut nested_thinking, content);
            }
            if nested.is_empty() {
                push_text(&mut nested, &first_string(value, &["text"]));
            }
            thinking.extend(nested_thinking);
            if !nested.is_empty() {
                texts.push(format!("[tool result]\n{}", nested.join("\n\n")));
            }
        }
        _ => {
            let text = first_string(value, &["text"]);
            if !text.is_empty() {
                texts.push(text);
            }
        }
    }
}

fn append_files(texts: &mut Vec<String>, files: Option<&Value>) {
    let Some(files) = files.and_then(Value::as_array) else {
        return;
    };
    let joined = texts.join("\n\n");
    for file in files {
        let name =
            text_field(file, &["file_name", "fileName", "name"]).unwrap_or_else(|| "file".into());
        let name = clean(&name);
        let extracted = text_field(file, &["extracted_content", "extractedContent", "text"])
            .map(|text| clean(&text))
            .filter(|text| !text.trim().is_empty());
        if let Some(extracted) = extracted {
            if joined.contains(extracted.as_str()) {
                continue;
            }
            texts.push(format!("[file: {name}]\n{extracted}"));
        } else if !name.is_empty() {
            texts.push(format!("[file: {name}]"));
        }
    }
}

fn pretty(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => clean(text),
        Some(value) => serde_json::to_string_pretty(value).unwrap_or_default(),
    }
}

fn push_text(out: &mut Vec<String>, text: &str) {
    let text = clean(text);
    if !text.trim().is_empty() {
        out.push(text);
    }
}

fn first_string(value: &Value, keys: &[&str]) -> String {
    text_field(value, keys).unwrap_or_default()
}

fn text_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    })
}

fn uuid_field(value: &Value, keys: &[&str]) -> Option<Uuid> {
    text_field(value, keys).and_then(|text| Uuid::parse_str(&text).ok())
}

fn message_key(message: &Value) -> Option<String> {
    text_field(message, &["uuid", "id"])
}

fn parent_key(message: &Value) -> Option<String> {
    text_field(
        message,
        &["parent_message_uuid", "parent_uuid", "parent_id"],
    )
    .filter(|id| id != "00000000-0000-0000-0000-000000000000")
}

fn index_of(message: &Value) -> Option<i64> {
    let value = message.get("index")?;
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
        .or_else(|| value.as_f64().map(|number| number as i64))
}

fn time_of(value: &Value, keys: &[&str]) -> Option<DateTime<Utc>> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str).and_then(parse_time))
}

fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Some(parsed.with_timezone(&Utc));
    }
    let with_zone = format!("{value}Z");
    DateTime::parse_from_rfc3339(&with_zone)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

fn clean(text: &str) -> String {
    text.replace('\0', "")
}

fn stable_uuid(seed: &str) -> Uuid {
    let mut hi: u64 = 0xcbf29ce484222325;
    let mut lo: u64 = 0x100000001b3;
    for byte in seed.as_bytes() {
        hi ^= u64::from(*byte);
        hi = hi.wrapping_mul(0x100000001b3);
        lo ^= u64::from(*byte);
        lo = lo.wrapping_mul(0xcbf29ce484222325);
    }
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&hi.to_le_bytes());
    bytes[8..].copy_from_slice(&lo.to_le_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

