use super::*;

pub(crate) async fn set_default_on(
    conn: &impl ConnectionTrait,
    user_id: Uuid,
    id: Uuid,
) -> Result<(), StorageError> {
    query::exec(
        conn,
        "UPDATE providers SET is_default = FALSE WHERE user_id = $1",
        vec![Value::from(user_id)],
    )
    .await?;
    let result = query::exec(
        conn,
        "UPDATE providers SET is_default = TRUE WHERE user_id = $1 AND id = $2",
        vec![Value::from(user_id), Value::from(id)],
    )
    .await?;
    if result.rows_affected() == 0 {
        return Err(StorageError::NotFound);
    }
    Ok(())
}

pub(crate) async fn layers_on(
    conn: &impl ConnectionTrait,
    user_id: Uuid,
) -> Result<SystemPromptLayers, StorageError> {
    let rows = query::all(
        conn,
        "SELECT user_id, scope, content FROM system_prompts
         WHERE (user_id = $1 AND scope IN ('assigned', 'self'))
            OR (user_id = $2 AND scope = 'global')",
        vec![Value::from(user_id), Value::from(LEGACY_USER_ID)],
    )
    .await?;
    let mut layers = SystemPromptLayers::default();
    for row in rows {
        let scope: String = row.try_get("", "scope")?;
        let content: String = row.try_get("", "content")?;
        match scope.as_str() {
            "global" => layers.global = Some(content),
            "assigned" => layers.assigned = Some(content),
            "self" => layers.own = Some(content),
            _ => {}
        }
    }
    Ok(layers)
}

