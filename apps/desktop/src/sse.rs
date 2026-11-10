/// One piece of an OpenAI chat-completion stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamPiece {
    Text(String),
    Thinking(String),
    Done,
}

/// Splits a byte stream into SSE frames. A frame that is still arriving stays buffered.
#[derive(Debug, Default)]
pub struct SseBuffer {
    pending: String,
}

impl SseBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, chunk: &str) -> Vec<StreamPiece> {
        self.pending.push_str(chunk);
        let mut out = Vec::new();
        while let Some(index) = self.pending.find("\n\n") {
            let frame = self.pending[..index].to_string();
            self.pending.drain(..index + 2);
            if let Some(piece) = parse_frame(&frame) {
                out.push(piece);
            }
        }
        out
    }
}

fn parse_frame(frame: &str) -> Option<StreamPiece> {
    let mut data = String::new();
    for line in frame.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let Some(rest) = line.strip_prefix("data:") else {
            continue;
        };
        if !data.is_empty() {
            data.push('\n');
        }
        data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
    }
    if data.is_empty() {
        return None;
    }
    if data == "[DONE]" {
        return Some(StreamPiece::Done);
    }
    let value: serde_json::Value = serde_json::from_str(&data).ok()?;
    let delta = value.pointer("/choices/0/delta")?;
    if let Some(text) = delta.get("content").and_then(serde_json::Value::as_str)
        && !text.is_empty()
    {
        return Some(StreamPiece::Text(text.to_string()));
    }
    if let Some(text) = delta
        .get("reasoning_content")
        .and_then(serde_json::Value::as_str)
        && !text.is_empty()
    {
        return Some(StreamPiece::Thinking(text.to_string()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_a_split_chunk_and_keeps_the_tail() {
        let mut buffer = SseBuffer::new();
        let first = buffer.push("data: {\"choices\":[{\"delta\":{\"content\":\"hel\"}}]}\n\nda");
        assert_eq!(first, vec![StreamPiece::Text("hel".into())]);
        let second = buffer.push(
            "ta: {\"choices\":[{\"delta\":{\"reasoning_content\":\"think\"}}]}\n\ndata: [DONE]\n\n",
        );
        assert_eq!(
            second,
            vec![StreamPiece::Thinking("think".into()), StreamPiece::Done]
        );
    }

    #[test]
    fn ignores_comments_and_empty_finish_chunks() {
        let mut buffer = SseBuffer::new();
        let pieces = buffer.push(
            ": keep-alive\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        );
        assert!(pieces.is_empty());
    }
}
