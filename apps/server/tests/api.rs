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

fn database_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        panic!(
            "DATABASE_URL is not set. Expected postgres://local_user:local_password@localhost:15432/ai-local-hosted"
        )
    })
}

struct Record {
    last_model: Mutex<String>,
    last_messages: Mutex<String>,
}

struct Stub {
    base: String,
    record: Arc<Record>,
}

async fn stub() -> Stub {
    let record = Arc::new(Record {
        last_model: Mutex::new(String::new()),
        last_messages: Mutex::new(String::new()),
    });
    let app = Router::new()
        .route("/v1/models", axum::routing::get(models))
        .route("/v1/chat/completions", post(completions))
        .with_state(record.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    Stub {
        base: format!("http://{addr}/v1"),
        record,
    }
}

struct World {
    base: String,
    store: Store,
    stub: Stub,
    server: tokio::task::JoinHandle<()>,
    _guard: tokio::sync::MutexGuard<'static, ()>,
}

impl Drop for World {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn world() -> World {
    let guard = lock().await;
    let stub = stub().await;
    let store = Store::connect(&database_url(), Some("alh_test"))
        .await
        .expect("connect");
    store.migrate().await.expect("migrate");
    let schema = store.current_schema().await.unwrap();
    assert_eq!(schema, "alh_test");
    store
        .delete_provider_data(LEGACY_USER_ID, UPSTREAM_ID)
        .await
        .unwrap();
    let access_expires = chrono::Utc::now() + chrono::Duration::days(30);
    store
        .put_token(
            &alh_server::token_hash(TOKEN),
            LEGACY_USER_ID,
            "access",
            access_expires,
        )
        .await
        .unwrap();
    store
        .upsert_provider(&ProviderRow {
            user_id: LEGACY_USER_ID,
            id: UPSTREAM_ID,
            protocol: "openai".into(),
            display_name: "Test".into(),
            url: stub.base.clone(),
            token: None,
            default_model: Some("stub-model".into()),
            is_default: false,
            enabled: true,
        })
        .await
        .unwrap();
    store
        .set_default_provider(LEGACY_USER_ID, UPSTREAM_ID)
        .await
        .unwrap();
    store
        .replace_models(
            LEGACY_USER_ID,
            UPSTREAM_ID,
            &[
                ModelDescriptor {
                    name: "stub-model".into(),
                    context_window: Some(8192),
                    supports_tools: false,
                },
                ModelDescriptor {
                    name: "missing-upstream".into(),
                    context_window: None,
                    supports_tools: false,
                },
            ],
        )
        .await
        .unwrap();
    let config = Config {
        database_url: database_url(),
        bind: "127.0.0.1:0".parse().unwrap(),
        ollama_base_url: "http://127.0.0.1:9".into(),
        provider_timeout: Duration::from_secs(3),
        stream_idle: Duration::from_secs(15),
        schema: Some("alh_test".into()),
        seed: true,
        alias_path: None,
    };
    let (addr, server) = spawn(config).await.expect("spawn");
    World {
        base: format!("http://{addr}"),
        store,
        stub,
        server,
        _guard: guard,
    }
}

async fn lock() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

#[tokio::test(flavor = "current_thread")]
async fn rejects_missing_token_and_reports_health() {
    let world = world().await;
    let client = client();
    let unauthorized = client
        .get(format!("{}/v1/models", world.base))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), 401);
    let body: Value = unauthorized.json().await.unwrap();
    assert_eq!(body["error"]["code"], "unauthorized");

    let health: Value = client
        .get(format!("{}/health", world.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["database"], "ok");
    assert_eq!(health["provider"], "unknown");
    assert_eq!(health["status"], "ok");

    let models: Value = client
        .get(format!("{}/v1/models", world.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<_> = models["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|model| model["id"].as_str())
        .collect();
    assert!(ids.contains(&"stub-model"));
}

#[tokio::test(flavor = "current_thread")]
async fn chat_completions_non_stream_stream_and_tools() {
    let world = world().await;
    let client = client();
    let plain: Value = client
        .post(format!("{}/v1/chat/completions", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({
            "model": "stub-model",
            "messages": [{"role": "user", "content": "SECRET_PROMPT_TEXT"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(plain["model"], "stub-model");
    assert_eq!(plain["choices"][0]["message"]["content"], "ok");
    let logs = world
        .store
        .request_log_text(LEGACY_USER_ID, UPSTREAM_ID)
        .await
        .unwrap();
    assert!(!logs.is_empty());
    assert!(logs.iter().all(|row| !row.contains("SECRET_PROMPT_TEXT")));

    let streamed = client
        .post(format!("{}/v1/chat/completions", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({
            "model": "stub-model",
            "stream": true,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(streamed.contains("[DONE]"));
    assert!(streamed.contains("ok"));

    let tools: Value = client
        .post(format!("{}/v1/chat/completions", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({
            "model": "stub-model",
            "messages": [{"role": "user", "content": "TOOLS"}],
            "tools": [{"type": "function", "function": {"name": "read", "parameters": {"type": "object"}}}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(tools["choices"][0]["message"]["content"], "tools:read");

    let tool_stream = client
        .post(format!("{}/v1/chat/completions", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({
            "model": "stub-model",
            "stream": true,
            "messages": [{"role": "user", "content": "TOOL"}]
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(tool_stream.contains("echo"));
    assert!(tool_stream.contains("[DONE]"));

    let missing = client
        .post(format!("{}/v1/chat/completions", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({
            "model": "missing-upstream",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
    let body: Value = missing.json().await.unwrap();
    assert_eq!(body["error"]["code"], "model_not_found");
}

#[tokio::test(flavor = "current_thread")]
async fn persisted_chat_two_turns_conflict_and_cancel() {
    let world = world().await;
    let client = client();
    let created: Value = client
        .post(format!("{}/api/conversations", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({"model": "stub-model"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap();
    let first = client
        .post(format!("{}/api/conversations/{id}/messages", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({"content": "hello\nthere"}))
        .send()
        .await
        .unwrap();
    assert!(first.headers().get("x-conversation-id").is_some());
    assert!(first.headers().get("x-message-id").is_some());
    let text = first.text().await.unwrap();
    assert!(text.contains("[DONE]"));
    let second = client
        .post(format!("{}/api/conversations/{id}/messages", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({"content": "again"}))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(second.contains("ok"));
    let loaded: Value = client
        .get(format!("{}/api/conversations/{id}", world.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(loaded["title"], "hello");
    assert_eq!(loaded["messages"].as_array().unwrap().len(), 4);
    let listed: Value = client
        .get(format!("{}/api/conversations", world.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let item = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == id)
        .unwrap();
    assert!(item.get("messages").is_none());

    let slow = client
        .post(format!("{}/api/conversations/{id}/messages", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({"content": "SLOW"}))
        .send()
        .await
        .unwrap();
    assert_eq!(slow.status(), 200);
    let conflict = client
        .post(format!("{}/api/conversations/{id}/messages", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({"content": "nope"}))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status(), 409);
    let body: Value = conflict.json().await.unwrap();
    assert_eq!(body["error"]["code"], "conflict");
    drop(slow);
    let mut status = String::new();
    let mut content = String::new();
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let loaded: Value = client
            .get(format!("{}/api/conversations/{id}", world.base))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let assistant = loaded["messages"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|message| message["role"] == "assistant")
            .unwrap();
        status = assistant["status"].as_str().unwrap().to_string();
        content = assistant["content"].as_str().unwrap().to_string();
        if status != "streaming" {
            break;
        }
    }
    assert_eq!(status, "cancelled", "content={content}");
    assert!(content.contains("partial"), "content={content}");
}

#[tokio::test(flavor = "current_thread")]
async fn responses_previous_id_and_anthropic_alias() {
    let world = world().await;
    let client = client();
    let first: Value = client
        .post(format!("{}/v1/responses", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({"model": "stub-model", "input": "one"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = first["id"].as_str().unwrap();
    assert_eq!(first["output_text"], "ok");
    assert!(first["output"].is_array());
    let second = client
        .post(format!("{}/v1/responses", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({
            "model": "stub-model",
            "previous_response_id": id,
            "input": "two",
            "store": false
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 200);
    let messages = world.stub.record.last_messages.lock().unwrap().clone();
    assert!(messages.contains("one"), "{messages}");
    assert!(messages.contains("two"), "{messages}");
    let missing = client
        .post(format!("{}/v1/responses", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({
            "model": "stub-model",
            "previous_response_id": "resp_missing",
            "input": "nope"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);

    let anthropic: Value = client
        .post(format!("{}/v1/messages", world.base))
        .header("x-api-key", TOKEN)
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": "stub-model",
            "max_tokens": 32,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(anthropic["model"], "stub-model");
    assert_eq!(anthropic["content"][0]["text"], "ok");

    let tool = client
        .post(format!("{}/v1/messages", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({
            "model": "stub-model",
            "max_tokens": 32,
            "stream": true,
            "messages": [{"role": "user", "content": "TOOL"}]
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(tool.contains("tool_use"));
    assert!(tool.contains("message_stop"));

    let alias: Value = client
        .post(format!("{}/v1/chat/completions", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(alias["model"], "claude-sonnet-4-5");
    assert_eq!(
        world.stub.record.last_model.lock().unwrap().as_str(),
        "stub-model"
    );

    let counted: Value = client
        .post(format!("{}/v1/messages/count_tokens", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({
            "model": "claude-sonnet-4-5",
            "messages": [{"role": "user", "content": "hello hello"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(counted["input_tokens"].as_u64().unwrap() > 0);
}

#[tokio::test(flavor = "current_thread")]
async fn provider_endpoints_store_token_without_returning_it() {
    let world = world().await;
    let client = client();
    let created = client
        .post(format!("{}/api/providers", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({
            "protocol": "antropic",
            "url": "https://api.anthropic.com",
            "token": "secret-token",
            "display_name": "Claude"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);
    let created: Value = created.json().await.unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    assert!(uuid::Uuid::parse_str(&id).is_ok());
    assert_eq!(created["protocol"], "anthropic");
    assert_eq!(created["has_token"], true);
    assert!(created.get("token").is_none());
    assert!(!created.to_string().contains("secret-token"));

    let listed: Value = client
        .get(format!("{}/api/providers", world.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!listed.to_string().contains("secret-token"));

    world
        .store
        .replace_models(
            LEGACY_USER_ID,
            uuid::Uuid::parse_str(&id).unwrap(),
            &[alh_core::ModelDescriptor {
                name: "claude-x".into(),
                context_window: None,
                supports_tools: false,
            }],
        )
        .await
        .unwrap();
    let rejected = client
        .post(format!("{}/v1/chat/completions", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({
            "model": format!("{id}::claude-x"),
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), 400);
    let rejected: Value = rejected.json().await.unwrap();
    assert_eq!(rejected["error"]["code"], "invalid_request");
    assert!(!rejected.to_string().contains("secret-token"));

    let cleared = client
        .patch(format!("{}/api/providers/{id}", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({"token": null}))
        .send()
        .await
        .unwrap();
    assert_eq!(cleared.status(), 200);
    let cleared: Value = cleared.json().await.unwrap();
    assert_eq!(cleared["has_token"], false);

    let again = client
        .post(format!("{}/api/providers", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({
            "protocol": "openai",
            "url": "http://127.0.0.1:9/v1",
            "display_name": "Claude"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(again.status(), 201);
    let again: Value = again.json().await.unwrap();
    assert_ne!(again["id"], created["id"]);
    let extra = again["id"].as_str().unwrap();
    let removed_extra = client
        .delete(format!("{}/api/providers/{extra}", world.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(removed_extra.status(), 204);

    let removed = client
        .delete(format!("{}/api/providers/{id}", world.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(removed.status(), 204);
}

#[tokio::test(flavor = "current_thread")]
async fn users_own_their_providers_and_sign_in_with_oauth() {
    let world = world().await;
    let client = client();
    let suffix = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let alice = format!("alice{suffix}");
    let bob = format!("bob{suffix}");
    let password = "correct-horse";

    let unknown = token_form(
        &client,
        &world.base,
        &format!("grant_type=password&username=missing{suffix}&password={password}"),
    )
    .await;
    let wrong = token_form(
        &client,
        &world.base,
        &format!("grant_type=password&username={alice}&password=wrong-password"),
    )
    .await;
    assert_eq!(unknown.status(), 400);
    assert_eq!(wrong.status(), 400);
    let unknown: Value = unknown.json().await.unwrap();
    let wrong: Value = wrong.json().await.unwrap();
    assert_eq!(unknown, wrong);
    assert_eq!(unknown["error"]["code"], "invalid_grant");

    let created = client
        .post(format!("{}/oauth/register", world.base))
        .json(&json!({"username": alice, "password": password}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);
    let created: Value = created.json().await.unwrap();
    let alice_access = created["access_token"].as_str().unwrap().to_string();
    let alice_refresh = created["refresh_token"].as_str().unwrap().to_string();
    assert_eq!(created["token_type"], "Bearer");
    assert!(created["expires_in"].as_u64().unwrap() > 0);
    let wrong_existing = token_form(
        &client,
        &world.base,
        &format!("grant_type=password&username={alice}&password=wrong-password"),
    )
    .await;
    assert_eq!(wrong_existing.status(), 400);
    let wrong_existing: Value = wrong_existing.json().await.unwrap();
    assert_eq!(wrong_existing, unknown);

    let provider_name = format!("mine{suffix}");
    let saved = client
        .post(format!("{}/api/providers", world.base))
        .bearer_auth(&alice_access)
        .json(&json!({
            "protocol": "openai",
            "url": "http://127.0.0.1:9/v1",
            "display_name": provider_name
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(saved.status(), 201);
    let saved: Value = saved.json().await.unwrap();
    let provider_id = saved["id"].as_str().unwrap().to_string();
    assert!(uuid::Uuid::parse_str(&provider_id).is_ok());

    let via_key = client
        .get(format!("{}/api/providers", world.base))
        .header("x-api-key", &alice_access)
        .send()
        .await
        .unwrap();
    assert_eq!(via_key.status(), 200);
    let listed: Value = via_key.json().await.unwrap();
    assert!(listed.to_string().contains(&provider_id));

    let legacy = client
        .get(format!("{}/api/providers", world.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert!(!legacy.to_string().contains(&provider_id));

    let bob_created = client
        .post(format!("{}/oauth/register", world.base))
        .json(&json!({"username": bob, "password": password}))
        .send()
        .await
        .unwrap();
    assert_eq!(bob_created.status(), 201);
    let bob_created: Value = bob_created.json().await.unwrap();
    let bob_access = bob_created["access_token"].as_str().unwrap();
    let bob_list: Value = client
        .get(format!("{}/api/providers", world.base))
        .bearer_auth(bob_access)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!bob_list.to_string().contains(&provider_id));

    let refreshed = token_form(
        &client,
        &world.base,
        &format!("grant_type=refresh_token&refresh_token={alice_refresh}"),
    )
    .await;
    assert_eq!(refreshed.status(), 200);
    let refreshed: Value = refreshed.json().await.unwrap();
    let rotated = refreshed["access_token"].as_str().unwrap();
    assert_ne!(rotated, alice_access);
    let reused = token_form(
        &client,
        &world.base,
        &format!("grant_type=refresh_token&refresh_token={alice_refresh}"),
    )
    .await;
    assert_eq!(reused.status(), 400);
    let still: Value = client
        .get(format!("{}/api/providers", world.base))
        .bearer_auth(rotated)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(still.to_string().contains(&provider_id));
}

#[tokio::test(flavor = "current_thread")]
async fn claude_export_imports_the_archive_and_skips_it_the_second_time() {
    let world = world().await;
    let chat_id = uuid::Uuid::parse_str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").unwrap();
    let _ = world
        .store
        .delete_conversation(LEGACY_USER_ID, chat_id)
        .await;
    let raw = br#"[{"uuid":"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa","name":"From Claude","model":"claude-sonnet-4-5","created_at":"2024-06-01T00:00:00Z","updated_at":"2024-06-01T00:05:00Z","chat_messages":[{"uuid":"bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb","sender":"human","text":"Hello from the export","created_at":"2024-06-01T00:00:00Z"},{"uuid":"cccccccc-cccc-cccc-cccc-cccccccccccc","sender":"assistant","created_at":"2024-06-01T00:01:00Z","content":[{"type":"thinking","thinking":"plan"},{"type":"text","text":"Here you go"}]}]}]"#;
    let dir = std::env::temp_dir().join(format!("alh-import-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("conversations.json"), raw).unwrap();
    let status = std::process::Command::new("zip")
        .args(["-q", "export.zip", "conversations.json"])
        .current_dir(&dir)
        .status()
        .unwrap();
    assert!(status.success());
    let bytes = std::fs::read(dir.join("export.zip")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    let client = client();
    let first = client
        .post(format!("{}/api/imports/claude", world.base))
        .bearer_auth(TOKEN)
        .body(bytes.clone())
        .send()
        .await
        .unwrap();
    let first_status = first.status();
    let first_body = first.text().await.unwrap();
    assert_eq!(first_status, 200, "{first_body}");
    let first_body: Value = serde_json::from_str(&first_body).unwrap();
    assert_eq!(first_body["conversations"], 1);
    assert_eq!(first_body["messages"], 2);
    assert_eq!(first_body["skipped"], 0);

    let loaded: Value = client
        .get(format!("{}/api/conversations/{chat_id}", world.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(loaded["title"], "From Claude");
    assert_eq!(loaded["model"], "claude-sonnet-4-5");
    assert_eq!(loaded["provider_id"], UPSTREAM_ID.to_string());
    let messages = loaded["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(messages[0]["content"], "Hello from the export");
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[1]["content"], "Here you go");
    assert_eq!(messages[1]["thinking"], "plan");

    let second = client
        .post(format!("{}/api/imports/claude", world.base))
        .bearer_auth(TOKEN)
        .body(bytes)
        .send()
        .await
        .unwrap();
    let second_status = second.status();
    let second_body = second.text().await.unwrap();
    assert_eq!(second_status, 200, "{second_body}");
    let second_body: Value = serde_json::from_str(&second_body).unwrap();
    assert_eq!(second_body["conversations"], 0);
    assert_eq!(second_body["skipped"], 1);
    assert_eq!(second_body["messages"], 0);

    world
        .store
        .delete_conversation(LEGACY_USER_ID, chat_id)
        .await
        .unwrap();
}

async fn token_form(client: &reqwest::Client, base: &str, body: &str) -> reqwest::Response {
    client
        .post(format!("{base}/oauth/token"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body.to_string())
        .send()
        .await
        .unwrap()
}

async fn models() -> impl IntoResponse {
    Json(json!({"object": "list", "data": [{"id": "stub-model"}, {"id": "missing-upstream"}]}))
}

async fn completions(
    State(stub): State<Arc<Record>>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    if body["stream"] != true {
        return (axum::http::StatusCode::BAD_REQUEST, "stream required").into_response();
    }
    let model = body["model"].as_str().unwrap_or("").to_string();
    let messages = body["messages"]
        .as_array()
        .map(|messages| {
            messages
                .iter()
                .map(|message| {
                    format!(
                        "{}:{}",
                        message["role"].as_str().unwrap_or(""),
                        message["content"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    *stub.last_model.lock().unwrap() = model.clone();
    *stub.last_messages.lock().unwrap() = messages.clone();
    if model == "missing-upstream" {
        return (axum::http::StatusCode::NOT_FOUND, "missing").into_response();
    }
    let content = body["messages"]
        .as_array()
        .and_then(|messages| messages.last())
        .and_then(|message| message["content"].as_str())
        .unwrap_or("");
    match content {
        "SLOW" => hang(),
        "TOOL" => sse(vec![
            chunk(
                json!({"tool_calls": [{
                    "index": 0,
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": "echo", "arguments": "{}"}
                }]}),
                Some("tool_calls"),
            ),
            Event::default().data("[DONE]"),
        ]),
        "TOOLS" => {
            let name = body["tools"][0]["function"]["name"].as_str().unwrap_or("");
            text_response(&format!("tools:{name}"))
        }
        _ => text_response("ok"),
    }
}

fn text_response(text: &str) -> axum::response::Response {
    sse(vec![
        chunk(json!({"content": text}), None),
        chunk(json!({}), Some("stop")),
        Event::default().data("[DONE]"),
    ])
}

fn sse(events: Vec<Event>) -> axum::response::Response {
    Sse::new(futures::stream::iter(
        events.into_iter().map(Ok::<_, std::convert::Infallible>),
    ))
    .into_response()
}

fn hang() -> axum::response::Response {
    Sse::new(futures::stream::unfold(0u8, |step| async move {
        if step == 0 {
            Some((
                Ok::<_, std::convert::Infallible>(chunk(json!({"content": "partial"}), None)),
                1,
            ))
        } else {
            tokio::time::sleep(Duration::from_secs(30)).await;
            None
        }
    }))
    .into_response()
}

fn chunk(delta: Value, finish: Option<&str>) -> Event {
    Event::default().data(
        json!({
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
        })
        .to_string(),
    )
}
