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
mod query;

pub use error::StorageError;

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
];

impl Store {
    pub async fn connect(database_url: &str, schema: Option<&str>) -> Result<Self, StorageError> {
        let mut options = ConnectOptions::new(database_url);
        options
            .max_connections(8)
            .sqlx_logging(false)
            .record_stmt_in_spans(false);
        if let Some(schema) = schema {
            if !safe_ident(schema) {
                return Err(StorageError::Db(DbErr::Custom(format!(
                    "unsafe schema name: {schema}"
                ))));
            }
            let schema = schema.to_string();
            options.map_sqlx_postgres_pool_opts(move |pool| {
                let schema = schema.clone();
                pool.after_connect(move |conn, _meta| {
                    let schema = schema.clone();
                    Box::pin(async move {
                        let sql = format!(
                            "CREATE SCHEMA IF NOT EXISTS \"{schema}\"; SET search_path TO \"{schema}\""
                        );
                        Executor::execute(conn, sea_orm::sqlx::AssertSqlSafe(sql)).await?;
                        Ok(())
                    })
                })
            });
        }
        let db = Database::connect(options).await?;
        Ok(Self { db })
    }

    pub async fn migrate(&self) -> Result<(), StorageError> {
        self.db
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS alh_migrations (version TEXT PRIMARY KEY)",
            )
            .await?;
        if self.table_exists("providers").await? && !self.migration_applied("0001_init").await? {
            self.record_migration("0001_init").await?;
        }
        let tx = self.db.begin().await?;
        query::exec(&tx, "LOCK TABLE alh_migrations IN EXCLUSIVE MODE", vec![]).await?;
        for (version, sql) in MIGRATIONS {
            if Self::migration_flag(&tx, version).await? {
                continue;
            }
            for statement in statements(sql) {
                tx.execute_unprepared(&statement).await?;
            }
            query::exec(
                &tx,
                "INSERT INTO alh_migrations (version) VALUES ($1)",
                vec![Value::from(*version)],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub fn connection(&self) -> &DatabaseConnection {
        &self.db
    }

    pub async fn begin(&self) -> Result<DatabaseTransaction, StorageError> {
        Ok(self.db.begin().await?)
    }

    pub async fn exec_on(
        conn: &impl ConnectionTrait,
        sql: &str,
        values: Vec<Value>,
    ) -> Result<u64, StorageError> {
        Ok(query::exec(conn, sql, values).await?.rows_affected())
    }

    pub async fn count_on(
        conn: &impl ConnectionTrait,
        sql: &str,
        values: Vec<Value>,
    ) -> Result<i64, StorageError> {
        let Some(row) = query::one(conn, sql, values).await? else {
            return Err(StorageError::NotFound);
        };
        Ok(row.try_get_by_index(0)?)
    }

    pub async fn strings_on(
        conn: &impl ConnectionTrait,
        sql: &str,
        values: Vec<Value>,
    ) -> Result<Vec<String>, StorageError> {
        let rows = query::all(conn, sql, values).await?;
        rows.iter()
            .map(|row| Ok(row.try_get_by_index(0)?))
            .collect()
    }

    pub async fn ping(&self) -> Result<(), StorageError> {
        query::exec(&self.db, "SELECT 1", vec![]).await?;
        Ok(())
    }

    pub async fn current_schema(&self) -> Result<String, StorageError> {
        let Some(row) = query::one(&self.db, "SELECT current_schema() AS schema", vec![]).await?
        else {
            return Err(StorageError::NotFound);
        };
        Ok(row.try_get("", "schema")?)
    }

    pub async fn request_log_text(
        &self,
        user_id: Uuid,
        provider_id: Uuid,
    ) -> Result<Vec<String>, StorageError> {
        Self::strings_on(
            &self.db,
            "SELECT to_jsonb(request_log)::text FROM request_log
             WHERE user_id = $1 AND provider_id = $2",
            vec![Value::from(user_id), Value::from(provider_id)],
        )
        .await
    }

    pub async fn delete_provider_data(
        &self,
        user_id: Uuid,
        provider_id: Uuid,
    ) -> Result<(), StorageError> {
        let user = Value::from(user_id);
        let id = Value::from(provider_id);
        query::exec(
            &self.db,
            "DELETE FROM request_log WHERE user_id = $1 AND provider_id = $2",
            vec![user.clone(), id.clone()],
        )
        .await?;
        query::exec(
            &self.db,
            "DELETE FROM response_states WHERE user_id = $1 AND provider_id = $2",
            vec![user.clone(), id.clone()],
        )
        .await?;
        query::exec(
            &self.db,
            "DELETE FROM conversations WHERE user_id = $1 AND provider_id = $2",
            vec![user, id],
        )
        .await?;
        Ok(())
    }

    pub async fn seed_ollama_if_empty(&self, base_url: &str) -> Result<bool, StorageError> {
        let count = Self::count_on(&self.db, "SELECT COUNT(*) FROM providers", vec![]).await?;
        if count > 0 {
            return Ok(false);
        }
        match query::exec(
            &self.db,
            "INSERT INTO providers (
                id, protocol, display_name, url, token, default_model, is_default, enabled, user_id
             ) VALUES (
                gen_random_uuid(), 'openai', 'Ollama', $1, NULL, NULL, TRUE, TRUE,
                '00000000-0000-0000-0000-000000000001'
             )",
            vec![Value::from(with_v1(base_url))],
        )
        .await
        {
            Ok(_) => Ok(true),
            Err(err) if is_unique(&err) => Ok(false),
            Err(err) => Err(err.into()),
        }
    }

    pub async fn mark_interrupted(&self) -> Result<u64, StorageError> {
        let result = query::exec(
            &self.db,
            "UPDATE messages
             SET status = 'error', error_code = 'interrupted', finished_at = now()
             WHERE status = 'streaming'",
            vec![],
        )
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn purge_response_states(&self, days: i64) -> Result<u64, StorageError> {
        let result = query::exec(
            &self.db,
            "DELETE FROM response_states WHERE created_at < now() - ($1::bigint * interval '1 day')",
            vec![Value::from(days)],
        )
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn register_user(
        &self,
        id: Uuid,
        username: &str,
        password_hash: &str,
    ) -> Result<(), StorageError> {
        let tx = self.db.begin().await?;
        let _locked = query::one(
            &tx,
            "SELECT id FROM users WHERE id = $1 FOR UPDATE",
            vec![Value::from(LEGACY_USER_ID)],
        )
        .await?;
        if let Err(err) = query::exec(
            &tx,
            "INSERT INTO users (id, username, password_hash) VALUES ($1, $2, $3)",
            vec![
                Value::from(id),
                Value::from(username),
                Value::from(password_hash),
            ],
        )
        .await
        {
            return Err(classify(err));
        }
        let real_users = Self::count_on(
            &tx,
            "SELECT COUNT(*) FROM users WHERE id <> $1",
            vec![Value::from(LEGACY_USER_ID)],
        )
        .await?;
        if real_users == 1 {
            let users = vec![Value::from(id), Value::from(LEGACY_USER_ID)];
            query::exec(
                &tx,
                "INSERT INTO providers (
                    user_id, id, protocol, display_name, url, token, default_model, is_default, enabled
                 )
                 SELECT $1, id, protocol, display_name, url, token, default_model, is_default, enabled
                 FROM providers WHERE user_id = $2",
                users.clone(),
            )
            .await?;
            query::exec(
                &tx,
                "UPDATE models SET user_id = $1 WHERE user_id = $2",
                users.clone(),
            )
            .await?;
            query::exec(
                &tx,
                "UPDATE conversations SET user_id = $1 WHERE user_id = $2",
                users.clone(),
            )
            .await?;
            query::exec(
                &tx,
                "UPDATE request_log SET user_id = $1 WHERE user_id = $2",
                users.clone(),
            )
            .await?;
            query::exec(
                &tx,
                "UPDATE response_states SET user_id = $1 WHERE user_id = $2",
                users.clone(),
            )
            .await?;
            query::exec(
                &tx,
                "DELETE FROM providers WHERE user_id = $1",
                vec![Value::from(LEGACY_USER_ID)],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn user_by_name(&self, username: &str) -> Result<Option<UserAccount>, StorageError> {
        let row = query::one(
            &self.db,
            "SELECT id, username, password_hash FROM users WHERE username = $1",
            vec![Value::from(username)],
        )
        .await?;
        row.as_ref().map(user_from).transpose()
    }

    pub async fn system_prompt_layers(
        &self,
        user_id: Uuid,
    ) -> Result<SystemPromptLayers, StorageError> {
        let rows = query::all(
            &self.db,
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

    pub async fn set_own_system_prompt(
        &self,
        user_id: Uuid,
        content: Option<&str>,
    ) -> Result<(), StorageError> {
        let content = content.map(str::trim).filter(|value| !value.is_empty());
        match content {
            Some(content) => {
                query::exec(
                    &self.db,
                    "INSERT INTO system_prompts (user_id, scope, content)
                     VALUES ($1, 'self', $2)
                     ON CONFLICT (user_id, scope) DO UPDATE SET
                        content = EXCLUDED.content, updated_at = now()",
                    vec![Value::from(user_id), Value::from(content)],
                )
                .await?;
            }
            None => {
                query::exec(
                    &self.db,
                    "DELETE FROM system_prompts WHERE user_id = $1 AND scope = 'self'",
                    vec![Value::from(user_id)],
                )
                .await?;
            }
        }
        Ok(())
    }

    pub async fn put_token(
        &self,
        token_hash: &str,
        user_id: Uuid,
        kind: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), StorageError> {
        query::exec(
            &self.db,
            "INSERT INTO oauth_tokens (token_hash, user_id, kind, expires_at)
             VALUES ($1, $2, $3, $4::timestamptz)
             ON CONFLICT (token_hash) DO UPDATE SET
                user_id = EXCLUDED.user_id,
                kind = EXCLUDED.kind,
                expires_at = EXCLUDED.expires_at",
            vec![
                Value::from(token_hash),
                Value::from(user_id),
                Value::from(kind),
                Value::from(expires_at.to_rfc3339()),
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn token_user(
        &self,
        token_hash: &str,
        kind: &str,
    ) -> Result<Option<Uuid>, StorageError> {
        let row = query::one(
            &self.db,
            "SELECT user_id FROM oauth_tokens
             WHERE token_hash = $1 AND kind = $2 AND expires_at > now()",
            vec![Value::from(token_hash), Value::from(kind)],
        )
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(row.try_get("", "user_id")?))
    }

    pub async fn delete_token(&self, token_hash: &str) -> Result<(), StorageError> {
        query::exec(
            &self.db,
            "DELETE FROM oauth_tokens WHERE token_hash = $1",
            vec![Value::from(token_hash)],
        )
        .await?;
        Ok(())
    }

    pub async fn list_providers(&self, user_id: Uuid) -> Result<Vec<ProviderRow>, StorageError> {
        let rows = query::all(
            &self.db,
            "SELECT user_id, id, protocol, display_name, url, token, default_model, is_default, enabled
             FROM providers
             WHERE user_id = $1
             ORDER BY id",
            vec![Value::from(user_id)],
        )
        .await?;
        rows.iter().map(provider_from).collect()
    }

    pub async fn get_provider(
        &self,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<Option<ProviderRow>, StorageError> {
        let row = query::one(
            &self.db,
            "SELECT user_id, id, protocol, display_name, url, token, default_model, is_default, enabled
             FROM providers WHERE user_id = $1 AND id = $2",
            vec![Value::from(user_id), Value::from(id)],
        )
        .await?;
        row.as_ref().map(provider_from).transpose()
    }

    pub async fn upsert_provider(&self, row: &ProviderRow) -> Result<(), StorageError> {
        query::exec(
            &self.db,
            "INSERT INTO providers (
                id, protocol, display_name, url, token, default_model, is_default, enabled, user_id
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (user_id, id) DO UPDATE SET
                protocol = EXCLUDED.protocol,
                display_name = EXCLUDED.display_name,
                url = EXCLUDED.url,
                token = EXCLUDED.token,
                default_model = EXCLUDED.default_model,
                enabled = EXCLUDED.enabled",
            provider_values(row),
        )
        .await?;
        Ok(())
    }

    pub async fn insert_provider(&self, row: &ProviderRow) -> Result<(), StorageError> {
        let tx = self.db.begin().await?;
        let mut values = provider_values(row);
        values[6] = Value::from(false);
        if let Err(err) = query::exec(
            &tx,
            "INSERT INTO providers (
                id, protocol, display_name, url, token, default_model, is_default, enabled, user_id
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            values,
        )
        .await
        {
            return Err(classify(err));
        }
        if row.is_default {
            set_default_on(&tx, row.user_id, row.id).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn update_provider(
        &self,
        user_id: Uuid,
        id: Uuid,
        patch: ProviderUpdate,
    ) -> Result<Option<ProviderRow>, StorageError> {
        let Some(mut row) = self.get_provider(user_id, id).await? else {
            return Ok(None);
        };
        if let Some(protocol) = patch.protocol {
            row.protocol = protocol;
        }
        if let Some(display_name) = patch.display_name {
            row.display_name = display_name;
        }
        if let Some(url) = patch.url {
            row.url = url;
        }
        if let Some(token) = patch.token {
            row.token = blank_to_none(token);
        }
        if let Some(default_model) = patch.default_model {
            row.default_model = blank_to_none(default_model);
        }
        if let Some(enabled) = patch.enabled {
            row.enabled = enabled;
        }
        if patch.is_default == Some(false) {
            row.is_default = false;
        }
        let tx = self.db.begin().await?;
        query::exec(
            &tx,
            "UPDATE providers
             SET protocol = $2, display_name = $3, url = $4, token = $5,
                 default_model = $6, is_default = $7, enabled = $8
             WHERE id = $1 AND user_id = $9",
            provider_values(&row),
        )
        .await?;
        if patch.is_default == Some(true) {
            set_default_on(&tx, user_id, id).await?;
        }
        tx.commit().await?;
        self.get_provider(user_id, id).await
    }

    pub async fn delete_provider(&self, user_id: Uuid, id: Uuid) -> Result<bool, StorageError> {
        match query::exec(
            &self.db,
            "DELETE FROM providers WHERE user_id = $1 AND id = $2",
            vec![Value::from(user_id), Value::from(id)],
        )
        .await
        {
            Ok(result) => Ok(result.rows_affected() > 0),
            Err(err) => Err(classify(err)),
        }
    }

    pub async fn set_default_provider(&self, user_id: Uuid, id: Uuid) -> Result<(), StorageError> {
        let tx = self.db.begin().await?;
        set_default_on(&tx, user_id, id).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn models_fetched_at(
        &self,
        user_id: Uuid,
        provider_id: Uuid,
    ) -> Result<Option<DateTime<Utc>>, StorageError> {
        let Some(row) = query::one(
            &self.db,
            "SELECT MAX(fetched_at) AS fetched_at FROM models
             WHERE user_id = $1 AND provider_id = $2",
            vec![Value::from(user_id), Value::from(provider_id)],
        )
        .await?
        else {
            return Ok(None);
        };
        Ok(row.try_get("", "fetched_at")?)
    }

    pub async fn replace_models(
        &self,
        user_id: Uuid,
        provider_id: Uuid,
        models: &[ModelDescriptor],
    ) -> Result<(), StorageError> {
        let tx = self.db.begin().await?;
        query::exec(
            &tx,
            "DELETE FROM models WHERE user_id = $1 AND provider_id = $2",
            vec![Value::from(user_id), Value::from(provider_id)],
        )
        .await?;
        for model in models {
            query::exec(
                &tx,
                "INSERT INTO models (
                    user_id, provider_id, name, context_window, supports_tools, fetched_at
                 ) VALUES ($1, $2, $3, $4, $5, now())",
                vec![
                    Value::from(user_id),
                    Value::from(provider_id),
                    Value::from(model.name.as_str()),
                    Value::from(model.context_window),
                    Value::from(model.supports_tools),
                ],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn catalog(&self, user_id: Uuid) -> Result<Vec<CatalogModel>, StorageError> {
        let rows = query::all(
            &self.db,
            "SELECT m.provider_id, m.name
             FROM models m
             JOIN providers p ON p.user_id = m.user_id AND p.id = m.provider_id
             WHERE p.enabled AND m.user_id = $1
             ORDER BY m.provider_id, m.name",
            vec![Value::from(user_id)],
        )
        .await?;
        rows.iter()
            .map(|row| {
                let provider_id: Uuid = row.try_get("", "provider_id")?;
                Ok(CatalogModel {
                    provider_slug: provider_id.to_string(),
                    name: row.try_get("", "name")?,
                })
            })
            .collect()
    }

    pub async fn create_conversation(
        &self,
        input: NewConversation,
    ) -> Result<Conversation, StorageError> {
        if self
            .get_provider(input.user_id, input.provider_id)
            .await?
            .is_none()
        {
            return Err(StorageError::NotFound);
        }
        let Some(row) = query::one(
            &self.db,
            "INSERT INTO conversations (id, user_id, title, provider_id, model, system_prompt)
             VALUES ($1, $2, $3, $4, $5, $6)
             RETURNING id, title, provider_id, model, system_prompt, created_at, updated_at, archived_at",
            vec![
                Value::from(input.id),
                Value::from(input.user_id),
                Value::from(input.title),
                Value::from(input.provider_id),
                Value::from(input.model),
                Value::from(input.system_prompt),
            ],
        )
        .await?
        else {
            return Err(StorageError::NotFound);
        };
        conversation_from(&row)
    }

    /// Inserts exported chats for this user. A conversation id that already exists is skipped.
    pub async fn import_chats(
        &self,
        user_id: Uuid,
        provider_id: Uuid,
        chats: &[ImportedChat],
    ) -> Result<ImportCounts, StorageError> {
        if self.get_provider(user_id, provider_id).await?.is_none() {
            return Err(StorageError::NotFound);
        }
        let tx = self.db.begin().await?;
        let mut counts = ImportCounts {
            conversations: 0,
            skipped: 0,
            messages: 0,
        };
        for chat in chats {
            let inserted = query::one(
                &tx,
                "INSERT INTO conversations
                    (id, user_id, title, provider_id, model, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6::timestamptz, $7::timestamptz)
                 ON CONFLICT (id) DO NOTHING
                 RETURNING id",
                vec![
                    Value::from(chat.id),
                    Value::from(user_id),
                    Value::from(chat.title.as_str()),
                    Value::from(provider_id),
                    Value::from(chat.model.as_str()),
                    Value::from(chat.created_at.to_rfc3339()),
                    Value::from(chat.updated_at.to_rfc3339()),
                ],
            )
            .await?;
            if inserted.is_none() {
                counts.skipped += 1;
                continue;
            }
            counts.conversations += 1;
            for (position, message) in chat.messages.iter().enumerate() {
                let position = i32::try_from(position).map_err(|_| {
                    StorageError::Db(DbErr::Custom("conversation has too many messages".into()))
                })?;
                let created = message.created_at.to_rfc3339();
                let row = query::one(
                    &tx,
                    "INSERT INTO messages
                        (id, conversation_id, position, role, content, thinking, status, created_at, finished_at)
                     VALUES ($1, $2, $3, $4, $5, $6, 'done', $7::timestamptz, $7::timestamptz)
                     ON CONFLICT (id) DO NOTHING
                     RETURNING id",
                    vec![
                        Value::from(message.id),
                        Value::from(chat.id),
                        Value::from(position),
                        Value::from(message.role.as_str()),
                        Value::from(message.content.as_str()),
                        Value::from(message.thinking.clone()),
                        Value::from(created.as_str()),
                    ],
                )
                .await?;
                if row.is_some() {
                    counts.messages += 1;
                }
            }
        }
        tx.commit().await?;
        Ok(counts)
    }

    pub async fn list_conversations(
        &self,
        user_id: Uuid,
        cursor: Option<(DateTime<Utc>, Uuid)>,
        limit: i64,
    ) -> Result<ConversationPage, StorageError> {
        let (updated_at, id) = match cursor {
            Some((updated_at, id)) => (Some(updated_at), Some(id)),
            None => (None, None),
        };
        let rows = query::all(
            &self.db,
            "SELECT id, title, provider_id, model, system_prompt, created_at, updated_at, archived_at
             FROM conversations
             WHERE user_id = $1
               AND archived_at IS NULL
               AND (
                    $2::timestamptz IS NULL
                    OR updated_at < $2
                    OR (updated_at = $2 AND id < $3)
               )
             ORDER BY updated_at DESC, id DESC
             LIMIT $4",
            vec![
                Value::from(user_id),
                Value::from(updated_at),
                Value::from(id),
                Value::from(limit + 1),
            ],
        )
        .await?;
        let mut items = rows
            .iter()
            .map(conversation_from)
            .collect::<Result<Vec<_>, _>>()?;
        let next_cursor = if items.len() as i64 > limit {
            items.pop();
            items.last().map(cursor_of)
        } else {
            None
        };
        Ok(ConversationPage { items, next_cursor })
    }

    pub async fn get_conversation(
        &self,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<Option<Conversation>, StorageError> {
        let row = query::one(
            &self.db,
            "SELECT id, title, provider_id, model, system_prompt, created_at, updated_at, archived_at
             FROM conversations WHERE user_id = $1 AND id = $2",
            vec![Value::from(user_id), Value::from(id)],
        )
        .await?;
        row.as_ref().map(conversation_from).transpose()
    }

    pub async fn list_messages(
        &self,
        user_id: Uuid,
        conversation_id: Uuid,
    ) -> Result<Vec<MessageRow>, StorageError> {
        if self
            .get_conversation(user_id, conversation_id)
            .await?
            .is_none()
        {
            return Err(StorageError::NotFound);
        }
        let rows = query::all(
            &self.db,
            "SELECT id, conversation_id, position, role, content, parts, tool_calls, tool_call_id,
                    thinking, status, error_code, usage, created_at, finished_at
             FROM messages
             WHERE conversation_id = $1
             ORDER BY position",
            vec![Value::from(conversation_id)],
        )
        .await?;
        rows.iter().map(message_from).collect()
    }

    pub async fn patch_conversation(
        &self,
        user_id: Uuid,
        id: Uuid,
        patch: ConversationPatch,
    ) -> Result<Option<Conversation>, StorageError> {
        if self.get_conversation(user_id, id).await?.is_none() {
            return Ok(None);
        }
        if let Some(title) = patch.title {
            query::exec(
                &self.db,
                "UPDATE conversations SET title = $2, updated_at = now() WHERE id = $1 AND user_id = $3",
                vec![Value::from(id), Value::from(title), Value::from(user_id)],
            )
            .await?;
        }
        if let Some(system_prompt) = patch.system_prompt {
            query::exec(
                &self.db,
                "UPDATE conversations SET system_prompt = $2, updated_at = now()
                 WHERE id = $1 AND user_id = $3",
                vec![
                    Value::from(id),
                    Value::from(system_prompt),
                    Value::from(user_id),
                ],
            )
            .await?;
        }
        if let Some(model) = patch.model {
            query::exec(
                &self.db,
                "UPDATE conversations SET model = $2, updated_at = now() WHERE id = $1 AND user_id = $3",
                vec![Value::from(id), Value::from(model), Value::from(user_id)],
            )
            .await?;
        }
        if let Some(archived) = patch.archived {
            if archived {
                query::exec(
                    &self.db,
                    "UPDATE conversations SET archived_at = now(), updated_at = now()
                     WHERE id = $1 AND user_id = $2",
                    vec![Value::from(id), Value::from(user_id)],
                )
                .await?;
            } else {
                query::exec(
                    &self.db,
                    "UPDATE conversations SET archived_at = NULL, updated_at = now()
                     WHERE id = $1 AND user_id = $2",
                    vec![Value::from(id), Value::from(user_id)],
                )
                .await?;
            }
        }
        self.get_conversation(user_id, id).await
    }

    pub async fn delete_conversation(&self, user_id: Uuid, id: Uuid) -> Result<bool, StorageError> {
        let result = query::exec(
            &self.db,
            "DELETE FROM conversations WHERE id = $1 AND user_id = $2",
            vec![Value::from(id), Value::from(user_id)],
        )
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn set_conversation_route(
        &self,
        user_id: Uuid,
        id: Uuid,
        provider_id: Uuid,
        model: &str,
    ) -> Result<(), StorageError> {
        if self.get_provider(user_id, provider_id).await?.is_none() {
            return Err(StorageError::NotFound);
        }
        query::exec(
            &self.db,
            "UPDATE conversations SET provider_id = $2, model = $3, updated_at = now()
             WHERE id = $1 AND user_id = $4",
            vec![
                Value::from(id),
                Value::from(provider_id),
                Value::from(model),
                Value::from(user_id),
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn begin_turn(
        &self,
        user_id: Uuid,
        conversation_id: Uuid,
        content: &str,
        model: Option<&str>,
    ) -> Result<BegunTurn, StorageError> {
        let tx = self.db.begin().await?;
        let row = query::one(
            &tx,
            "SELECT title, provider_id, model, system_prompt
             FROM conversations
             WHERE id = $1 AND user_id = $2
             FOR UPDATE",
            vec![Value::from(conversation_id), Value::from(user_id)],
        )
        .await?;
        let Some(row) = row else {
            return Err(StorageError::NotFound);
        };
        let title: String = row.try_get("", "title")?;
        let provider_id: Uuid = row.try_get("", "provider_id")?;
        let mut model_name: String = row.try_get("", "model")?;
        let system_prompt: Option<String> = row.try_get("", "system_prompt")?;
        let Some(streaming_row) = query::one(
            &tx,
            "SELECT EXISTS(
                SELECT 1 FROM messages WHERE conversation_id = $1 AND status = 'streaming'
             ) AS streaming",
            vec![Value::from(conversation_id)],
        )
        .await?
        else {
            return Err(StorageError::NotFound);
        };
        let streaming: bool = streaming_row.try_get("", "streaming")?;
        if streaming {
            return Err(StorageError::Conflict);
        }
        if let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) {
            model_name = model.to_string();
        }
        let Some(next_row) = query::one(
            &tx,
            "SELECT COALESCE(MAX(position), -1) + 1 AS next FROM messages WHERE conversation_id = $1",
            vec![Value::from(conversation_id)],
        )
        .await?
        else {
            return Err(StorageError::NotFound);
        };
        let next: i32 = next_row.try_get("", "next")?;
        let user_message_id = Uuid::new_v4();
        let assistant_id = Uuid::new_v4();
        query::exec(
            &tx,
            "INSERT INTO messages (id, conversation_id, position, role, content, status)
             VALUES ($1, $2, $3, 'user', $4, 'done')",
            vec![
                Value::from(user_message_id),
                Value::from(conversation_id),
                Value::from(next),
                Value::from(content),
            ],
        )
        .await?;
        query::exec(
            &tx,
            "INSERT INTO messages (id, conversation_id, position, role, content, status)
             VALUES ($1, $2, $3, 'assistant', '', 'streaming')",
            vec![
                Value::from(assistant_id),
                Value::from(conversation_id),
                Value::from(next + 1),
            ],
        )
        .await?;
        let new_title = if title.trim().is_empty() {
            let drafted = title_from(content);
            if drafted.is_empty() {
                None
            } else {
                Some(drafted)
            }
        } else {
            None
        };
        query::exec(
            &tx,
            "UPDATE conversations
             SET model = $2, title = COALESCE($3, title), updated_at = now()
             WHERE id = $1",
            vec![
                Value::from(conversation_id),
                Value::from(&model_name),
                Value::from(new_title),
            ],
        )
        .await?;
        let history = load_done(&tx, conversation_id).await?;
        tx.commit().await?;
        Ok(BegunTurn {
            conversation_id,
            user_message_id,
            assistant_message_id: assistant_id,
            provider_id,
            model: model_name,
            system_prompt,
            history,
        })
    }

    pub async fn finalize_message(&self, update: MessageFinal) -> Result<(), StorageError> {
        let tool_calls = if update.tool_calls.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&update.tool_calls).map_err(json_err)?)
        };
        let usage = match &update.usage {
            Some(usage) => Some(serde_json::to_string(usage).map_err(json_err)?),
            None => None,
        };
        query::exec(
            &self.db,
            "UPDATE messages
             SET content = $2, thinking = $3, tool_calls = $4::jsonb, status = $5,
                 error_code = $6, usage = $7::jsonb, finished_at = now()
             WHERE id = $1",
            vec![
                Value::from(update.id),
                Value::from(update.content),
                Value::from(update.thinking),
                Value::from(tool_calls),
                Value::from(update.status),
                Value::from(update.error_code),
                Value::from(usage),
            ],
        )
        .await?;
        query::exec(
            &self.db,
            "UPDATE conversations
             SET updated_at = now()
             WHERE id = (SELECT conversation_id FROM messages WHERE id = $1)",
            vec![Value::from(update.id)],
        )
        .await?;
        Ok(())
    }

    pub async fn insert_request_log(&self, entry: &RequestLogEntry) -> Result<(), StorageError> {
        query::exec(
            &self.db,
            "INSERT INTO request_log (
                id, source, protocol, conversation_id, message_id, provider_id,
                client_model, model, status, error_code, latency_ms, prompt_tokens,
                completion_tokens, user_id
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
            vec![
                Value::from(Uuid::new_v4()),
                Value::from(entry.source.as_str()),
                Value::from(entry.protocol.as_str()),
                Value::from(entry.conversation_id),
                Value::from(entry.message_id),
                Value::from(entry.provider_id),
                Value::from(entry.client_model.as_str()),
                Value::from(entry.model.as_str()),
                Value::from(entry.status.as_str()),
                Value::from(entry.error_code.clone()),
                Value::from(entry.latency_ms),
                Value::from(entry.prompt_tokens),
                Value::from(entry.completion_tokens),
                Value::from(entry.user_id),
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn save_response_state(&self, state: &NewResponseState) -> Result<(), StorageError> {
        query::exec(
            &self.db,
            "INSERT INTO response_states (
                id, previous_id, client_model, provider_id, upstream_model, messages, output, user_id
             ) VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7::jsonb, $8)
             ON CONFLICT (id) DO UPDATE SET
                messages = EXCLUDED.messages,
                output = EXCLUDED.output
             WHERE response_states.user_id = EXCLUDED.user_id",
            vec![
                Value::from(state.id.as_str()),
                Value::from(state.previous_id.clone()),
                Value::from(state.client_model.as_str()),
                Value::from(state.provider_id),
                Value::from(state.upstream_model.as_str()),
                Value::from(serde_json::to_string(&state.messages).map_err(json_err)?),
                Value::from(serde_json::to_string(&state.output).map_err(json_err)?),
                Value::from(state.user_id),
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn get_response_state(
        &self,
        user_id: Uuid,
        id: &str,
    ) -> Result<Option<ResponseState>, StorageError> {
        let row = query::one(
            &self.db,
            "SELECT id, previous_id, client_model, provider_id, upstream_model, messages, output, created_at
             FROM response_states WHERE user_id = $1 AND id = $2",
            vec![Value::from(user_id), Value::from(id)],
        )
        .await?;
        row.as_ref().map(response_state_from).transpose()
    }

    async fn table_exists(&self, table: &str) -> Result<bool, StorageError> {
        let Some(row) = query::one(
            &self.db,
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.tables
                WHERE table_schema = current_schema() AND table_name = $1
             ) AS present",
            vec![Value::from(table)],
        )
        .await?
        else {
            return Ok(false);
        };
        Ok(row.try_get("", "present")?)
    }

    async fn migration_applied(&self, version: &str) -> Result<bool, StorageError> {
        Self::migration_flag(&self.db, version).await
    }

    async fn migration_flag(
        conn: &impl ConnectionTrait,
        version: &str,
    ) -> Result<bool, StorageError> {
        let Some(row) = query::one(
            conn,
            "SELECT EXISTS(SELECT 1 FROM alh_migrations WHERE version = $1) AS applied",
            vec![Value::from(version)],
        )
        .await?
        else {
            return Ok(false);
        };
        Ok(row.try_get("", "applied")?)
    }

    async fn record_migration(&self, version: &str) -> Result<(), StorageError> {
        query::exec(
            &self.db,
            "INSERT INTO alh_migrations (version) VALUES ($1) ON CONFLICT (version) DO NOTHING",
            vec![Value::from(version)],
        )
        .await?;
        Ok(())
    }
}

async fn set_default_on(
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

async fn load_done(
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

fn provider_values(row: &ProviderRow) -> Vec<Value> {
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

fn user_from(row: &QueryResult) -> Result<UserAccount, StorageError> {
    Ok(UserAccount {
        id: row.try_get("", "id")?,
        username: row.try_get("", "username")?,
        password_hash: row.try_get("", "password_hash")?,
    })
}

fn provider_from(row: &QueryResult) -> Result<ProviderRow, StorageError> {
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

fn conversation_from(row: &QueryResult) -> Result<Conversation, StorageError> {
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

fn message_from(row: &QueryResult) -> Result<MessageRow, StorageError> {
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

fn response_state_from(row: &QueryResult) -> Result<ResponseState, StorageError> {
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

fn tool_calls_from(row: &QueryResult) -> Result<Vec<ToolCall>, StorageError> {
    Ok(json_opt(row, "tool_calls")?.unwrap_or_default())
}

fn optional_json(row: &QueryResult, column: &str) -> Result<Option<Json>, StorageError> {
    Ok(row.try_get(column_prefix(), column)?)
}

fn json_get<T: DeserializeOwned>(row: &QueryResult, column: &str) -> Result<T, StorageError> {
    let value: Json = row.try_get(column_prefix(), column)?;
    serde_json::from_value(value).map_err(json_err)
}

fn json_opt<T: DeserializeOwned>(
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

fn column_prefix() -> &'static str {
    ""
}

fn cursor_of(conversation: &Conversation) -> String {
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

fn title_from(content: &str) -> String {
    let line = content.lines().next().unwrap_or("").trim();
    line.chars().take(80).collect()
}

fn with_v1(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    if url.ends_with("/v1") {
        url.to_string()
    } else {
        format!("{url}/v1")
    }
}

fn safe_ident(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn blank_to_none(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn text_opt(value: &Option<String>) -> Value {
    Value::from(blank_to_none(value.clone()))
}

fn token_value(token: &Option<String>) -> Value {
    text_opt(token)
}

fn json_err(err: serde_json::Error) -> StorageError {
    StorageError::Db(DbErr::Json(err.to_string()))
}

fn is_unique(err: &DbErr) -> bool {
    matches!(err.sql_err(), Some(SqlErr::UniqueConstraintViolation(_)))
}

fn classify(err: DbErr) -> StorageError {
    match err.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => StorageError::Duplicate,
        Some(SqlErr::ForeignKeyConstraintViolation(_)) => StorageError::InUse,
        _ => StorageError::Db(err),
    }
}

fn statements(sql: &str) -> Vec<String> {
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
    use super::{SystemPromptLayers, normalize_protocol};

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
    }
}
