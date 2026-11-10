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
async fn user_can_only_manage_own_system_prompt() {
    let world = world().await;
    let client = client();
    let username = format!("prompt{}", uuid::Uuid::new_v4().simple());
    let registered: Value = client
        .post(format!("{}/oauth/register", world.base))
        .json(&json!({"username": username, "password": "correct-horse"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let access_token = registered["access_token"].as_str().unwrap();
    let put = client
        .put(format!("{}/api/system-prompt", world.base))
        .bearer_auth(access_token)
        .json(&json!({"content": "Keep answers concise."}))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), 204);

    let body: Value = client
        .get(format!("{}/api/system-prompt", world.base))
        .bearer_auth(access_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["content"], "Keep answers concise.");

    let clear = client
        .put(format!("{}/api/system-prompt", world.base))
        .bearer_auth(access_token)
        .json(&json!({"content": null}))
        .send()
        .await
        .unwrap();
    assert_eq!(clear.status(), 204);
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

