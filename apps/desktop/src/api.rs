use std::fmt::Display;
use std::time::Duration;

use serde::Deserialize;
use uuid::Uuid;

use crate::store::ConversationItem;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub id: Uuid,
    pub role: String,
    pub content: String,
    pub thinking: String,
    pub status: String,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatDetail {
    pub item: ConversationItem,
    pub messages: Vec<ChatMessage>,
}

/// A provider row as the server returns it. The upstream token is not in this struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderEndpoint {
    pub id: Uuid,
    pub protocol: String,
    pub display_name: String,
    pub url: String,
    pub has_token: bool,
    pub is_default: bool,
    pub enabled: bool,
}

/// Fields the settings form sends. `token: None` leaves a stored secret in place on update.
#[derive(Clone)]
pub struct ProviderDraft {
    pub id: Option<Uuid>,
    pub protocol: String,
    pub url: String,
    pub display_name: String,
    pub token: Option<String>,
    pub is_default: bool,
    pub enabled: bool,
    pub update: bool,
    /// The settings form asked for this save and should reset after it succeeds.
    pub reset_form: bool,
}

impl std::fmt::Debug for ProviderDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderDraft")
            .field("id", &self.id)
            .field("protocol", &self.protocol)
            .field("url", &self.url)
            .field("display_name", &self.display_name)
            .field("has_token", &self.token.is_some())
            .field("is_default", &self.is_default)
            .field("enabled", &self.enabled)
            .field("update", &self.update)
            .field("reset_form", &self.reset_form)
            .finish()
    }
}

#[derive(Clone)]
pub struct AuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
}

impl std::fmt::Debug for AuthTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthTokens")
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportResult {
    pub conversations: i64,
    pub skipped: i64,
    pub messages: i64,
}

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: u16,
    pub code: String,
    pub message: String,
}

impl Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

/// HTTP client for this app's own server. It never calls Ollama.
#[derive(Clone)]
pub struct Api {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Api {
    pub fn new(base: impl Into<String>, token: impl Into<String>) -> Result<Self, ApiError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .build()
            .map_err(transport)?;
        Ok(Self {
            http,
            base: base.into().trim_end_matches('/').to_string(),
            token: token.into(),
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn set_base(&mut self, base: impl Into<String>) {
        self.base = base.into().trim_end_matches('/').to_string();
    }

    pub fn set_token(&mut self, token: impl Into<String>) {
        self.token = token.into();
    }

    pub async fn register(&self, username: &str, password: &str) -> Result<AuthTokens, ApiError> {
        let response = self
            .http
            .post(self.url("/oauth/register"))
            .json(&serde_json::json!({
                "username": username,
                "password": password,
            }))
            .send()
            .await
            .map_err(transport)?;
        auth_from(response).await
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<AuthTokens, ApiError> {
        self.token_form(&[
            ("grant_type", "password"),
            ("username", username),
            ("password", password),
        ])
        .await
    }

    pub async fn refresh(&self, refresh_token: &str) -> Result<AuthTokens, ApiError> {
        self.token_form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])
        .await
    }

    async fn token_form(&self, fields: &[(&str, &str)]) -> Result<AuthTokens, ApiError> {
        let response = self
            .http
            .post(self.url("/oauth/token"))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form_body(fields))
            .send()
            .await
            .map_err(transport)?;
        auth_from(response).await
    }

    pub async fn list_conversations(&self) -> Result<Vec<ConversationItem>, ApiError> {
        let mut items = Vec::new();
        let mut cursor = None;
        for _ in 0..500 {
            let mut path = "/api/conversations?limit=100".to_string();
            if let Some(cursor) = cursor.as_deref() {
                path.push_str("&cursor=");
                path.push_str(&query_escape(cursor));
            }
            let body: ListBody<ConversationBody> = self.get_json(&path).await?;
            let next = body.next_cursor.filter(|value| !value.is_empty());
            items.extend(body.data.into_iter().map(ConversationBody::item));
            match next {
                Some(next) if Some(next.as_str()) != cursor.as_deref() => cursor = Some(next),
                _ => break,
            }
        }
        Ok(items)
    }

    pub async fn import_claude(&self, bytes: Vec<u8>) -> Result<ImportResult, ApiError> {
        let response = self
            .http
            .post(self.url("/api/imports/claude"))
            .bearer_auth(&self.token)
            .header("content-type", "application/octet-stream")
            .body(bytes)
            .send()
            .await
            .map_err(transport)?;
        if !response.status().is_success() {
            return Err(error_from(response).await);
        }
        let body: ImportBody = response.json().await.map_err(transport)?;
        Ok(ImportResult {
            conversations: body.conversations,
            skipped: body.skipped,
            messages: body.messages,
        })
    }

