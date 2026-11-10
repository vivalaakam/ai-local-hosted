use std::cmp::Reverse;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRef {
    pub provider_slug: Option<String>,
    pub name: String,
}

/// `slug::name` selects a provider. A single colon stays inside the model name
/// so Ollama tags such as `qwen2.5-coder:7b` keep working.
pub fn parse_model_ref(raw: &str) -> ModelRef {
    let raw = raw.trim();
    if let Some((slug, name)) = raw.split_once("::") {
        let slug = slug.trim();
        let name = name.trim();
        if !slug.is_empty() && !name.is_empty() {
            return ModelRef {
                provider_slug: Some(slug.to_string()),
                name: name.to_string(),
            };
        }
    }
    ModelRef {
        provider_slug: None,
        name: raw.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogModel {
    pub provider_slug: String,
    pub name: String,
}

#[derive(Debug, Clone, Default)]
pub struct AliasTable {
    pub exact: BTreeMap<String, String>,
    pub prefix: Vec<(String, String)>,
}

impl AliasTable {
    pub fn builtin() -> Self {
        let mut table = Self::default();
        for prefix in ["claude-", "gpt-", "o1", "o3", "o4", "codex-"] {
            table.prefix.push((prefix.to_string(), String::new()));
        }
        table
    }

    pub fn from_maps(exact: BTreeMap<String, String>, prefix: BTreeMap<String, String>) -> Self {
        let mut prefix: Vec<_> = prefix.into_iter().collect();
        prefix.sort_by_key(|pair| Reverse(pair.0.len()));
        Self { exact, prefix }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModel {
    pub client_model: String,
    pub provider_slug: String,
    pub upstream: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    UnknownProvider(String),
    UnknownModel(String),
}

pub fn resolve_model(
    raw: &str,
    default_provider: &str,
    default_model: Option<&str>,
    providers: &[&str],
    catalog: &[CatalogModel],
    aliases: &AliasTable,
) -> Result<ResolvedModel, ResolveError> {
    let parsed = parse_model_ref(raw);
    let client_model = raw.trim().to_string();
    if let Some(slug) = parsed.provider_slug {
        if !providers.contains(&slug.as_str()) {
            return Err(ResolveError::UnknownProvider(slug));
        }
        let listed = catalog
            .iter()
            .any(|model| model.provider_slug == slug && model.name == parsed.name);
        if !listed {
            return Err(ResolveError::UnknownModel(client_model));
        }
        return Ok(ResolvedModel {
            client_model,
            provider_slug: slug,
            upstream: parsed.name,
        });
    }

    if catalog
        .iter()
        .any(|model| model.provider_slug == default_provider && model.name == parsed.name)
    {
        return Ok(ResolvedModel {
            client_model,
            provider_slug: default_provider.to_string(),
            upstream: parsed.name,
        });
    }

    if let Some(target) = aliases.exact.get(raw.trim()) {
        return alias_target(client_model, default_provider, default_model, target);
    }
    if let Some((_, target)) = aliases
        .prefix
        .iter()
        .find(|(prefix, _)| raw.trim().starts_with(prefix.as_str()))
    {
        return alias_target(client_model, default_provider, default_model, target);
    }
    Err(ResolveError::UnknownModel(client_model))
}

fn alias_target(
    client_model: String,
    default_provider: &str,
    default_model: Option<&str>,
    target: &str,
) -> Result<ResolvedModel, ResolveError> {
    let upstream = if target.is_empty() {
        default_model
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .ok_or_else(|| ResolveError::UnknownModel(client_model.clone()))?
            .to_string()
    } else {
        target.to_string()
    };
    Ok(ResolvedModel {
        client_model,
        provider_slug: default_provider.to_string(),
        upstream,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Vec<CatalogModel> {
        vec![CatalogModel {
            provider_slug: "ollama".into(),
            name: "qwen2.5-coder:7b".into(),
        }]
    }

    #[test]
    fn parses_provider_slug_without_splitting_ollama_tags() {
        let parsed = parse_model_ref("ollama::qwen2.5-coder:7b");
        assert_eq!(parsed.provider_slug.as_deref(), Some("ollama"));
        assert_eq!(parsed.name, "qwen2.5-coder:7b");
        assert_eq!(parse_model_ref("qwen2.5-coder:7b").provider_slug, None);
    }

    #[test]
    fn rejects_an_unknown_provider_slug() {
        let error = resolve_model(
            "missing::qwen2.5-coder:7b",
            "ollama",
            Some("qwen2.5-coder:7b"),
            &["ollama"],
            &catalog(),
            &AliasTable::default(),
        )
        .unwrap_err();
        assert_eq!(error, ResolveError::UnknownProvider("missing".into()));
    }

    #[test]
    fn bare_name_uses_the_default_provider() {
        let resolved = resolve_model(
            "qwen2.5-coder:7b",
            "ollama",
            None,
            &["ollama"],
            &catalog(),
            &AliasTable::default(),
        )
        .unwrap();
        assert_eq!(resolved.provider_slug, "ollama");
        assert_eq!(resolved.upstream, "qwen2.5-coder:7b");
    }

    #[test]
    fn claude_alias_uses_the_default_model_and_keeps_the_client_name() {
        let resolved = resolve_model(
            "claude-sonnet-4-5",
            "ollama",
            Some("qwen2.5-coder:7b"),
            &["ollama"],
            &[],
            &AliasTable::builtin(),
        )
        .unwrap();
        assert_eq!(resolved.client_model, "claude-sonnet-4-5");
        assert_eq!(resolved.upstream, "qwen2.5-coder:7b");
    }

    #[test]
    fn alias_without_a_default_model_is_unknown() {
        let error = resolve_model(
            "claude-sonnet-4-5",
            "ollama",
            None,
            &["ollama"],
            &[],
            &AliasTable::builtin(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            ResolveError::UnknownModel("claude-sonnet-4-5".into())
        );
    }
}
