#[tokio::test(flavor = "current_thread")]
async fn first_chat_message_persists_system_prompt() {
    let world = world().await;
    let client = client();

    let conversation: Value = client
        .post(format!("{}/api/conversations", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({
            "model": "stub-model",
            "system_prompt": "Keep answers short."
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = conversation["id"].as_str().unwrap();
    let first = client
        .post(format!("{}/api/conversations/{id}/messages", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({"content": "hello"}))
        .send()
        .await
        .unwrap();
    assert!(first.headers().get("x-system-message-id").is_some());
    let _ = first.text().await.unwrap();

    let detail: Value = client
        .get(format!("{}/api/conversations/{id}", world.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let messages = detail["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "system");
    let system = messages[0]["content"].as_str().unwrap();
    assert!(system.contains("Keep answers short."));
    assert!(system.contains("Available tools"));
    assert!(system.contains("current_time"));
    assert!(system.contains("get_weather"));
    assert_eq!(messages[1]["role"], "user");
    assert_eq!(messages[2]["role"], "assistant");

    let second = client
        .post(format!("{}/api/conversations/{id}/messages", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({"content": "again"}))
        .send()
        .await
        .unwrap();
    assert!(second.headers().get("x-system-message-id").is_none());
    let _ = second.text().await.unwrap();
}
