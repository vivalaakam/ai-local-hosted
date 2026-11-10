use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::TranslateError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            "tool" => Some(Self::Tool),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub index: u32,
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: Option<String>,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionMessage {
    pub role: Role,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
}

impl CompletionMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self::text(Role::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::text(Role::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::text(Role::Assistant, content)
    }

    pub fn text(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            parts: None,
            tool_calls: Vec::new(),
            tool_call_id: None,
            thinking: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Other(String),
}

impl FinishReason {
    pub fn openai(&self) -> &str {
        match self {
            Self::Stop => "stop",
            Self::Length => "length",
            Self::ToolCalls => "tool_calls",
            Self::ContentFilter => "content_filter",
            Self::Other(value) => value,
        }
    }

    pub fn anthropic(&self) -> &'static str {
        match self {
            Self::Stop => "end_turn",
            Self::Length => "max_tokens",
            Self::ToolCalls => "tool_use",
            Self::ContentFilter => "refusal",
            Self::Other(_) => "end_turn",
        }
    }

    pub fn from_openai(value: &str) -> Self {
        match value {
            "stop" => Self::Stop,
            "length" => Self::Length,
            "tool_calls" | "function_call" => Self::ToolCalls,
            "content_filter" => Self::ContentFilter,
            other => Self::Other(other.to_string()),
        }
    }
}

/// Keeps the newest messages, without starting on a tool result, and adds the
/// conversation system prompt once.
pub fn select_context(
    system_prompt: Option<&str>,
    history: &[CompletionMessage],
    max_messages: usize,
) -> Vec<CompletionMessage> {
    let mut filtered: Vec<_> = history
        .iter()
        .filter(|message| message.role != Role::System)
        .cloned()
        .collect();
    if max_messages > 0 && filtered.len() > max_messages {
        let mut start = filtered.len() - max_messages;
        while start > 0 && filtered[start].role == Role::Tool {
            start -= 1;
        }
        filtered = filtered.split_off(start);
    } else if max_messages == 0 {
        filtered.clear();
    }

    let mut out = Vec::new();
    if let Some(prompt) = system_prompt
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
    {
        out.push(CompletionMessage::system(prompt));
    }
    out.extend(filtered);
    out
}

pub fn estimate_tokens(messages: &[CompletionMessage], tools: &[ToolDefinition]) -> u32 {
    let mut bytes = 0usize;
    for message in messages {
        bytes += message.content.len();
        bytes += message.thinking.as_deref().map_or(0, str::len);
        if let Some(parts) = &message.parts {
            bytes += parts.to_string().len();
        }
        for call in &message.tool_calls {
            bytes += call.name.len() + call.arguments.len() + call.id.len();
        }
    }
    for tool in tools {
        bytes += tool.name.len();
        bytes += tool.description.as_deref().map_or(0, str::len);
        bytes += tool.parameters.to_string().len();
    }
    u32::try_from(bytes / 4).unwrap_or(u32::MAX)
}

pub(crate) fn require_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, TranslateError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| TranslateError::new(format!("{field} is required")))
}

pub(crate) fn text_and_parts(content: &Value) -> (String, Option<Value>) {
    match content {
        Value::String(text) => (text.clone(), None),
        Value::Array(items) => {
            let mut text = String::new();
            for item in items {
                if let Some(piece) = item.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(piece);
                } else if let Some(piece) = item.get("content").and_then(Value::as_str) {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(piece);
                }
            }
            (text, Some(content.clone()))
        }
        Value::Null => (String::new(), None),
        other => (other.to_string(), Some(other.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_is_included_once() {
        let history = vec![
            CompletionMessage::system("from history"),
            CompletionMessage::user("hello"),
            CompletionMessage::assistant("hi"),
        ];
        let selected = select_context(Some("from conversation"), &history, 20);
        let systems: Vec<_> = selected
            .iter()
            .filter(|message| message.role == Role::System)
            .collect();
        assert_eq!(systems.len(), 1);
        assert_eq!(systems[0].content, "from conversation");
        assert_eq!(selected.len(), 3);
    }

    #[test]
    fn context_does_not_start_on_a_tool_result() {
        let history = vec![
            CompletionMessage::user("one"),
            CompletionMessage::assistant("call"),
            CompletionMessage {
                role: Role::Tool,
                content: "result".into(),
                parts: None,
                tool_calls: Vec::new(),
                tool_call_id: Some("call_1".into()),
                thinking: None,
            },
            CompletionMessage::user("two"),
        ];
        let selected = select_context(None, &history, 2);
        assert_eq!(selected[0].role, Role::Assistant);
        assert_eq!(selected.last().unwrap().content, "two");
    }
}
