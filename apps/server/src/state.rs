use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alh_core::{
    AliasTable, CatalogModel, ChatProvider, ResolveError, parse_model_ref, resolve_model,
};
use alh_providers::{OpenAiProvider, ProviderConfig};
use alh_storage::{ProviderRow, Store};
use uuid::Uuid;

use crate::error::ApiError;

pub struct AppState {
    pub store: Store,
    pub aliases: AliasTable,
    pub header_timeout: Duration,
    pub idle_timeout: Duration,
    refresh_attempt: Mutex<HashMap<String, Instant>>,
}

impl AppState {
    pub fn new(
        store: Store,
        aliases: AliasTable,
        header_timeout: Duration,
        idle_timeout: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            aliases,
            header_timeout,
            idle_timeout,
            refresh_attempt: Mutex::new(HashMap::new()),
        })
    }
}

pub struct ResolvedRoute {
    pub provider: OpenAiProvider,
    pub provider_id: Uuid,
    pub client_model: String,
    pub upstream_model: String,
}

pub fn apply_provider_header(model: &str, header: Option<&str>) -> String {
    let Some(header) = header.map(str::trim).filter(|header| !header.is_empty()) else {
        return model.trim().to_string();
    };
    let name = parse_model_ref(model).name;
    format!("{header}::{name}")
}

pub async fn resolve_route(
    state: &AppState,
    user_id: Uuid,
    requested: &str,
) -> Result<ResolvedRoute, ApiError> {
    let providers = state
        .store
        .list_providers(user_id)
        .await
        .map_err(ApiError::database)?;
    let enabled: Vec<ProviderRow> = providers
        .into_iter()
        .filter(|provider| provider.enabled)
        .collect();
    let default = enabled
        .iter()
        .find(|provider| provider.is_default)
        .or_else(|| enabled.first())
        .cloned()
        .ok_or_else(|| ApiError::invalid("no provider is configured"))?;
    let parsed = parse_model_ref(requested);
    let slug = parsed
        .provider_slug
        .clone()
        .unwrap_or_else(|| default.id.to_string());
    let row = enabled
        .iter()
        .find(|provider| provider.id.to_string() == slug)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("unknown provider {slug}")))?;
    refresh_models(state, &row).await;
    let catalog = state
        .store
        .catalog(user_id)
        .await
        .map_err(ApiError::database)?;
    let id_strings: Vec<String> = enabled
        .iter()
        .map(|provider| provider.id.to_string())
        .collect();
    let ids: Vec<&str> = id_strings.iter().map(String::as_str).collect();
    let default_id = default.id.to_string();
    let resolved = resolve_model(
        requested,
        &default_id,
        default
            .default_model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty()),
        &ids,
        &catalog,
        &state.aliases,
    )
    .map_err(|err| match err {
        ResolveError::UnknownProvider(slug) => {
            ApiError::not_found(format!("unknown provider {slug}"))
        }
        ResolveError::UnknownModel(model) => ApiError::model_not_found(model),
    })?;
    let row = enabled
        .into_iter()
        .find(|provider| provider.id.to_string() == resolved.provider_slug)
        .ok_or_else(|| {
            ApiError::not_found(format!("unknown provider {}", resolved.provider_slug))
        })?;
    Ok(ResolvedRoute {
        provider: open_provider(state, &row)?,
        provider_id: row.id,
        client_model: resolved.client_model,
        upstream_model: resolved.upstream,
    })
}

pub fn open_provider(state: &AppState, row: &ProviderRow) -> Result<OpenAiProvider, ApiError> {
    match row.protocol.as_str() {
        "openai" => {
            let api_key = row.token.clone().filter(|token| !token.trim().is_empty());
            Ok(OpenAiProvider::new(ProviderConfig {
                id: row.id.to_string(),
                base_url: row.url.clone(),
                api_key,
                header_timeout: state.header_timeout,
                idle_timeout: state.idle_timeout,
            }))
        }
        "anthropic" => Err(ApiError::invalid(
            "anthropic providers are stored but not called yet",
        )),
        _ => Err(ApiError::invalid(format!(
            "unsupported provider protocol {}",
            row.protocol
        ))),
    }
}

async fn refresh_models(state: &AppState, row: &ProviderRow) {
    let fetched = match state.store.models_fetched_at(row.user_id, row.id).await {
        Ok(fetched) => fetched,
        Err(err) => {
            tracing::warn!(%err, provider = %row.id, "model cache lookup failed");
            return;
        }
    };
    if let Some(fetched) = fetched
        && chrono::Utc::now() - fetched < chrono::Duration::minutes(5)
    {
        return;
    }
    {
        let mut attempts = state
            .refresh_attempt
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let key = format!("{}:{}", row.user_id, row.id);
        if let Some(at) = attempts.get(&key)
            && at.elapsed() < Duration::from_secs(30)
        {
            return;
        }
        attempts.insert(key, Instant::now());
    }
    let provider = match open_provider(state, row) {
        Ok(provider) => provider,
        Err(_) => {
            tracing::warn!(
                provider = %row.id,
                protocol = %row.protocol,
                "model refresh skipped"
            );
            return;
        }
    };
    match provider.list_models().await {
        Ok(models) => {
            if let Err(err) = state
                .store
                .replace_models(row.user_id, row.id, &models)
                .await
            {
                tracing::warn!(%err, provider = %row.id, "model cache update failed");
            }
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                detail = err.detail().unwrap_or(""),
                provider = %row.id,
                "model refresh failed"
            );
        }
    }
}

pub async fn model_ids(state: &AppState, user_id: Uuid) -> Result<Vec<String>, ApiError> {
    let providers = state
        .store
        .list_providers(user_id)
        .await
        .map_err(ApiError::database)?;
    let enabled: Vec<_> = providers
        .into_iter()
        .filter(|provider| provider.enabled)
        .collect();
    let default = enabled.iter().find(|provider| provider.is_default);
    for provider in &enabled {
        refresh_models(state, provider).await;
    }
    let catalog: Vec<CatalogModel> = state
        .store
        .catalog(user_id)
        .await
        .map_err(ApiError::database)?;
    let default_id = default.map(|provider| provider.id.to_string());
    let mut ids = Vec::new();
    for model in catalog {
        if Some(model.provider_slug.as_str()) == default_id.as_deref() {
            ids.push(model.name);
        } else {
            ids.push(format!("{}::{}", model.provider_slug, model.name));
        }
    }
    if let Some(default) = default
        && default
            .default_model
            .as_deref()
            .is_some_and(|model| !model.trim().is_empty())
    {
        ids.extend(state.aliases.exact.keys().cloned());
    }
    ids.sort();
    ids.dedup();
    Ok(ids)
}
