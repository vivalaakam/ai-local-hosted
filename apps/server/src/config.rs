use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub bind: SocketAddr,
    pub ollama_base_url: String,
    pub provider_timeout: Duration,
    pub stream_idle: Duration,
    pub schema: Option<String>,
    pub seed: bool,
    pub alias_path: Option<PathBuf>,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let database_url = required("DATABASE_URL")?;
        let bind = std::env::var("BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
        let bind = bind
            .parse()
            .map_err(|_| format!("BIND must be an address like 127.0.0.1:8080, got {bind}"))?;
        Ok(Self {
            database_url,
            bind,
            ollama_base_url: std::env::var("OLLAMA_BASE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:11434".to_string()),
            provider_timeout: duration_ms("PROVIDER_TIMEOUT_MS", 60_000),
            stream_idle: duration_ms("STREAM_IDLE_MS", 120_000),
            schema: None,
            seed: false,
            alias_path: std::env::var("ALIAS_PATH").ok().map(PathBuf::from),
        })
    }
}

fn required(name: &str) -> Result<String, String> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(format!("{name} is required")),
    }
}

fn duration_ms(name: &str, default_ms: u64) -> Duration {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(default_ms))
}
