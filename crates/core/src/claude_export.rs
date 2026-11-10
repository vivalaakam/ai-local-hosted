//! Claude data-export conversations. The bytes are `conversations.json`
//! (one chat array, or one chat object). Zip extraction stays with the caller.

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedChat {
    pub id: Uuid,
    pub title: String,
    pub model: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub messages: Vec<ImportedMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedMessage {
    pub id: Uuid,
    pub role: String,
    pub content: String,
    pub thinking: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Parse one or more `conversations.json` documents into chats.
/// The same conversation id keeps the copy with more messages.
pub fn parse_claude_documents(documents: &[impl AsRef<[u8]>]) -> Result<Vec<ImportedChat>, String> {
    let mut merged: Vec<ImportedChat> = Vec::new();
    let mut failure = None;
    for document in documents {
        match parse_claude_export(document.as_ref()) {
            Ok(chats) => {
                for chat in chats {
                    if let Some(existing) = merged.iter_mut().find(|item| item.id == chat.id) {
                        if chat.messages.len() > existing.messages.len() {
                            *existing = chat;
                        }
                    } else {
                        merged.push(chat);
                    }
                }
            }
            Err(err) => failure = Some(err),
        }
    }
    if merged.is_empty() {
        return Err(failure.unwrap_or_else(|| "No Claude conversations found in that file.".into()));
    }
    Ok(merged)
}

pub fn parse_claude_export(bytes: &[u8]) -> Result<Vec<ImportedChat>, String> {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| "This file is not a Claude export.".to_string())?;
    let items = conversation_values(&value)?;
    let mut chats: Vec<ImportedChat> = Vec::new();
    for item in items {
        if let Some(chat) = chat_from(item) {
            if let Some(existing) = chats.iter_mut().find(|row| row.id == chat.id) {
                if chat.messages.len() > existing.messages.len() {
                    *existing = chat;
                }
            } else {
                chats.push(chat);
            }
        }
    }
    Ok(chats)
}

fn conversation_values(value: &Value) -> Result<Vec<&Value>, String> {
    if let Some(items) = value.as_array() {
        return Ok(items.iter().collect());
    }
    let Some(object) = value.as_object() else {
        return Err("This file is not a Claude export.".into());
    };
    for key in ["conversations", "chat_conversations", "chats"] {
        if let Some(items) = object.get(key).and_then(Value::as_array) {
            return Ok(items.iter().collect());
        }
    }
    if object.contains_key("chat_messages") || object.contains_key("messages") {
        return Ok(vec![value]);
    }
    if looks_like_manifest(object) {
        return Err(
            "This file is the export manifest. Import the conversations archive instead.".into(),
        );
    }
    let nested: Vec<&Value> = object
        .values()
        .filter(|item| item.get("chat_messages").is_some() || item.get("messages").is_some())
        .collect();
    if nested.is_empty() {
        Err("No Claude conversations found in that file.".into())
    } else {
        Ok(nested)
    }
}

fn looks_like_manifest(object: &serde_json::Map<String, Value>) -> bool {
    object.keys().any(|key| {
        let key = key.to_ascii_lowercase();
        key.contains("manifest") || key == "files" || key == "downloads" || key.ends_with(".zip")
    })
}

fn chat_from(value: &Value) -> Option<ImportedChat> {
    if !value.is_object() {
        return None;
    }
    let created_at = time_of(value, &["created_at", "createdAt"]).unwrap_or_else(Utc::now);
    let updated_at = time_of(value, &["updated_at", "updatedAt"]).unwrap_or(created_at);
    let messages = select_messages(value, created_at);
    let title = text_field(value, &["name", "title", "summary"]).unwrap_or_default();
    let title = clean(&title);
    let title = if title.is_empty() {
        "Untitled".to_string()
    } else {
        title.chars().take(300).collect()
    };
    let raw_messages = value
        .get("chat_messages")
        .or_else(|| value.get("messages"))
        .and_then(Value::as_array);
    let model = text_field(value, &["model", "model_slug"])
        .filter(|model| !model.is_empty())
        .or_else(|| {
            raw_messages.and_then(|items| {
                items.iter().rev().find_map(|message| {
                    text_field(message, &["model", "model_slug"]).filter(|model| !model.is_empty())
                })
            })
        })
        .map(|model| clean(&model).chars().take(200).collect())
        .unwrap_or_default();
    let id = uuid_field(value, &["uuid", "id"]).unwrap_or_else(|| {
        let seed = format!(
            "{title}|{}|{}",
            created_at.timestamp_micros(),
            messages
                .first()
                .map(|message| message.content.as_str())
                .unwrap_or("")
        );
        stable_uuid(&seed)
    });
    Some(ImportedChat {
        id,
        title,
        model,
        created_at,
        updated_at,
        messages,
    })
}

fn select_messages(chat: &Value, fallback: DateTime<Utc>) -> Vec<ImportedMessage> {
    let Some(raw) = chat
        .get("chat_messages")
        .or_else(|| chat.get("messages"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let ordered = order_messages(chat, raw);
    let mut messages: Vec<ImportedMessage> = Vec::new();
    for item in ordered {
        if let Some(message) = message_from(item, fallback) {
            if messages.iter().any(|row| row.id == message.id) {
                continue;
            }
            messages.push(message);
        }
    }
    messages
}

fn order_messages<'a>(chat: &'a Value, raw: &'a [Value]) -> Vec<&'a Value> {
    let indexed = raw.iter().enumerate().collect::<Vec<_>>();
    let all_indexed = indexed
        .iter()
        .all(|(_, message)| index_of(message).is_some());
    let mut file_order = indexed;
    if all_indexed {
        file_order.sort_by_key(|(position, message)| (index_of(message).unwrap_or(0), *position));
    }
    let by_id = file_order
        .iter()
        .filter_map(|(_, message)| message_key(message).map(|id| (id, *message)))
        .collect::<std::collections::BTreeMap<_, _>>();
    let linked = file_order.iter().any(|(_, message)| {
        parent_key(message).is_some_and(|parent| by_id.contains_key(parent.as_str()))
    });
    if !linked {
        return file_order.into_iter().map(|(_, message)| message).collect();
    }
    let leaf = explicit_leaf(chat, &by_id).or_else(|| latest_leaf(&file_order, &by_id));
    let Some(leaf) = leaf else {
        return file_order.into_iter().map(|(_, message)| message).collect();
    };
    let mut chain = Vec::new();
    let mut cursor = Some(leaf);
    let mut guard = 0;
    while let Some(message) = cursor {
        if guard > raw.len() {
            break;
        }
        guard += 1;
        chain.push(message);
        cursor = parent_key(message).and_then(|parent| by_id.get(parent.as_str()).copied());
    }
    chain.reverse();
    if chain.is_empty() {
        file_order.into_iter().map(|(_, message)| message).collect()
    } else {
        chain
    }
}

fn explicit_leaf<'a>(
    chat: &Value,
    by_id: &std::collections::BTreeMap<String, &'a Value>,
) -> Option<&'a Value> {
    let id = text_field(
        chat,
        &[
            "current_leaf_message_uuid",
            "current_leaf",
            "leaf_message_uuid",
        ],
    )?;
    by_id.get(&id).copied()
}

fn latest_leaf<'a>(
    ordered: &[(usize, &'a Value)],
    by_id: &std::collections::BTreeMap<String, &'a Value>,
) -> Option<&'a Value> {
    let parents: std::collections::BTreeSet<String> = by_id
        .values()
        .filter_map(|message| parent_key(message))
        .filter(|parent| by_id.contains_key(parent.as_str()))
        .collect();
    ordered
        .iter()
        .map(|(_, message)| *message)
        .filter(|message| message_key(message).is_none_or(|id| !parents.contains(&id)))
        .max_by_key(|message| {
            time_of(message, &["created_at", "updated_at", "createdAt"])
                .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
        })
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_text_thinking_tools_files_and_the_active_branch() {
        let raw = r#"{
            "conversations": [
                {
                    "uuid": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                    "name": "From Claude",
                    "model": "claude-sonnet-4-5",
                    "created_at": "2024-06-01T00:00:00Z",
                    "updated_at": "2024-06-01T00:05:00Z",
                    "chat_messages": [
                        {
                            "uuid": "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                            "sender": "human",
                            "text": "Hello from the export",
                            "created_at": "2024-06-01T00:00:00Z",
                            "attachments": [{
                                "file_name": "notes.txt",
                                "extracted_content": "remember this"
                            }]
                        },
                        {
                            "uuid": "cccccccc-cccc-cccc-cccc-cccccccccccc",
                            "sender": "assistant",
                            "created_at": "2024-06-01T00:01:00Z",
                            "content": [
                                {"type": "thinking", "thinking": "plan"},
                                {"type": "text", "text": "Here you go"},
                                {"type": "tool_use", "name": "search", "input": {"q": "alh"}}
                            ]
                        }
                    ]
                },
                {
                    "uuid": "dddddddd-dddd-dddd-dddd-dddddddddddd",
                    "name": "Branched",
                    "created_at": "2024-05-01T00:00:00Z",
                    "updated_at": "2024-05-01T00:00:02Z",
                    "current_leaf_message_uuid": "ffffffff-ffff-ffff-ffff-ffffffffffff",
                    "chat_messages": [
                        {"uuid": "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee", "sender": "human", "text": "question", "created_at": "2024-05-01T00:00:00Z"},
                        {"uuid": "99999999-9999-9999-9999-999999999999", "parent_message_uuid": "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee", "sender": "assistant", "text": "old", "created_at": "2024-05-01T00:00:03Z"},
                        {"uuid": "ffffffff-ffff-ffff-ffff-ffffffffffff", "parent_message_uuid": "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee", "sender": "assistant", "text": "chosen", "created_at": "2024-05-01T00:00:01Z"}
                    ]
                }
            ]
        }"#;
        let chats = parse_claude_export(raw.as_bytes()).unwrap();
        assert_eq!(chats.len(), 2);
        assert_eq!(chats[0].title, "From Claude");
        assert_eq!(chats[0].model, "claude-sonnet-4-5");
        assert_eq!(chats[0].messages.len(), 2);
        assert_eq!(chats[0].messages[0].role, "user");
        assert!(
            chats[0].messages[0]
                .content
                .contains("Hello from the export")
        );
        assert!(chats[0].messages[0].content.contains("remember this"));
        assert_eq!(chats[0].messages[1].thinking.as_deref(), Some("plan"));
        assert!(chats[0].messages[1].content.contains("Here you go"));
        assert!(chats[0].messages[1].content.contains("[tool search]"));
        assert_eq!(chats[1].messages.len(), 2);
        assert_eq!(chats[1].messages[1].content, "chosen");
    }

    #[test]
    fn manifest_is_not_a_conversation_file() {
        let err = parse_claude_export(br#"{"manifest":"x","files":[]}"#).unwrap_err();
        assert!(err.contains("manifest"));
    }
}
