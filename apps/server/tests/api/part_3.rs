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

