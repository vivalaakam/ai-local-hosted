async fn count_tokens(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(body): Json<Value>,
) -> Response {
    match parse_anthropic_message(&body) {
        Ok(mut parsed) => match prepend_server_prompts(&state, user, &mut parsed.request).await {
            Ok(()) => Json(json!({
                "input_tokens": estimate_request_tokens(&parsed.request),
            }))
            .into_response(),
            Err(err) => err.anthropic().response(),
        },
        Err(err) => ApiError::invalid(err.to_string()).anthropic().response(),
    }
}

