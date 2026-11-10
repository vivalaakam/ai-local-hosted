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

