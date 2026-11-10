async fn fail_message(state: &AppState, message_id: Uuid, code: &str, error: ApiError) -> Response {
    finalize_chat(
        &state.store,
        message_id,
        "error",
        &alh_core::CompletionAccum::default(),
        Some(code),
    )
    .await;
    error.response()
}

