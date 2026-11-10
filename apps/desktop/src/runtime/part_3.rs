async fn wait_until_settled(api: &Api, id: Uuid) -> Result<ChatDetail, crate::api::ApiError> {
    let mut latest = None;
    for _ in 0..8 {
        let detail = api.get_conversation(id).await?;
        let streaming = detail
            .messages
            .iter()
            .any(|message| message.status == "streaming");
        if !streaming {
            return Ok(detail);
        }
        latest = Some(detail);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    latest.ok_or_else(|| crate::api::ApiError {
        status: 0,
        code: "interrupted".into(),
        message: "conversation did not settle".into(),
    })
}

async fn pump(
    mut live: crate::api::LiveTurn,
    turn: u64,
    interval: Duration,
    tx: UnboundedSender<PumpMsg>,
) {
    let mut guard = EndGuard {
        tx: tx.clone(),
        turn,
        conversation_id: live.conversation_id,
        end: PumpEnd::Dropped,
    };
    let mut sse = SseBuffer::new();
    let mut batch = DeltaBatch::new(interval, Instant::now());
    let mut deadline: Option<tokio::time::Instant> = None;
    loop {
        let idle = async {
            match deadline {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            chunk = live.chunk() => {
                match chunk {
                    Ok(Some(bytes)) => {
                        let text = String::from_utf8_lossy(&bytes);
                        for piece in sse.push(&text) {
                            if matches!(piece, StreamPiece::Done) {
                                flush_batch(&mut batch, turn, live.message_id, &tx);
                                guard.end = PumpEnd::Done;
                                return;
                            }
                            batch.push(&piece);
                            if deadline.is_none() {
                                deadline = Some(tokio::time::Instant::now() + interval);
                            }
                        }
                    }
                    Ok(None) => {
                        flush_batch(&mut batch, turn, live.message_id, &tx);
                        guard.end = PumpEnd::Done;
                        return;
                    }
                    Err(err) => {
                        flush_batch(&mut batch, turn, live.message_id, &tx);
                        guard.end = PumpEnd::Error(err.message);
                        return;
                    }
                }
            }
            _ = idle => {
                flush_batch(&mut batch, turn, live.message_id, &tx);
                deadline = None;
            }
        }
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn import_error(message: String) -> crate::api::ApiError {
    crate::api::ApiError {
        status: 0,
        code: "invalid_request".into(),
        message,
    }
}

fn import_files(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            let entries = std::fs::read_dir(&path)
                .map_err(|err| format!("could not read {}: {err}", path.display()))?;
            let mut found = Vec::new();
            for entry in entries {
                let entry =
                    entry.map_err(|err| format!("could not read {}: {err}", path.display()))?;
                let child = entry.path();
                if !child.is_file() {
                    continue;
                }
                let name = child
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if name.starts_with('.') {
                    continue;
                }
                if name.contains("conversation")
                    && (name.ends_with(".json") || name.ends_with(".zip"))
                {
                    found.push(child);
                }
            }
            found.sort();
            if found.is_empty() {
                return Err(format!("No conversations archive in {}", path.display()));
            }
            files.extend(found);
        } else {
            files.push(path);
        }
    }
    if files.is_empty() {
        return Err("Choose a Claude export archive.".into());
    }
    Ok(files)
}

fn connect_first() -> crate::api::ApiError {
    crate::api::ApiError {
        status: 0,
        code: "unauthorized".into(),
        message: "Connect to the server first".into(),
    }
}

fn flush_batch(batch: &mut DeltaBatch, turn: u64, message_id: Uuid, tx: &UnboundedSender<PumpMsg>) {
    if let Some(batch) = batch.take_pending(Instant::now()) {
        let _ = tx.send(PumpMsg::Batch {
            turn,
            message_id,
            batch,
        });
    }
}

struct EndGuard {
    tx: UnboundedSender<PumpMsg>,
    turn: u64,
    conversation_id: Uuid,
    end: PumpEnd,
}

impl Drop for EndGuard {
    fn drop(&mut self) {
        let end = std::mem::replace(&mut self.end, PumpEnd::Dropped);
        let _ = self.tx.send(PumpMsg::Ended {
            turn: self.turn,
            conversation_id: self.conversation_id,
            end,
        });
    }
}

