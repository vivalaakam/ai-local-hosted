use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::Duration;

use alh_core::{
    BoxFuture, ChatProvider, CompletionEvent, CompletionRequest, CompletionStream, FinishReason,
    ModelDescriptor, ProviderError, ProviderErrorKind,
};
use futures::StreamExt;
use serde_json::{Value, json};

include!("openai/part_1.rs");
include!("openai/part_2.rs");
