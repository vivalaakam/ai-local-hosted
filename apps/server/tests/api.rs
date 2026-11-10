use std::sync::{Arc, Mutex};
use std::time::Duration;

use alh_core::ModelDescriptor;
use alh_server::{Config, spawn};
use alh_storage::{LEGACY_USER_ID, ProviderRow, Store};
use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::response::sse::{Event, Sse};
use axum::routing::post;
use serde_json::{Value, json};
use tokio::net::TcpListener;

const TOKEN: &str = "test-token";
const UPSTREAM_ID: uuid::Uuid = uuid::Uuid::from_u128(2);

include!("api/part_1.rs");
include!("api/part_2.rs");
include!("api/part_3.rs");
include!("api/part_4.rs");
include!("api/part_5.rs");
