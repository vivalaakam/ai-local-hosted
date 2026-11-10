async fn next_event(
    stream: &mut alh_core::CompletionStream,
) -> Result<Option<alh_core::CompletionEvent>, alh_core::ProviderError> {
    match stream.next().await {
        None => Ok(None),
        Some(result) => result.map(Some),
    }
}

