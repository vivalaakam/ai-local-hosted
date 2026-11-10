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

/// A server profile owns its URL, OAuth session, drafts, and sidebar cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerProfile {
    pub id: Uuid,
    pub name: String,
    pub url: String,
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
CREATE TABLE IF NOT EXISTS server_profiles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    url TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS profile_sessions (
    profile_id TEXT PRIMARY KEY REFERENCES server_profiles(id) ON DELETE CASCADE,
    username TEXT NOT NULL,
    access_token TEXT NOT NULL,
    refresh_token TEXT NOT NULL,
    access_expires_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS profile_drafts (
    profile_id TEXT NOT NULL REFERENCES server_profiles(id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL,
    body TEXT NOT NULL,
    PRIMARY KEY (profile_id, conversation_id)
);
CREATE TABLE IF NOT EXISTS profile_conversation_cache (
    profile_id TEXT NOT NULL REFERENCES server_profiles(id) ON DELETE CASCADE,
    id TEXT NOT NULL,
    title TEXT NOT NULL,
    model TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    position INTEGER NOT NULL,
    PRIMARY KEY (profile_id, id)
);
";

/// SQLite file for the server URL, unsent drafts, and the sidebar cache.
pub struct LocalDb {
    conn: Connection,
}

