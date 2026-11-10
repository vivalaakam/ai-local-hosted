use alh_core::{CompletionMessage, ModelDescriptor};
use alh_storage::{
    ConversationPatch, LEGACY_USER_ID, MessageFinal, NewConversation, NewResponseState,
    ProviderRow, ProviderUpdate, RequestLogEntry, StorageError, Store,
};
use sea_orm::Value;
use uuid::Uuid;

fn database_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        panic!(
            "DATABASE_URL is not set. Expected postgres://local_user:local_password@localhost:15432/ai-local-hosted"
        )
    })
}

async fn store() -> Store {
    let store = Store::connect(&database_url(), Some("alh_storage_test"))
        .await
        .expect("connect");
    store.migrate().await.expect("migrate");
    let schema = store.current_schema().await.unwrap();
    assert_eq!(schema, "alh_storage_test");
    store
}

#[tokio::test]
async fn roundtrip_rolls_back() {
    let store = store().await;
    let tx = store.begin().await.unwrap();
    let id = Uuid::new_v4();
    Store::exec_on(
        &tx,
        "INSERT INTO providers (id, protocol, display_name, url, token, is_default, enabled, user_id)
         VALUES ($1, 'openai', 'rollback', 'http://127.0.0.1:9/v1', NULL, FALSE, TRUE, $2)",
        vec![Value::from(id), Value::from(LEGACY_USER_ID)],
    )
    .await
    .unwrap();
    let found = Store::count_on(
        &tx,
        "SELECT COUNT(*) FROM providers WHERE id = $1",
        vec![Value::from(id)],
    )
    .await
    .unwrap();
    assert_eq!(found, 1);
    tx.rollback().await.unwrap();
    let found = Store::count_on(
        store.connection(),
        "SELECT COUNT(*) FROM providers WHERE id = $1",
        vec![Value::from(id)],
    )
    .await
    .unwrap();
    assert_eq!(found, 0);
}

#[tokio::test]
async fn chat_turn_conflict_and_response_state() {
    let store = store().await;
    let slug = Uuid::new_v4();
    store
        .upsert_provider(&ProviderRow {
            user_id: LEGACY_USER_ID,
            id: slug,
            protocol: "openai".into(),
            display_name: "turn".into(),
            url: "http://127.0.0.1:9/v1".into(),
            token: None,
            default_model: Some("stub".into()),
            is_default: false,
            enabled: true,
        })
        .await
        .unwrap();
    store
        .replace_models(
            LEGACY_USER_ID,
            slug,
            &[ModelDescriptor {
                name: "stub".into(),
                context_window: Some(1024),
                supports_tools: false,
            }],
        )
        .await
        .unwrap();
    let conversation = store
        .create_conversation(NewConversation {
            user_id: LEGACY_USER_ID,
            id: Uuid::new_v4(),
            title: String::new(),
            provider_id: slug,
            model: "stub".into(),
            system_prompt: Some("be brief".into()),
        })
        .await
        .unwrap();
    let turn = store
        .begin_turn(LEGACY_USER_ID, conversation.id, "hello\nmore", None)
        .await
        .unwrap();
    assert_eq!(turn.history.len(), 1);
    assert_eq!(turn.history[0].content, "hello\nmore");
    assert_eq!(turn.system_prompt.as_deref(), Some("be brief"));
    let saved = store
        .get_conversation(LEGACY_USER_ID, conversation.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.title, "hello");
    let conflict = store
        .begin_turn(LEGACY_USER_ID, conversation.id, "second", None)
        .await;
    assert!(matches!(conflict, Err(StorageError::Conflict)));
    store
        .finalize_message(MessageFinal {
            id: turn.assistant_message_id,
            status: "done".into(),
            content: "hi".into(),
            thinking: None,
            tool_calls: Vec::new(),
            error_code: None,
            usage: None,
        })
        .await
        .unwrap();
    let again = store
        .begin_turn(LEGACY_USER_ID, conversation.id, "second", Some("other"))
        .await
        .unwrap();
    assert_eq!(again.history.len(), 3);
    assert_eq!(again.model, "other");
    store
        .insert_request_log(&RequestLogEntry {
            user_id: LEGACY_USER_ID,
            source: "chat".into(),
            protocol: "chat".into(),
            conversation_id: Some(conversation.id),
            message_id: Some(again.assistant_message_id),
            provider_id: slug,
            client_model: "other".into(),
            model: "stub".into(),
            status: "done".into(),
            error_code: None,
            latency_ms: 4,
            prompt_tokens: Some(1),
            completion_tokens: Some(1),
        })
        .await
        .unwrap();
    let columns = Store::strings_on(
        store.connection(),
        "SELECT column_name FROM information_schema.columns
         WHERE table_schema = current_schema() AND table_name = 'request_log'",
        vec![],
    )
    .await
    .unwrap();
    assert!(
        !columns
            .iter()
            .any(|column| column == "content" || column == "prompt")
    );
    let state_id = format!("resp_{}", Uuid::new_v4().simple());
    store
        .save_response_state(&NewResponseState {
            user_id: LEGACY_USER_ID,
            id: state_id.clone(),
            previous_id: None,
            client_model: "gpt-test".into(),
            provider_id: slug,
            upstream_model: "stub".into(),
            messages: vec![CompletionMessage::user("hello")],
            output: serde_json::json!({"output_text": "hi"}),
        })
        .await
        .unwrap();
    let loaded = store
        .get_response_state(LEGACY_USER_ID, &state_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.messages[0].content, "hello");
    store
        .patch_conversation(
            LEGACY_USER_ID,
            conversation.id,
            ConversationPatch {
                archived: Some(true),
                ..ConversationPatch::default()
            },
        )
        .await
        .unwrap();
    let page = store
        .list_conversations(LEGACY_USER_ID, None, 50)
        .await
        .unwrap();
    assert!(page.items.iter().all(|item| item.id != conversation.id));
    assert!(
        store
            .delete_conversation(LEGACY_USER_ID, conversation.id)
            .await
            .unwrap()
    );
    store
        .delete_provider_data(LEGACY_USER_ID, slug)
        .await
        .unwrap();
    assert!(store.delete_provider(LEGACY_USER_ID, slug).await.unwrap());
}

