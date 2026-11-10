use serde_json::{Value, json};

use crate::error::TranslateError;
use crate::message::{
    CompletionMessage, Role, ToolCall, ToolDefinition, estimate_tokens, require_str, text_and_parts,
};
use crate::provider::{CompletionAccum, CompletionEvent, CompletionRequest};

include!("codec/part_1.rs");
include!("codec/part_2.rs");
include!("codec/part_3.rs");
include!("codec/part_4.rs");
include!("codec/part_5.rs");
include!("codec/part_6.rs");
