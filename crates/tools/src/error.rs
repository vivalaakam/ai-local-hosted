use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolErrorKind {
    NotFound,
    Disabled,
    InvalidArguments,
    Script,
    Http,
    Internal,
}

#[derive(Debug)]
pub struct ToolError {
    pub kind: ToolErrorKind,
    pub message: String,
}

impl ToolError {
    pub fn new(kind: ToolErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ToolErrorKind::NotFound, message)
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ToolErrorKind::InvalidArguments, message)
    }

    pub fn script(message: impl Into<String>) -> Self {
        Self::new(ToolErrorKind::Script, message)
    }

    pub fn http(message: impl Into<String>) -> Self {
        Self::new(ToolErrorKind::Http, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ToolErrorKind::Internal, message)
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.as_str(), self.message)
    }
}

impl std::error::Error for ToolError {}

impl ToolErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Disabled => "disabled",
            Self::InvalidArguments => "invalid_arguments",
            Self::Script => "script",
            Self::Http => "http",
            Self::Internal => "internal",
        }
    }
}
