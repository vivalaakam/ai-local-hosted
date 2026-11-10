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

