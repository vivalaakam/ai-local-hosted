use std::fmt::Display;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension};
use uuid::Uuid;

/// Draft key used when no conversation is selected.
pub const NEW_DRAFT: &str = "new";

pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:8080";

/// Desktop OAuth session. Provider tokens stay on the server.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionCreds {
    pub username: String,
    pub access_token: String,
    pub refresh_token: String,
    pub access_expires_at: i64,
}

impl std::fmt::Debug for SessionCreds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionCreds")
            .field("username", &self.username)
            .field("access_expires_at", &self.access_expires_at)
            .finish()
    }
}

/// A conversation row cached for the sidebar. The server remains the source of messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationItem {
    pub id: Uuid,
    pub title: String,
    pub model: String,
    pub updated_at: String,
}

#[derive(Debug)]
pub struct StoreError {
    message: String,
}

impl Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> Self {
        Self {
            message: err.to_string(),
        }
    }
}

impl From<std::io::Error> for StoreError {
    fn from(err: std::io::Error) -> Self {
        Self {
            message: err.to_string(),
        }
    }
}

pub fn draft_key(conversation: Option<Uuid>) -> String {
    conversation
        .map(|id| id.to_string())
        .unwrap_or_else(|| NEW_DRAFT.to_string())
}

pub fn default_db_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join("Library/Application Support/alh-chat")
        .join("local.sqlite")
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS drafts (
    conversation_id TEXT PRIMARY KEY,
    body TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS conversation_cache (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    model TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    position INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS session (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    username TEXT NOT NULL,
    access_token TEXT NOT NULL,
    refresh_token TEXT NOT NULL,
    access_expires_at INTEGER NOT NULL
);
";

/// SQLite file for the server URL, unsent drafts, and the sidebar cache.
pub struct LocalDb {
    conn: Connection,
}

impl LocalDb {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::from_conn(Connection::open(path)?)
    }

    pub fn open_memory() -> Result<Self, StoreError> {
        Self::from_conn(Connection::open_in_memory()?)
    }

    fn from_conn(conn: Connection) -> Result<Self, StoreError> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn server_url(&self) -> Result<String, StoreError> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'server_url'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.unwrap_or_else(|| DEFAULT_SERVER_URL.to_string()))
    }

    pub fn set_server_url(&self, url: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES ('server_url', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [url.trim()],
        )?;
        Ok(())
    }

    pub fn session(&self) -> Result<Option<SessionCreds>, StoreError> {
        self.conn
            .query_row(
                "SELECT username, access_token, refresh_token, access_expires_at
                 FROM session WHERE id = 1",
                [],
                |row| {
                    Ok(SessionCreds {
                        username: row.get(0)?,
                        access_token: row.get(1)?,
                        refresh_token: row.get(2)?,
                        access_expires_at: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn set_session(&self, session: &SessionCreds) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO session (id, username, access_token, refresh_token, access_expires_at)
             VALUES (1, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                username = excluded.username,
                access_token = excluded.access_token,
                refresh_token = excluded.refresh_token,
                access_expires_at = excluded.access_expires_at",
            rusqlite::params![
                session.username,
                session.access_token,
                session.refresh_token,
                session.access_expires_at
            ],
        )?;
        Ok(())
    }

    pub fn clear_session(&self) -> Result<(), StoreError> {
        self.conn.execute("DELETE FROM session WHERE id = 1", [])?;
        Ok(())
    }

    pub fn draft(&self, conversation_id: &str) -> Result<String, StoreError> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT body FROM drafts WHERE conversation_id = ?1",
                [conversation_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.unwrap_or_default())
    }

    pub fn set_draft(&self, conversation_id: &str, body: &str) -> Result<(), StoreError> {
        if body.is_empty() {
            return self.clear_draft(conversation_id);
        }
        self.conn.execute(
            "INSERT INTO drafts (conversation_id, body) VALUES (?1, ?2)
             ON CONFLICT(conversation_id) DO UPDATE SET body = excluded.body",
            [conversation_id, body],
        )?;
        Ok(())
    }

    pub fn clear_draft(&self, conversation_id: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM drafts WHERE conversation_id = ?1",
            [conversation_id],
        )?;
        Ok(())
    }

    pub fn cached_conversations(&self) -> Result<Vec<ConversationItem>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT id, title, model, updated_at FROM conversation_cache ORDER BY position",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ConversationItem {
                id: parse_uuid(&row.get::<_, String>(0)?),
                title: row.get(1)?,
                model: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn replace_cache(&self, items: &[ConversationItem]) -> Result<(), StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM conversation_cache", [])?;
        for (position, item) in items.iter().enumerate() {
            tx.execute(
                "INSERT INTO conversation_cache (id, title, model, updated_at, position)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    item.id.to_string(),
                    item.title,
                    item.model,
                    item.updated_at,
                    position as i64
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

fn parse_uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).unwrap_or_else(|_| Uuid::nil())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_draft_and_cache_roundtrip() {
        let db = LocalDb::open_memory().unwrap();
        assert_eq!(db.server_url().unwrap(), DEFAULT_SERVER_URL);
        db.set_server_url("http://127.0.0.1:9").unwrap();
        assert_eq!(db.server_url().unwrap(), "http://127.0.0.1:9");

        let id = Uuid::parse_str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").unwrap();
        assert!(db.draft(&draft_key(Some(id))).unwrap().is_empty());
        db.set_draft(&draft_key(Some(id)), "unsent").unwrap();
        assert_eq!(db.draft(&draft_key(Some(id))).unwrap(), "unsent");
        db.clear_draft(&draft_key(Some(id))).unwrap();
        assert!(db.draft(&draft_key(Some(id))).unwrap().is_empty());

        db.replace_cache(&[ConversationItem {
            id,
            title: "hello".into(),
            model: "qwen".into(),
            updated_at: "2026-11-08T00:00:00Z".into(),
        }])
        .unwrap();
        let cached = db.cached_conversations().unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].title, "hello");
        db.replace_cache(&[]).unwrap();
        assert!(db.cached_conversations().unwrap().is_empty());

        assert!(db.session().unwrap().is_none());
        db.set_session(&SessionCreds {
            username: "ada".into(),
            access_token: "sekret-token".into(),
            refresh_token: "sekret-refresh".into(),
            access_expires_at: 10,
        })
        .unwrap();
        let saved = db.session().unwrap().unwrap();
        assert_eq!(saved.username, "ada");
        assert_eq!(saved.access_token, "sekret-token");
        assert!(!format!("{saved:?}").contains("sekret"));
        db.clear_session().unwrap();
        assert!(db.session().unwrap().is_none());
    }
}
