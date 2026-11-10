async fn finish_encoder<F>(
    tx: &tokio::sync::mpsc::Sender<Result<axum::response::sse::Event, std::convert::Infallible>>,
    outcome: &crate::turn::Outcome,
    mut encode: F,
) where
    F: FnMut(&alh_core::CompletionEvent) -> Vec<alh_core::SseFrame>,
{
    if outcome.cancelled || outcome.error_code.is_some() || outcome.accum.finish.is_some() {
        return;
    }
    let _ = forward(
        tx,
        encode(&alh_core::CompletionEvent::Done {
            finish_reason: alh_core::FinishReason::Stop,
            usage: outcome.accum.usage,
        }),
    )
    .await;
}

