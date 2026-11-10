async fn list_conversations(
    State(state): State<Arc<AppState>>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(query): Query<ListQuery>,
) -> Response {
    let cursor = match query.cursor.as_deref() {
        None => None,
        Some(cursor) => match parse_cursor(cursor) {
            Some(cursor) => Some(cursor),
            None => return ApiError::invalid("invalid cursor").response(),
        },
    };
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    match state.store.list_conversations(user, cursor, limit).await {
        Ok(page) => Json(json!({
            "data": page.items.iter().map(|item| conversation_json(item, None)).collect::<Vec<_>>(),
            "next_cursor": page.next_cursor,
        }))
        .into_response(),
        Err(err) => ApiError::database(err).response(),
    }
}

