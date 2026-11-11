impl LiveTurn {
    pub async fn chunk(&mut self) -> Result<Option<Vec<u8>>, ApiError> {
        Ok(self
            .response
            .chunk()
            .await
            .map_err(transport)?
            .map(|chunk| chunk.to_vec()))
    }
}

fn header_uuid(response: &reqwest::Response, name: &str) -> Result<Uuid, ApiError> {
    let value = response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    Uuid::parse_str(value).map_err(|_| ApiError {
        status: response.status().as_u16(),
        code: "invalid_request".into(),
        message: format!("missing {name}"),
    })
}

async fn auth_from(response: reqwest::Response) -> Result<AuthTokens, ApiError> {
    if !response.status().is_success() {
        return Err(error_from(response).await);
    }
    let body: serde_json::Value = response.json().await.map_err(transport)?;
    let access_token = body["access_token"]
        .as_str()
        .filter(|token| !token.is_empty())
        .ok_or_else(|| ApiError {
            status: 0,
            code: "invalid_request".into(),
            message: "missing access token".into(),
        })?
        .to_string();
    let refresh_token = body["refresh_token"].as_str().unwrap_or("").to_string();
    let expires_in = body["expires_in"].as_i64().unwrap_or(0);
    Ok(AuthTokens {
        access_token,
        refresh_token,
        expires_in,
    })
}

fn form_body(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{key}={}", form_encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn form_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn transport(err: reqwest::Error) -> ApiError {
    ApiError {
        status: 0,
        code: "provider_unreachable".into(),
        message: err.to_string(),
    }
}

async fn error_from(response: reqwest::Response) -> ApiError {
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
        let code = value["error"]["code"]
            .as_str()
            .unwrap_or("upstream")
            .to_string();
        let message = value["error"]["message"]
            .as_str()
            .filter(|message| !message.is_empty())
            .unwrap_or(if text.is_empty() {
                "request failed"
            } else {
                &text
            })
            .to_string();
        return ApiError {
            status,
            code,
            message,
        };
    }
    ApiError {
        status,
        code: "upstream".into(),
        message: if text.is_empty() {
            format!("request failed ({status})")
        } else {
            text
        },
    }
}
#[derive(Deserialize)]
struct ImportBody {
    #[serde(default)]
    conversations: i64,
    #[serde(default)]
    skipped: i64,
    #[serde(default)]
    messages: i64,
}

#[derive(Deserialize)]
struct ListBody<T> {
    data: Vec<T>,
    #[serde(default)]
    next_cursor: Option<String>,
}

fn query_escape(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
#[derive(Deserialize)]
struct ModelBody {
    id: String,
}

#[derive(Deserialize)]
struct ProviderBody {
    id: Uuid,
    #[serde(default)]
    protocol: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    has_token: bool,
    #[serde(default)]
    is_default: bool,
    #[serde(default = "default_enabled")]
    enabled: bool,
}

fn default_enabled() -> bool {
    true
}

impl ProviderBody {
    fn endpoint(self) -> ProviderEndpoint {
        ProviderEndpoint {
            id: self.id,
            protocol: self.protocol,
            display_name: self.display_name,
            url: self.url,
            has_token: self.has_token,
            is_default: self.is_default,
            enabled: self.enabled,
        }
    }
}

fn provider_body(draft: &ProviderDraft) -> serde_json::Value {
    let mut body = serde_json::json!({
        "protocol": draft.protocol,
        "url": draft.url,
        "display_name": draft.display_name.trim(),
        "is_default": draft.is_default,
        "enabled": draft.enabled,
    });
    if let Some(token) = &draft.token {
        body["token"] = serde_json::json!(token);
    }
    body
}

#[derive(Deserialize)]
struct ToolBody {
    id: Uuid,
    #[serde(default)]
    owner_user_id: Option<Uuid>,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    parameters: serde_json::Value,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[serde(default)]
    model_name: String,
}

impl ToolBody {
    fn endpoint(self) -> ToolEndpoint {
        ToolEndpoint {
            id: self.id,
            owner_user_id: self.owner_user_id,
            slug: self.slug,
            name: self.name,
            description: self.description,
            parameters: self.parameters.to_string(),
            kind: self.kind,
            source: self.source,
            enabled: self.enabled,
            model_name: self.model_name,
        }
    }
}

fn tool_body(draft: &ToolDraft) -> serde_json::Value {
    let parameters = serde_json::from_str::<serde_json::Value>(&draft.parameters)
        .unwrap_or_else(|_| serde_json::json!({"type":"object","properties":{}}));
    let mut body = serde_json::json!({
        "name": draft.name.trim(),
        "description": draft.description,
        "parameters": parameters,
        "source": draft.source,
        "enabled": draft.enabled,
    });
    if !draft.update {
        body["slug"] = serde_json::json!(draft.slug.trim());
    }
    body
}

#[derive(Deserialize)]
struct SystemPromptBody {
    #[serde(default)]
    content: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::ProviderBody;

    #[test]
    fn provider_json_does_not_keep_the_upstream_token() {
        let raw = r#"{"id":"11111111-1111-1111-1111-111111111111","protocol":"openai","display_name":"Local","url":"http://127.0.0.1:11434/v1","has_token":true,"token":"secret-token","is_default":true,"enabled":true}"#;
        let endpoint = serde_json::from_str::<ProviderBody>(raw)
            .unwrap()
            .endpoint();
        assert!(endpoint.has_token);
        assert_eq!(endpoint.url, "http://127.0.0.1:11434/v1");
        assert!(!format!("{endpoint:?}").contains("secret-token"));
    }
}

#[derive(Deserialize)]
struct ConversationBody {
    id: Uuid,
    #[serde(default)]
    title: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    messages: Vec<MessageBody>,
}

impl ConversationBody {
    fn item(self) -> ConversationItem {
        ConversationItem {
            id: self.id,
            title: self.title,
            model: self.model,
            updated_at: self.updated_at,
        }
    }

    fn detail(self) -> ChatDetail {
        let messages = self
            .messages
            .iter()
            .map(|message| ChatMessage {
                id: message.id,
                role: message.role.clone(),
                content: message.content.clone(),
                thinking: message.thinking.clone().unwrap_or_default(),
                status: message.status.clone(),
                error_code: message.error_code.clone(),
            })
            .collect();
        ChatDetail {
            item: ConversationItem {
                id: self.id,
                title: self.title,
                model: self.model,
                updated_at: self.updated_at,
            },
            messages,
        }
    }
}
