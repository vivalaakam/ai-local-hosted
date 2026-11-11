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

/// A catalog tool from `/api/tools`. System tools have `owner_user_id = None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolEndpoint {
    pub id: Uuid,
    pub owner_user_id: Option<Uuid>,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub parameters: String,
    pub kind: String,
    pub source: Option<String>,
    pub enabled: bool,
    pub model_name: String,
}

impl ToolEndpoint {
    pub fn is_system(&self) -> bool {
        self.owner_user_id.is_none()
    }
}

/// Fields the tool form sends for create/update of user Rhai tools.
#[derive(Debug, Clone)]
pub struct ToolDraft {
    pub id: Option<Uuid>,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub parameters: String,
    pub source: String,
    pub enabled: bool,
    pub update: bool,
    pub reset_form: bool,
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

