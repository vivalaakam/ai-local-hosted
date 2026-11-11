use alh_core::{ToolCall, ToolDefinition};
use serde_json::Value;
use uuid::Uuid;

use crate::error::ToolError;
use crate::http::HttpClient;
use crate::names::{ToolOwner, model_name, parse_model_name};
use crate::native::{self, WeatherEndpoints, call_native};
use crate::rhai_runtime::run_rhai;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Native,
    Rhai,
}

impl ToolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Rhai => "rhai",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "native" => Some(Self::Native),
            "rhai" => Some(Self::Rhai),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub id: Uuid,
    pub owner_user_id: Option<Uuid>,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub kind: ToolKind,
    pub source: Option<String>,
    pub enabled: bool,
}

impl ToolSpec {
    pub fn owner(&self) -> ToolOwner {
        if self.owner_user_id.is_some() {
            ToolOwner::User
        } else {
            ToolOwner::System
        }
    }

    pub fn model_tool_name(&self) -> String {
        model_name(self.owner(), &self.slug)
    }

    pub fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.model_tool_name(),
            description: Some(self.description.clone()),
            parameters: self.parameters.clone(),
        }
    }
}

#[derive(Clone)]
pub struct ToolExecutor {
    http: HttpClient,
    weather: WeatherEndpoints,
}

impl ToolExecutor {
    pub fn new() -> Result<Self, ToolError> {
        Ok(Self {
            http: HttpClient::new()?,
            weather: WeatherEndpoints::default(),
        })
    }

    pub fn with_weather_endpoints(mut self, weather: WeatherEndpoints) -> Self {
        self.weather = weather;
        self
    }

    pub fn http(&self) -> &HttpClient {
        &self.http
    }

    pub async fn execute_call(
        &self,
        specs: &[ToolSpec],
        call: &ToolCall,
    ) -> Result<String, ToolError> {
        let (owner, slug) = parse_model_name(&call.name);
        let spec = specs
            .iter()
            .find(|spec| spec.owner() == owner && spec.slug == slug)
            .ok_or_else(|| ToolError::not_found(format!("unknown tool {}", call.name)))?;
        if !spec.enabled {
            return Err(ToolError::new(
                crate::error::ToolErrorKind::Disabled,
                format!("tool {} is disabled", call.name),
            ));
        }
        match spec.kind {
            ToolKind::Native => {
                if owner != ToolOwner::System {
                    return Err(ToolError::invalid("native tools must be system-owned"));
                }
                call_native(slug, &call.arguments, &self.http, &self.weather).await
            }
            ToolKind::Rhai => {
                let source = spec
                    .source
                    .as_deref()
                    .map(str::trim)
                    .filter(|source| !source.is_empty())
                    .ok_or_else(|| ToolError::script("rhai source is empty"))?
                    .to_string();
                let arguments = call.arguments.clone();
                let http = self.http.clone();
                tokio::task::spawn_blocking(move || run_rhai(&source, &arguments, http))
                    .await
                    .map_err(|err| ToolError::internal(format!("rhai join: {err}")))?
            }
        }
    }

    pub async fn execute_calls(
        &self,
        specs: &[ToolSpec],
        calls: &[ToolCall],
    ) -> Vec<(ToolCall, Result<String, ToolError>)> {
        let mut out = Vec::with_capacity(calls.len());
        for call in calls {
            let result = self.execute_call(specs, call).await;
            out.push((call.clone(), result));
        }
        out
    }
}

impl Default for ToolExecutor {
    fn default() -> Self {
        Self::new().expect("tool executor")
    }
}

pub fn definitions_for(specs: &[ToolSpec]) -> Vec<ToolDefinition> {
    specs
        .iter()
        .filter(|spec| spec.enabled)
        .map(ToolSpec::definition)
        .collect()
}

/// Text block for the system prompt listing enabled tools.
pub fn catalog_prompt(specs: &[ToolSpec]) -> Option<String> {
    let mut lines = Vec::new();
    for spec in specs.iter().filter(|spec| spec.enabled) {
        let name = spec.model_tool_name();
        let description = spec.description.trim();
        if description.is_empty() {
            lines.push(format!("- `{name}`"));
        } else {
            lines.push(format!("- `{name}`: {description}"));
        }
    }
    if lines.is_empty() {
        None
    } else {
        let mut out = String::from(
            "Available tools (call them through the provider tool interface when needed):\n",
        );
        out.push_str(&lines.join("\n"));
        Some(out)
    }
}

pub fn seeded_native_specs() -> Vec<ToolSpec> {
    native::native_manifests()
        .into_iter()
        .map(|manifest| ToolSpec {
            id: manifest.id,
            owner_user_id: None,
            slug: manifest.slug.to_string(),
            name: manifest.name.to_string(),
            description: manifest.description.to_string(),
            parameters: manifest.parameters,
            kind: ToolKind::Native,
            source: None,
            enabled: true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_prompt_lists_enabled_tools() {
        let prompt = catalog_prompt(&[
            ToolSpec {
                id: Uuid::nil(),
                owner_user_id: None,
                slug: "current_time".into(),
                name: "Current time".into(),
                description: "UTC now".into(),
                parameters: json!({}),
                kind: ToolKind::Native,
                source: None,
                enabled: true,
            },
            ToolSpec {
                id: Uuid::nil(),
                owner_user_id: Some(Uuid::nil()),
                slug: "ping".into(),
                name: "Ping".into(),
                description: "echo".into(),
                parameters: json!({}),
                kind: ToolKind::Rhai,
                source: Some("fn run(args) { \"\" }".into()),
                enabled: false,
            },
        ])
        .unwrap();
        assert!(prompt.contains("Available tools"));
        assert!(prompt.contains("`current_time`: UTC now"));
        assert!(!prompt.contains("ping"));
    }
}
