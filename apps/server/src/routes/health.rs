async fn health(State(state): State<Arc<AppState>>) -> Response {
    let database_ok = state.store.ping().await.is_ok();
    let status = if database_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(json!({
            "status": if database_ok { "ok" } else { "down" },
            "database": if database_ok { "ok" } else { "down" },
            "provider": "unknown",
        })),
    )
        .into_response()
}

