fn conversation_json(
    conversation: &alh_storage::Conversation,
    messages: Option<&[alh_storage::MessageRow]>,
) -> Value {
    let mut value = json!({
        "id": conversation.id,
        "title": conversation.title,
        "provider_id": conversation.provider_id,
        "model": conversation.model,
        "system_prompt": conversation.system_prompt,
        "created_at": conversation.created_at.to_rfc3339(),
        "updated_at": conversation.updated_at.to_rfc3339(),
        "archived_at": conversation.archived_at.map(|value| value.to_rfc3339()),
    });
    if let Some(messages) = messages {
        value["messages"] = json!(messages.iter().map(message_json).collect::<Vec<_>>());
    }
    value
}

