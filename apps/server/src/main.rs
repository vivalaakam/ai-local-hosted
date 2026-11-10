use std::path::PathBuf;

use alh_server::{Config, serve};

fn main() {
    let env_file = match load_dotenv() {
        Ok(path) => path,
        Err(err) => {
            eprintln!(".env: {err}");
            std::process::exit(1);
        }
    };
    if let Err(err) = run(env_file) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn load_dotenv() -> Result<Option<PathBuf>, dotenvy::Error> {
    match dotenvy::dotenv() {
        Ok(path) => Ok(Some(path)),
        Err(err) if err.not_found() => Ok(None),
        Err(err) => Err(err),
    }
}

#[tokio::main]
async fn run(env_file: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    if let Some(path) = &env_file {
        tracing::info!(path = %path.display(), "loaded .env");
    }
    let config = Config::from_env().map_err(|err| -> Box<dyn std::error::Error> { err.into() })?;
    serve(config).await?;
    Ok(())
}
