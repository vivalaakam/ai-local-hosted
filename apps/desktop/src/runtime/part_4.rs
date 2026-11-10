#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::BATCH_INTERVAL;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const CHAT: &str = "11111111-1111-1111-1111-111111111111";
    const MESSAGE: &str = "22222222-2222-2222-2222-222222222222";
    const USER_MESSAGE: &str = "33333333-3333-3333-3333-333333333333";

    #[test]
    fn streamed_send_batches_text_and_reloads_the_saved_turn() {
        let base = serve(Script::Stream);
        let bridge = spawn_bridge(base, "test-token".into(), Duration::from_millis(15));
        bridge.send(Command::Send {
            conversation: None,
            content: "hello there".into(),
            model: "stub-model".into(),
        });
        let mut text = String::new();
        let mut saved = false;
        let mut finished = false;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(4) {
            let Ok(event) = bridge.recv_timeout(Duration::from_millis(200)) else {
                continue;
            };
            match event {
                UiEvent::Batch { batch, .. } => text.push_str(&batch.text),
                UiEvent::Opened(detail) => {
                    saved = detail
                        .messages
                        .iter()
                        .any(|message| message.content == "hello");
                }
                UiEvent::Finished { .. } => finished = true,
                UiEvent::Failed { message, .. } => panic!("{message}"),
                _ => {}
            }
            if finished && saved && text == "hello" {
                return;
            }
        }
        panic!("stream did not finish: text={text:?} saved={saved} finished={finished}");
    }

    #[test]
    fn rejected_send_reports_the_server_message() {
        let base = serve(Script::Reject);
        let bridge = spawn_bridge(base, "test-token".into(), BATCH_INTERVAL);
        bridge.send(Command::Send {
            conversation: Some(Uuid::parse_str(CHAT).unwrap()),
            content: "nope".into(),
            model: "stub-model".into(),
        });
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(3) {
            let Ok(event) = bridge.recv_timeout(Duration::from_millis(200)) else {
                continue;
            };
            if let UiEvent::Failed { message, accepted } = event {
                assert!(!accepted);
                assert_eq!(message, "busy");
                return;
            }
        }
        panic!("missing failure");
    }

    #[derive(Clone, Copy)]
    enum Script {
        Stream,
        Reject,
    }

    fn serve(script: Script) -> String {
        let (ready, address) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                ready.send(listener.local_addr().unwrap()).unwrap();
                loop {
                    let Ok((mut socket, _)) = listener.accept().await else {
                        break;
                    };
                let script = script;
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buf = [0_u8; 2048];
                    loop {
                        let n = socket.read(&mut buf).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        request.extend_from_slice(&buf[..n]);
                        if request.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let text = String::from_utf8_lossy(&request);
                    let line = text.lines().next().unwrap_or("");
                    if !text.to_ascii_lowercase().contains("authorization: bearer test-token")
                    {
                        let _ = write_json(&mut socket, 401, r#"{"error":{"code":"unauthorized","message":"unauthorized"}}"#).await;
                        return;
                    }
                    if line.starts_with("GET /api/providers") {
                        let _ = write_json(&mut socket, 200, r#"{"data":[]}"#).await;
                    } else if line.starts_with("GET /v1/models") {
                        let _ = write_json(
                            &mut socket,
                            200,
                            r#"{"object":"list","data":[{"id":"stub-model","object":"model"}]}"#,
                        )
                        .await;
                    } else if line.starts_with("GET /api/conversations/") {
                        let body = format!(
                            r#"{{"id":"{CHAT}","title":"hello there","model":"stub-model","updated_at":"2026-11-08T00:00:00Z","messages":[{{"id":"{MESSAGE}","role":"assistant","content":"hello","status":"done"}}]}}"#
                        );
                        let _ = write_json(&mut socket, 200, &body).await;
                    } else if line.starts_with("GET /api/conversations") {
                        let _ = write_json(&mut socket, 200, r#"{"data":[],"next_cursor":null}"#).await;
                    } else if line.starts_with("POST /api/conversations/") {
                        match script {
                            Script::Reject => {
                                let _ = write_json(
                                    &mut socket,
                                    409,
                                    r#"{"error":{"code":"conflict","message":"busy"}}"#,
                                )
                                .await;
                            }
                            Script::Stream => write_stream(&mut socket).await,
                        }
                    } else if line.starts_with("POST /api/conversations") {
                        let body = format!(
                            r#"{{"id":"{CHAT}","title":"","model":"stub-model","updated_at":"2026-11-08T00:00:00Z"}}"#
                        );
                        let _ = write_json(&mut socket, 201, &body).await;
                    } else {
                        let _ = write_json(
                            &mut socket,
                            404,
                            r#"{"error":{"code":"not_found","message":"no such route"}}"#,
                        )
                        .await;
                    }
                });
                }
            });
        });
        let addr = address.recv().unwrap();
        format!("http://{addr}")
    }

    async fn write_json(
        socket: &mut tokio::net::TcpStream,
        status: u16,
        body: &str,
    ) -> std::io::Result<()> {
        let response = format!(
            "HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await
    }

    async fn write_stream(socket: &mut tokio::net::TcpStream) {
        let header = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\nx-conversation-id: {CHAT}\r\nx-user-message-id: {USER_MESSAGE}\r\nx-message-id: {MESSAGE}\r\nconnection: close\r\n\r\n"
        );
        if socket.write_all(header.as_bytes()).await.is_err() {
            return;
        }
        let _ = write_chunk(
            socket,
            "data: {\"choices\":[{\"delta\":{\"content\":\"hel\"}}]}\n\n",
        )
        .await;
        tokio::time::sleep(Duration::from_millis(40)).await;
        let _ = write_chunk(
            socket,
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
        )
        .await;
        tokio::time::sleep(Duration::from_millis(40)).await;
        let _ = write_chunk(socket, "data: [DONE]\n\n").await;
        let _ = socket.write_all(b"0\r\n\r\n").await;
    }

    async fn write_chunk(socket: &mut tokio::net::TcpStream, data: &str) -> std::io::Result<()> {
        socket
            .write_all(format!("{:x}\r\n", data.len()).as_bytes())
            .await?;
        socket.write_all(data.as_bytes()).await?;
        socket.write_all(b"\r\n").await?;
        socket.flush().await
    }
}
