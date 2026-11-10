//! Local gateway: agent protocols and persisted chat.

mod auth;
mod claude_import;
mod config;
mod error;
mod oauth;
mod providers;
mod routes;
mod state;
mod turn;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use alh_core::AliasTable;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub use auth::token_hash;
pub use config::Config;

use state::AppState;

pub async fn serve(config: Config) -> Result<(), StartupError> {
    let (listener, state) = bind(config).await?;
    let addr = listener.local_addr().map_err(StartupError::bind)?;
    tracing::info!(%addr, "alh-server listening");
    axum::serve(listener, routes::router(state))
        .await
        .map_err(StartupError::bind)
}

pub async fn spawn(config: Config) -> Result<(std::net::SocketAddr, JoinHandle<()>), StartupError> {
    let (listener, state) = bind(config).await?;
    let addr = listener.local_addr().map_err(StartupError::bind)?;
    let handle = tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, routes::router(state)).await {
            tracing::error!(%err, "server stopped");
        }
    });
    Ok((addr, handle))
}

async fn bind(config: Config) -> Result<(TcpListener, Arc<AppState>), StartupError> {
    let state = prepare(&config).await?;
    let listener = TcpListener::bind(config.bind)
        .await
        .map_err(StartupError::bind)?;
    Ok((listener, state))
}

async fn prepare(config: &Config) -> Result<Arc<AppState>, StartupError> {
    let store = alh_storage::Store::connect(&config.database_url, config.schema.as_deref())
        .await
        .map_err(|err| StartupError::message(format!("database: {err}")))?;
    store
        .migrate()
        .await
        .map_err(|err| StartupError::message(format!("migrate: {err}")))?;
    if config.seed {
        store
            .seed_ollama_if_empty(&config.ollama_base_url)
            .await
            .map_err(|err| StartupError::message(format!("database: {err}")))?;
    }
    store
        .mark_interrupted()
        .await
        .map_err(|err| StartupError::message(format!("database: {err}")))?;
    store
        .purge_response_states(7)
        .await
        .map_err(|err| StartupError::message(format!("database: {err}")))?;
    Ok(AppState::new(
        store,
        load_aliases(config.alias_path.as_deref()),
        config.provider_timeout,
        config.stream_idle,
    ))
}

fn load_aliases(path: Option<&Path>) -> AliasTable {
    let path = path.unwrap_or_else(|| Path::new("config/model-aliases.toml"));
    match std::fs::read_to_string(path) {
        Ok(text) => match toml::from_str::<AliasFile>(&text) {
            Ok(file) => AliasTable::from_maps(file.exact, file.prefix),
            Err(err) => {
                tracing::warn!(%err, "alias file is invalid, using built-in prefixes");
                AliasTable::builtin()
            }
        },
        Err(err) => {
            tracing::debug!(%err, path = %path.display(), "alias file not loaded");
            AliasTable::builtin()
        }
    }
}

#[derive(Deserialize)]
struct AliasFile {
    #[serde(default)]
    exact: BTreeMap<String, String>,
    #[serde(default)]
    prefix: BTreeMap<String, String>,
}

#[derive(Debug)]
pub struct StartupError {
    message: String,
}

impl StartupError {
    fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn bind(err: std::io::Error) -> Self {
        Self::message(format!("bind: {err}"))
    }
}

impl std::fmt::Display for StartupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for StartupError {}
