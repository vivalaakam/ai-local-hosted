pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/oauth/register", post(crate::oauth::register))
        .route("/oauth/token", post(crate::oauth::token))
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/responses", post(responses))
        .route("/v1/messages", post(messages))
        .route("/v1/messages/count_tokens", post(count_tokens))
        .route(
            "/api/system-prompt",
            get(get_own_system_prompt).put(put_own_system_prompt),
        )
        .route(
            "/api/conversations",
            get(list_conversations).post(create_conversation),
        )
        .route(
            "/api/conversations/{id}",
            get(get_conversation)
                .patch(patch_conversation)
                .delete(delete_conversation),
        )
        .route("/api/conversations/{id}/messages", post(post_message))
        .route(
            "/api/imports/claude",
            post(import_claude).layer(DefaultBodyLimit::max(512 * 1024 * 1024)),
        )
        .merge(crate::providers::routes())
        .layer(middleware::from_fn_with_state(state.clone(), require_auth))
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .with_state(state)
}

