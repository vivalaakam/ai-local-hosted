use std::time::{Duration, Instant};

use crate::sse::StreamPiece;

/// How long the window waits before painting another slice of a stream.
pub const BATCH_INTERVAL: Duration = Duration::from_millis(50);

/// Text gathered since the previous paint.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextBatch {
    pub text: String,
    pub thinking: String,
}

impl TextBatch {
    fn is_empty(&self) -> bool {
        self.text.is_empty() && self.thinking.is_empty()
    }
}

/// Collects stream pieces and releases them on a fixed interval.
#[derive(Debug)]
pub struct DeltaBatch {
    interval: Duration,
    pending: TextBatch,
    last_flush: Instant,
}

impl DeltaBatch {
    pub fn new(interval: Duration, now: Instant) -> Self {
        Self {
            interval,
            pending: TextBatch::default(),
            last_flush: now,
        }
    }

    pub fn push(&mut self, piece: &StreamPiece) {
        match piece {
            StreamPiece::Text(text) => self.pending.text.push_str(text),
            StreamPiece::Thinking(text) => self.pending.thinking.push_str(text),
            StreamPiece::Done => {}
        }
    }

    /// Returns the pending text once `interval` has passed since the last flush.
    pub fn take_due(&mut self, now: Instant) -> Option<TextBatch> {
        if self.pending.is_empty() || now.duration_since(self.last_flush) < self.interval {
            return None;
        }
        self.last_flush = now;
        Some(std::mem::take(&mut self.pending))
    }

    /// Returns whatever is pending, including a flush that is still early.
    pub fn take_pending(&mut self, now: Instant) -> Option<TextBatch> {
        if self.pending.is_empty() {
            return None;
        }
        self.last_flush = now;
        Some(std::mem::take(&mut self.pending))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_pieces_until_the_interval_elapses() {
        let start = Instant::now();
        let mut batch = DeltaBatch::new(Duration::from_millis(50), start);
        batch.push(&StreamPiece::Text("hel".into()));
        batch.push(&StreamPiece::Text("lo".into()));
        assert!(batch.take_due(start + Duration::from_millis(49)).is_none());
        let flushed = batch
            .take_due(start + Duration::from_millis(50))
            .expect("due");
        assert_eq!(flushed.text, "hello");
        assert!(batch.take_due(start + Duration::from_millis(80)).is_none());
    }

    #[test]
    fn take_pending_releases_an_early_tail() {
        let start = Instant::now();
        let mut batch = DeltaBatch::new(Duration::from_millis(50), start);
        batch.push(&StreamPiece::Thinking("hmm".into()));
        batch.push(&StreamPiece::Text("a".into()));
        let flushed = batch.take_pending(start).expect("tail");
        assert_eq!(flushed.thinking, "hmm");
        assert_eq!(flushed.text, "a");
    }
}
