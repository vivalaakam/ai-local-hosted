//! Postgres storage for providers, chats, request metadata, and Responses API state.

use std::fmt;

use alh_core::{
    CatalogModel, CompletionMessage, ImportedChat, ModelDescriptor, Role, TokenUsage, ToolCall,
};
use chrono::{DateTime, Utc};
use sea_orm::sqlx::Executor;
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DatabaseTransaction, DbErr,
    QueryResult, SqlErr, TransactionTrait, Value,
};
use serde::de::DeserializeOwned;
use serde_json::Value as Json;
use uuid::Uuid;

mod error;
mod helpers;
mod query;

pub use error::StorageError;
pub(crate) use helpers::*;
pub use helpers::{normalize_protocol, parse_cursor};

/// Rows that existed before accounts. The first real registration adopts them.
/// The password hash is not a valid argon2 string, so this account cannot sign in.
pub const LEGACY_USER_ID: Uuid = Uuid::from_u128(1);

#[derive(Clone)]
pub struct Store {
    db: DatabaseConnection,
}

#[derive(Clone)]
pub struct UserAccount {
    pub id: Uuid,
    pub username: String,
    pub password_hash: String,
}

impl fmt::Debug for UserAccount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UserAccount")
            .field("id", &self.id)
            .field("username", &self.username)
            .field("password_hash", &"[redacted]")
            .finish()
    }
}

#[derive(Clone)]
pub struct ProviderRow {
    pub user_id: Uuid,
    pub id: Uuid,
    pub protocol: String,
    pub display_name: String,
    pub url: String,
    pub token: Option<String>,
    pub default_model: Option<String>,
    pub is_default: bool,
    pub enabled: bool,
}

impl fmt::Debug for ProviderRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderRow")
            .field("user_id", &self.user_id)
            .field("id", &self.id)
            .field("protocol", &self.protocol)
            .field("display_name", &self.display_name)
            .field("url", &self.url)
            .field("token", &self.token.as_ref().map(|_| "[redacted]"))
            .field("default_model", &self.default_model)
            .field("is_default", &self.is_default)
            .field("enabled", &self.enabled)
            .finish()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProviderUpdate {
    pub protocol: Option<String>,
    pub display_name: Option<String>,
    pub url: Option<String>,
    pub token: Option<Option<String>>,
    pub default_model: Option<Option<String>>,
    pub is_default: Option<bool>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct Conversation {
    pub id: Uuid,
    pub title: String,
    pub provider_id: Uuid,
    pub model: String,
    pub system_prompt: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct MessageRow {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub position: i32,
    pub role: String,
    pub content: String,
    pub parts: Option<Json>,
    pub tool_calls: Vec<ToolCall>,
    pub tool_call_id: Option<String>,
    pub thinking: Option<String>,
    pub status: String,
    pub error_code: Option<String>,
    pub usage: Option<TokenUsage>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportCounts {
    pub conversations: i64,
    pub skipped: i64,
    pub messages: i64,
}

#[derive(Debug, Clone)]
pub struct NewConversation {
    pub user_id: Uuid,
    pub id: Uuid,
    pub title: String,
    pub provider_id: Uuid,
    pub model: String,
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ConversationPatch {
    pub title: Option<String>,
    pub system_prompt: Option<Option<String>>,
    pub model: Option<String>,
    pub archived: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct ConversationPage {
    pub items: Vec<Conversation>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BegunTurn {
    pub conversation_id: Uuid,
    pub user_message_id: Uuid,
    pub assistant_message_id: Uuid,
    pub system_message_id: Option<Uuid>,
    pub provider_id: Uuid,
    pub model: String,
    pub system_prompt: Option<String>,
    pub history: Vec<CompletionMessage>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SystemPromptLayers {
    pub global: Option<String>,
    pub assigned: Option<String>,
    pub own: Option<String>,
}

impl SystemPromptLayers {
    pub fn effective(&self) -> Vec<String> {
        [
            self.global.as_deref(),
            self.assigned.as_deref(),
            self.own.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
        .map(str::to_string)
        .collect()
    }
}

#[derive(Debug, Clone)]
pub struct MessageFinal {
    pub id: Uuid,
    pub status: String,
    pub content: String,
    pub thinking: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub error_code: Option<String>,
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone)]
pub struct RequestLogEntry {
    pub user_id: Uuid,
    pub source: String,
    pub protocol: String,
    pub conversation_id: Option<Uuid>,
    pub message_id: Option<Uuid>,
    pub provider_id: Uuid,
    pub client_model: String,
    pub model: String,
    pub status: String,
    pub error_code: Option<String>,
    pub latency_ms: i32,
    pub prompt_tokens: Option<i32>,
    pub completion_tokens: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct NewResponseState {
    pub user_id: Uuid,
    pub id: String,
    pub previous_id: Option<String>,
    pub client_model: String,
    pub provider_id: Uuid,
    pub upstream_model: String,
    pub messages: Vec<CompletionMessage>,
    pub output: Json,
}

#[derive(Debug, Clone)]
pub struct ResponseState {
    pub id: String,
    pub previous_id: Option<String>,
    pub client_model: String,
    pub provider_id: Uuid,
    pub upstream_model: String,
    pub messages: Vec<CompletionMessage>,
    pub output: Json,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ToolRow {
    pub id: Uuid,
    pub owner_user_id: Option<Uuid>,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub parameters: Json,
    pub kind: String,
    pub source: Option<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewTool {
    pub id: Uuid,
    pub owner_user_id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub parameters: Json,
    pub source: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ToolPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub parameters: Option<Json>,
    pub source: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct ToolResultMessage {
    pub tool_call_id: String,
    pub content: String,
}

const MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001_init",
        include_str!("../../../migrations/0001_init.sql"),
    ),
    (
        "0002_provider_endpoints",
        include_str!("../../../migrations/0002_provider_endpoints.sql"),
    ),
    (
        "0003_users",
        include_str!("../../../migrations/0003_users.sql"),
    ),
    (
        "0004_provider_uuid",
        include_str!("../../../migrations/0004_provider_uuid.sql"),
    ),
    (
        "0005_system_prompts",
        include_str!("../../../migrations/0005_system_prompts.sql"),
    ),
    (
        "0006_tools",
        include_str!("../../../migrations/0006_tools.sql"),
    ),
];

mod store;
