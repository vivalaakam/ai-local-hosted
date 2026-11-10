use std::fmt::Display;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension};
use uuid::Uuid;

/// Draft key used when no conversation is selected.
pub const NEW_DRAFT: &str = "new";

pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:8080";

include!("store/part_1.rs");
include!("store/part_2.rs");
include!("store/part_3.rs");