#[tokio::test]
async fn seed_runs_once() {
    let store = store().await;
    let first = store
        .seed_ollama_if_empty("http://127.0.0.1:11434")
        .await
        .unwrap();
    let second = store
        .seed_ollama_if_empty("http://127.0.0.1:11434")
        .await
        .unwrap();
    assert!(!second);
    if first {
        let providers = store.list_providers(LEGACY_USER_ID).await.unwrap();
        let ollama = providers
            .iter()
            .find(|provider| provider.display_name == "Ollama")
            .unwrap();
        assert!(ollama.is_default);
        assert_eq!(ollama.protocol, "openai");
        assert!(ollama.url.ends_with("/v1"));
        assert!(ollama.token.is_none());
    }
}

#[tokio::test]
async fn provider_token_is_stored_and_redacted() {
    let store = store().await;
    let id = Uuid::new_v4();
    store
        .upsert_provider(&ProviderRow {
            user_id: LEGACY_USER_ID,
            id,
            protocol: "anthropic".into(),
            display_name: "Anthropic".into(),
            url: "https://api.anthropic.com".into(),
            token: Some("super-secret".into()),
            default_model: Some("claude".into()),
            is_default: false,
            enabled: true,
        })
        .await
        .unwrap();
    let row = store
        .get_provider(LEGACY_USER_ID, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.protocol, "anthropic");
    assert_eq!(row.token.as_deref(), Some("super-secret"));
    let debug = format!("{row:?}");
    assert!(debug.contains("[redacted]"));
    assert!(!debug.contains("super-secret"));
    let cleared = store
        .update_provider(
            LEGACY_USER_ID,
            id,
            ProviderUpdate {
                token: Some(None),
                ..ProviderUpdate::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(cleared.token.is_none());
    assert_eq!(cleared.default_model.as_deref(), Some("claude"));
    let columns = Store::strings_on(
        store.connection(),
        "SELECT column_name FROM information_schema.columns
         WHERE table_schema = current_schema() AND table_name = 'providers'",
        vec![],
    )
    .await
    .unwrap();
    assert!(columns.iter().any(|column| column == "protocol"));
    assert!(columns.iter().any(|column| column == "url"));
    assert!(columns.iter().any(|column| column == "token"));
    assert!(
        !columns
            .iter()
            .any(|column| { column == "kind" || column == "base_url" || column == "api_key_env" })
    );
    assert!(store.delete_provider(LEGACY_USER_ID, id).await.unwrap());
}
