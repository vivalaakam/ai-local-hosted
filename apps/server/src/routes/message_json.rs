fn message_json(message: &alh_storage::MessageRow) -> Value {
    json!({
        "id": message.id,
        "position": message.position,
        "role": message.role,
        "content": message.content,
        "tool_calls": message.tool_calls,
        "tool_call_id": message.tool_call_id,
        "thinking": message.thinking,
        "status": message.status,
        "error_code": message.error_code,
    })
}
