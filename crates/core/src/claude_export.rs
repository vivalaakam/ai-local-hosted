//! Claude data-export conversations. The bytes are `conversations.json`
//! (one chat array, or one chat object). Zip extraction stays with the caller.

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

include!("claude_export/part_1.rs");
include!("claude_export/part_2.rs");
include!("claude_export/part_3.rs");
