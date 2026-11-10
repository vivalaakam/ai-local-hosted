//! Shared types for the gateway. No HTTP, database, or UI dependencies.

mod claude_export;
mod codec;
mod error;
mod message;
mod model;
mod provider;

pub use claude_export::{
    ImportedChat, ImportedMessage, parse_claude_documents, parse_claude_export,
};
pub use codec::{
    AnthropicEncoder, IncomingCompletion, OpenAiEncoder, ResponsesEncoder, ResponsesParsed,
    SseFrame, estimate_request_tokens, parse_anthropic_message, parse_chat_completion,
    parse_responses,
};
pub use error::{ProviderError, ProviderErrorKind, TranslateError};
pub use message::{
    CompletionMessage, FinishReason, Role, TokenUsage, ToolCall, ToolDefinition, estimate_tokens,
    select_context,
};
pub use model::{
    AliasTable, CatalogModel, ModelRef, ResolveError, ResolvedModel, parse_model_ref, resolve_model,
};
pub use provider::{
    BoxFuture, ChatProvider, CompletionAccum, CompletionEvent, CompletionRequest, CompletionStream,
    ModelDescriptor,
};
