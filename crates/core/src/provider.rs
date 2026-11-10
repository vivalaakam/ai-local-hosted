use std::pin::Pin;

use futures::Future;
use futures::Stream;
use serde::{Deserialize, Serialize};

use crate::error::ProviderError;
use crate::message::{CompletionMessage, FinishReason, TokenUsage, ToolDefinition};

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
pub type CompletionStream =
    Pin<Box<dyn Stream<Item = Result<CompletionEvent, ProviderError>> + Send>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelDescriptor {
    pub name: String,
    pub context_window: Option<i32>,
    pub supports_tools: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<CompletionMessage>,
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
    #[serde(default)]
    pub tool_choice: Option<serde_json::Value>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub stop: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CompletionEvent {
    Delta(String),
    ThinkingDelta(String),
    ToolCallDelta {
        index: u32,
        id: Option<String>,
        name: Option<String>,
        arguments: String,
    },
    Done {
        finish_reason: FinishReason,
        usage: Option<TokenUsage>,
    },
}

pub trait ChatProvider: Send + Sync {
    fn id(&self) -> &str;
    fn list_models(&self) -> BoxFuture<Result<Vec<ModelDescriptor>, ProviderError>>;
    fn stream(&self, request: CompletionRequest) -> CompletionStream;
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CompletionAccum {
    pub text: String,
    pub thinking: String,
    pub tool_calls: Vec<crate::message::ToolCall>,
    pub finish: Option<FinishReason>,
    pub usage: Option<TokenUsage>,
}

impl CompletionAccum {
    pub fn apply(&mut self, event: &CompletionEvent) {
        match event {
            CompletionEvent::Delta(text) => self.text.push_str(text),
            CompletionEvent::ThinkingDelta(text) => self.thinking.push_str(text),
            CompletionEvent::ToolCallDelta {
                index,
                id,
                name,
                arguments,
            } => {
                let position = self.tool_calls.iter().position(|call| call.index == *index);
                if let Some(position) = position {
                    let call = &mut self.tool_calls[position];
                    if let Some(id) = id
                        && !id.is_empty()
                    {
                        call.id = id.clone();
                    }
                    if let Some(name) = name
                        && !name.is_empty()
                    {
                        call.name = name.clone();
                    }
                    call.arguments.push_str(arguments);
                } else {
                    self.tool_calls.push(crate::message::ToolCall {
                        index: *index,
                        id: id.clone().unwrap_or_default(),
                        name: name.clone().unwrap_or_default(),
                        arguments: arguments.clone(),
                    });
                }
            }
            CompletionEvent::Done {
                finish_reason,
                usage,
            } => {
                self.finish = Some(finish_reason.clone());
                if usage.is_some() {
                    self.usage = *usage;
                }
            }
        }
    }

    pub fn finish_reason(&self) -> FinishReason {
        self.finish
            .clone()
            .unwrap_or(if self.tool_calls.is_empty() {
                FinishReason::Stop
            } else {
                FinishReason::ToolCalls
            })
    }
}
