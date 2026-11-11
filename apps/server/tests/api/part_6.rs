#[tokio::test(flavor = "current_thread")]
async fn tools_crud_and_chat_tool_roundtrip() {
    let world = world().await;
    let client = client();

    let listed: Value = client
        .get(format!("{}/api/tools", world.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let slugs: Vec<&str> = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["slug"].as_str().unwrap())
        .collect();
    assert!(slugs.contains(&"current_time"));
    assert!(slugs.contains(&"get_weather"));

    let created: Value = client
        .post(format!("{}/api/tools", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({
            "slug": "echo_msg",
            "name": "Echo",
            "description": "echo args",
            "parameters": {"type":"object","properties":{"msg":{"type":"string"}}},
            "source": "fn run(args) { args.msg }"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(created["slug"], "echo_msg");
    assert_eq!(created["kind"], "rhai");
    assert_eq!(created["model_name"], "user__echo_msg");
    let tool_id = created["id"].as_str().unwrap();

    let patched: Value = client
        .patch(format!("{}/api/tools/{tool_id}", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({"description": "echo message"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(patched["description"], "echo message");

    let conversation: Value = client
        .post(format!("{}/api/conversations", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({"model": "stub-model"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = conversation["id"].as_str().unwrap();
    let stream = client
        .post(format!("{}/api/conversations/{id}/messages", world.base))
        .bearer_auth(TOKEN)
        .json(&json!({"content": "USE_TIME"}))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(stream.contains("[DONE]"));
    assert!(stream.contains("tool-result:"));
    assert!(stream.contains("utc"));

    let detail: Value = client
        .get(format!("{}/api/conversations/{id}", world.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let roles: Vec<&str> = detail["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["role"].as_str().unwrap())
        .collect();
    assert!(roles.contains(&"tool"));
    assert!(roles.iter().filter(|role| **role == "assistant").count() >= 2);

    let deleted = client
        .delete(format!("{}/api/tools/{tool_id}", world.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), 204);

    let system_delete = client
        .delete(format!(
            "{}/api/tools/00000000-0000-0000-0000-000000001001",
            world.base
        ))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(system_delete.status(), 404);
}