/// Prompt text shown once in the chat transcript on the first turn.
pub(crate) fn display_system_prompt(
    layers: &SystemPromptLayers,
    conversation_prompt: Option<&str>,
    tools_prompt: Option<&str>,
) -> Option<String> {
    let mut parts = layers.effective();
    if let Some(prompt) = conversation_prompt
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
    {
        parts.push(prompt.to_string());
    }
    if let Some(prompt) = tools_prompt
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
    {
        parts.push(prompt.to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

pub(crate) async fn load_done(
    conn: &impl ConnectionTrait,
    conversation_id: Uuid,
) -> Result<Vec<CompletionMessage>, StorageError> {
    let rows = query::all(
        conn,
        "SELECT role, content, parts, tool_calls, tool_call_id, thinking
         FROM messages
         WHERE conversation_id = $1 AND status = 'done'
         ORDER BY position",
        vec![Value::from(conversation_id)],
    )
    .await?;
    rows.iter()
        .map(|row| {
            let role_name: String = row.try_get("", "role")?;
            let role = Role::parse(&role_name).unwrap_or(Role::User);
            Ok(CompletionMessage {
                role,
                content: row.try_get("", "content")?,
                parts: optional_json(row, "parts")?,
                tool_calls: tool_calls_from(row)?,
                tool_call_id: row.try_get("", "tool_call_id")?,
                thinking: row.try_get("", "thinking")?,
            })
        })
        .collect()
}

pub(crate) fn provider_values(row: &ProviderRow) -> Vec<Value> {
    vec![
        Value::from(row.id),
        Value::from(row.protocol.as_str()),
        Value::from(row.display_name.as_str()),
        Value::from(row.url.as_str()),
        token_value(&row.token),
        text_opt(&row.default_model),
        Value::from(row.is_default),
        Value::from(row.enabled),
        Value::from(row.user_id),
    ]
}

pub(crate) fn user_from(row: &QueryResult) -> Result<UserAccount, StorageError> {
    Ok(UserAccount {
        id: row.try_get("", "id")?,
        username: row.try_get("", "username")?,
        password_hash: row.try_get("", "password_hash")?,
    })
}

pub(crate) fn provider_from(row: &QueryResult) -> Result<ProviderRow, StorageError> {
    Ok(ProviderRow {
        user_id: row.try_get("", "user_id")?,
        id: row.try_get("", "id")?,
        protocol: row.try_get("", "protocol")?,
        display_name: row.try_get("", "display_name")?,
        url: row.try_get("", "url")?,
        token: blank_to_none(row.try_get("", "token")?),
        default_model: blank_to_none(row.try_get("", "default_model")?),
        is_default: row.try_get("", "is_default")?,
        enabled: row.try_get("", "enabled")?,
    })
}

pub(crate) fn conversation_from(row: &QueryResult) -> Result<Conversation, StorageError> {
    Ok(Conversation {
        id: row.try_get("", "id")?,
        title: row.try_get("", "title")?,
        provider_id: row.try_get("", "provider_id")?,
        model: row.try_get("", "model")?,
        system_prompt: row.try_get("", "system_prompt")?,
        created_at: row.try_get("", "created_at")?,
        updated_at: row.try_get("", "updated_at")?,
        archived_at: row.try_get("", "archived_at")?,
    })
}

pub(crate) fn message_from(row: &QueryResult) -> Result<MessageRow, StorageError> {
    Ok(MessageRow {
        id: row.try_get("", "id")?,
        conversation_id: row.try_get("", "conversation_id")?,
        position: row.try_get("", "position")?,
        role: row.try_get("", "role")?,
        content: row.try_get("", "content")?,
        parts: optional_json(row, "parts")?,
        tool_calls: tool_calls_from(row)?,
        tool_call_id: row.try_get("", "tool_call_id")?,
        thinking: row.try_get("", "thinking")?,
        status: row.try_get("", "status")?,
        error_code: row.try_get("", "error_code")?,
        usage: json_opt(row, "usage")?,
        created_at: row.try_get("", "created_at")?,
        finished_at: row.try_get("", "finished_at")?,
    })
}

pub(crate) fn response_state_from(row: &QueryResult) -> Result<ResponseState, StorageError> {
    Ok(ResponseState {
        id: row.try_get("", "id")?,
        previous_id: row.try_get("", "previous_id")?,
        client_model: row.try_get("", "client_model")?,
        provider_id: row.try_get("", "provider_id")?,
        upstream_model: row.try_get("", "upstream_model")?,
        messages: json_get(row, "messages")?,
        output: json_get(row, "output")?,
        created_at: row.try_get("", "created_at")?,
    })
}

pub(crate) fn tool_calls_from(row: &QueryResult) -> Result<Vec<ToolCall>, StorageError> {
    Ok(json_opt(row, "tool_calls")?.unwrap_or_default())
}

pub(crate) fn optional_json(row: &QueryResult, column: &str) -> Result<Option<Json>, StorageError> {
    Ok(row.try_get(column_prefix(), column)?)
}

pub(crate) fn json_get<T: DeserializeOwned>(
    row: &QueryResult,
    column: &str,
) -> Result<T, StorageError> {
    let value: Json = row.try_get(column_prefix(), column)?;
    serde_json::from_value(value).map_err(json_err)
}

pub(crate) fn json_opt<T: DeserializeOwned>(
    row: &QueryResult,
    column: &str,
) -> Result<Option<T>, StorageError> {
    let Some(value) = optional_json(row, column)? else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(value).map(Some).map_err(json_err)
}

pub(crate) fn column_prefix() -> &'static str {
    ""
}

pub(crate) fn cursor_of(conversation: &Conversation) -> String {
    format!(
        "{}|{}",
        conversation.updated_at.timestamp_micros(),
        conversation.id
    )
}

pub fn parse_cursor(cursor: &str) -> Option<(DateTime<Utc>, Uuid)> {
    let (micros, id) = cursor.split_once('|')?;
    let micros: i64 = micros.parse().ok()?;
    let updated_at = DateTime::from_timestamp_micros(micros)?;
    let id = Uuid::parse_str(id).ok()?;
    Some((updated_at, id))
}

pub fn normalize_protocol(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "openai" => Some("openai"),
        "anthropic" | "antropic" => Some("anthropic"),
        _ => None,
    }
}

pub(crate) fn title_from(content: &str) -> String {
    let line = content.lines().next().unwrap_or("").trim();
    line.chars().take(80).collect()
}

pub(crate) fn with_v1(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    if url.ends_with("/v1") {
        url.to_string()
    } else {
        format!("{url}/v1")
    }
}

pub(crate) fn safe_ident(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

pub(crate) fn blank_to_none(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn text_opt(value: &Option<String>) -> Value {
    Value::from(blank_to_none(value.clone()))
}

pub(crate) fn token_value(token: &Option<String>) -> Value {
    text_opt(token)
}

pub(crate) fn json_err(err: serde_json::Error) -> StorageError {
    StorageError::Db(DbErr::Json(err.to_string()))
}

pub(crate) fn is_unique(err: &DbErr) -> bool {
    matches!(err.sql_err(), Some(SqlErr::UniqueConstraintViolation(_)))
}

pub(crate) fn classify(err: DbErr) -> StorageError {
    match err.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => StorageError::Duplicate,
        Some(SqlErr::ForeignKeyConstraintViolation(_)) => StorageError::InUse,
        _ => StorageError::Db(err),
    }
}

pub(crate) fn statements(sql: &str) -> Vec<String> {
    let mut cleaned = String::new();
    for line in sql.lines() {
        if line.trim_start().starts_with("--") {
            continue;
        }
        cleaned.push_str(line);
        cleaned.push('\n');
    }
    cleaned
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{display_system_prompt, normalize_protocol};
    use crate::SystemPromptLayers;

    #[test]
    fn protocol_alias() {
        assert_eq!(normalize_protocol("OpenAI"), Some("openai"));
        assert_eq!(normalize_protocol("antropic"), Some("anthropic"));
        assert_eq!(normalize_protocol("anthropic"), Some("anthropic"));
        assert_eq!(normalize_protocol("gemini"), None);
    }

    #[test]
    fn system_prompt_layers_keep_server_precedence() {
        let layers = SystemPromptLayers {
            global: Some("global".into()),
            assigned: Some("assigned".into()),
            own: Some("own".into()),
        };
        assert_eq!(layers.effective(), ["global", "assigned", "own"]);
        assert_eq!(
            display_system_prompt(
                &layers,
                Some("conversation"),
                Some("Available tools:\n- `t`")
            ),
            Some("global\n\nassigned\n\nown\n\nconversation\n\nAvailable tools:\n- `t`".into())
        );
        assert_eq!(
            display_system_prompt(&SystemPromptLayers::default(), None, None),
            None
        );
    }
}