    pub async fn list_models(&self) -> Result<Vec<String>, ApiError> {
        let body: ListBody<ModelBody> = self.get_json("/v1/models").await?;
        Ok(body.data.into_iter().map(|model| model.id).collect())
    }

    pub async fn list_providers(&self) -> Result<Vec<ProviderEndpoint>, ApiError> {
        let body: ListBody<ProviderBody> = self.get_json("/api/providers").await?;
        Ok(body.data.into_iter().map(ProviderBody::endpoint).collect())
    }

    pub async fn save_provider(&self, draft: &ProviderDraft) -> Result<ProviderEndpoint, ApiError> {
        let (path, method) = if draft.update {
            let Some(id) = draft.id else {
                return Err(ApiError {
                    status: 0,
                    code: "invalid_request".into(),
                    message: "missing provider id".into(),
                });
            };
            (format!("/api/providers/{id}"), reqwest::Method::PATCH)
        } else {
            ("/api/providers".to_string(), reqwest::Method::POST)
        };
        let body: ProviderBody = self.send_json(method, &path, provider_body(draft)).await?;
        Ok(body.endpoint())
    }

    pub async fn delete_provider(&self, id: Uuid) -> Result<(), ApiError> {
        let response = self
            .http
            .delete(self.url(&format!("/api/providers/{id}")))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(transport)?;
        if response.status().is_success() {
            return Ok(());
        }
        Err(error_from(response).await)
    }

    pub async fn get_conversation(&self, id: Uuid) -> Result<ChatDetail, ApiError> {
        let body: ConversationBody = self.get_json(&format!("/api/conversations/{id}")).await?;
        Ok(body.detail())
    }

    pub async fn create_conversation(&self, model: &str) -> Result<ConversationItem, ApiError> {
        let body: ConversationBody = self
            .send_json(
                reqwest::Method::POST,
                "/api/conversations",
                serde_json::json!({ "model": model }),
            )
            .await?;
        Ok(body.item())
    }

    pub async fn set_model(&self, id: Uuid, model: &str) -> Result<ConversationItem, ApiError> {
        let body: ConversationBody = self
            .send_json(
                reqwest::Method::PATCH,
                &format!("/api/conversations/{id}"),
                serde_json::json!({ "model": model }),
            )
            .await?;
        Ok(body.item())
    }

    pub async fn delete_conversation(&self, id: Uuid) -> Result<(), ApiError> {
        let response = self
            .http
            .delete(self.url(&format!("/api/conversations/{id}")))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(transport)?;
        if response.status().is_success() {
            return Ok(());
        }
        Err(error_from(response).await)
    }

    /// Starts a chat turn. The body is SSE in the chat-completion shape.
    pub async fn send_message(
        &self,
        id: Uuid,
        content: &str,
        model: &str,
    ) -> Result<LiveTurn, ApiError> {
        let response = self
            .http
            .post(self.url(&format!("/api/conversations/{id}/messages")))
            .bearer_auth(&self.token)
            .json(&serde_json::json!({ "content": content, "model": model }))
            .send()
            .await
            .map_err(transport)?;
        if !response.status().is_success() {
            return Err(error_from(response).await);
        }
        let conversation_id = header_uuid(&response, "x-conversation-id")?;
        let user_message_id = header_uuid(&response, "x-user-message-id")?;
        let message_id = header_uuid(&response, "x-message-id")?;
        Ok(LiveTurn {
            conversation_id,
            user_message_id,
            message_id,
            response,
        })
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, ApiError> {
        self.send_json(reqwest::Method::GET, path, serde_json::Value::Null)
            .await
    }

    async fn send_json<T: for<'de> Deserialize<'de>>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: serde_json::Value,
    ) -> Result<T, ApiError> {
        let mut request = self
            .http
            .request(method, self.url(path))
            .bearer_auth(&self.token);
        if !body.is_null() {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(transport)?;
        if !response.status().is_success() {
            return Err(error_from(response).await);
        }
        response.json().await.map_err(transport)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
}

pub struct LiveTurn {
    pub conversation_id: Uuid,
    pub user_message_id: Uuid,
    pub message_id: Uuid,
    response: reqwest::Response,
}

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

#[derive(Deserialize)]
struct MessageBody {
    id: Uuid,
    role: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    error_code: Option<String>,
}
