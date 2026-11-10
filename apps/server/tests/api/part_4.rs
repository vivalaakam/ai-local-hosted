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

