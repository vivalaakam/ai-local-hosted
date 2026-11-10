use std::fmt;

/// Failure from a model provider. `detail` is for server logs only.
#[derive(Debug)]
pub struct ProviderError {
    pub kind: ProviderErrorKind,
    detail: Option<String>,
}

impl ProviderError {
    pub fn new(kind: ProviderErrorKind) -> Self {
        Self { kind, detail: None }
    }

    pub fn with_detail(kind: ProviderErrorKind, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        let detail = if detail.is_empty() {
            None
        } else {
            Some(detail)
        };
        Self { kind, detail }
    }

    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.kind.public_message())
    }
}

impl std::error::Error for ProviderError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorKind {
    Auth,
    NotFound,
    RateLimited,
    Unavailable,
    Timeout,
    Decode,
    Cancelled,
    Invalid,
}

impl ProviderErrorKind {
    pub fn public_message(self) -> &'static str {
        match self {
            Self::Auth => "provider rejected the credentials",
            Self::NotFound => "model not found",
            Self::RateLimited => "provider rate limit",
            Self::Unavailable => "provider unreachable",
            Self::Timeout => "provider timed out",
            Self::Decode => "provider returned an unreadable response",
            Self::Cancelled => "interrupted",
            Self::Invalid => "invalid request",
        }
    }
}

/// Bad client JSON or a field we refuse to translate.
#[derive(Debug)]
pub struct TranslateError {
    pub message: String,
}

impl TranslateError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for TranslateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TranslateError {}
