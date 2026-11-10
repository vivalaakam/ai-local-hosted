fn with_chat_headers(
    mut response: Response,
    conversation: Uuid,
    user_message: Uuid,
    message: Uuid,
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
    response
}

