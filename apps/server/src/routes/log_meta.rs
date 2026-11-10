fn log_meta(
    user_id: Uuid,
    source: &'static str,
    protocol: &'static str,
    route: &crate::state::ResolvedRoute,
    conversation_id: Option<Uuid>,
    message_id: Option<Uuid>,
) -> LogMeta {
    LogMeta {
        user_id,
        source,
        protocol,
        conversation_id,
        message_id,
        provider_id: route.provider_id,
        client_model: route.client_model.clone(),
        upstream_model: route.upstream_model.clone(),
        started: Instant::now(),
    }
}

