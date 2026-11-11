fn with_chat_headers(
    mut response: Response,
    conversation: Uuid,
    user_message: Uuid,
    message: Uuid,
    system_message: Option<Uuid>,
) -> Response {
    insert_header(
        &mut response,
        "x-conversation-id",
        &conversation.to_string(),
    );
    insert_header(
        &mut response,
        "x-user-message-id",
        &user_message.to_string(),
    );
    insert_header(&mut response, "x-message-id", &message.to_string());
    if let Some(system_message) = system_message {
        insert_header(
            &mut response,
            "x-system-message-id",
            &system_message.to_string(),
        );
    }
    response
}

