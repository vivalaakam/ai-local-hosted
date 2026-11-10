//! Desktop chat client. The window lives in the `alh-chat` binary.
//! This library holds the pieces that can run without a GPU: local SQLite,
//! the server client, SSE parsing, and the 50 ms text batch.

mod api;
mod batch;
mod feed;
mod runtime;
mod sse;
mod store;

pub use api::{
    Api, ApiError, AuthTokens, ChatDetail, ChatMessage, ImportResult, ProviderDraft,
    ProviderEndpoint,
};
pub use batch::{BATCH_INTERVAL, DeltaBatch, TextBatch};
pub use feed::{Bubble, Feed};
pub use runtime::{Bridge, Command, DesktopSession, Login, UiEvent, spawn_bridge, spawn_session};
pub use sse::{SseBuffer, StreamPiece};
pub use store::{
    ConversationItem, DEFAULT_SERVER_URL, LocalDb, NEW_DRAFT, ServerProfile, SessionCreds,
    StoreError, default_db_path, draft_key,
};
