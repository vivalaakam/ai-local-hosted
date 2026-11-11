use crate::batch::TextBatch;

/// One row in the transcript. `id` is the server id, or a local placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bubble {
    pub id: String,
    pub role: String,
    pub content: String,
    pub thinking: String,
    pub status: String,
    pub error_code: Option<String>,
}

/// Transcript plus the unsent composer text.
///
/// The composer is cleared only after the server accepts the turn. A failure
/// before that leaves the draft in place. A failure after that keeps the text
/// that already arrived.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Feed {
    pub messages: Vec<Bubble>,
    pub draft: String,
    pub streaming_id: Option<String>,
    pub banner: Option<String>,
}

impl Bubble {
    /// Tool round-trips stay in Postgres for the model, but the chat UI only
    /// shows user/assistant text (and the optional first-turn system prompt).
    pub fn visible_in_chat(&self) -> bool {
        match self.role.as_str() {
            "tool" => false,
            "assistant"
                if self.content.trim().is_empty()
                    && self.thinking.trim().is_empty()
                    && self.status != "streaming"
                    && self.status != "error" =>
            {
                false
            }
            _ => true,
        }
    }
}

impl Feed {
    pub fn set_draft(&mut self, draft: String) {
        self.draft = draft;
    }

    pub fn accept(&mut self, user: Bubble, assistant: Bubble) {
        self.banner = None;
        self.draft.clear();
        self.streaming_id = Some(assistant.id.clone());
        self.messages.push(user);
        self.messages.push(assistant);
    }

    pub fn apply(&mut self, assistant_id: &str, batch: &TextBatch) {
        let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.id == assistant_id)
        else {
            return;
        };
        message.content.push_str(&batch.text);
        message.thinking.push_str(&batch.thinking);
    }

    pub fn finish(&mut self, assistant_id: &str, status: &str) {
        if let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.id == assistant_id)
        {
            message.status = status.to_string();
        }
        if self.streaming_id.as_deref() == Some(assistant_id) {
            self.streaming_id = None;
        }
    }

    /// The request never reached the server. The draft stays.
    pub fn fail_before_accept(&mut self, message: impl Into<String>) {
        self.banner = Some(message.into());
    }

    /// The server accepted the turn and then the stream failed.
    pub fn fail_after_accept(&mut self, assistant_id: &str, message: impl Into<String>) {
        let message = message.into();
        if let Some(bubble) = self
            .messages
            .iter_mut()
            .find(|bubble| bubble.id == assistant_id)
        {
            bubble.status = "error".into();
            bubble.error_code = Some(message.clone());
        }
        self.banner = Some(message);
        if self.streaming_id.as_deref() == Some(assistant_id) {
            self.streaming_id = None;
        }
    }

    pub fn replace_messages(&mut self, messages: Vec<Bubble>) {
        let messages: Vec<Bubble> = messages
            .into_iter()
            .filter(Bubble::visible_in_chat)
            .collect();
        self.streaming_id = messages
            .iter()
            .find(|message| message.status == "streaming")
            .map(|message| message.id.clone());
        self.messages = messages;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(content: &str) -> Bubble {
        Bubble {
            id: "user".into(),
            role: "user".into(),
            content: content.into(),
            thinking: String::new(),
            status: "done".into(),
            error_code: None,
        }
    }

    fn assistant() -> Bubble {
        Bubble {
            id: "asst".into(),
            role: "assistant".into(),
            content: String::new(),
            thinking: String::new(),
            status: "streaming".into(),
            error_code: None,
        }
    }

    #[test]
    fn accept_clears_the_draft_and_failure_before_accept_does_not() {
        let mut feed = Feed::default();
        feed.set_draft("keep me".into());
        feed.fail_before_accept("offline");
        assert_eq!(feed.draft, "keep me");
        assert_eq!(feed.banner.as_deref(), Some("offline"));

        feed.accept(user("keep me"), assistant());
        assert!(feed.draft.is_empty());
        assert!(feed.banner.is_none());
        assert_eq!(feed.streaming_id.as_deref(), Some("asst"));
    }

    #[test]
    fn replace_messages_hides_tool_results_and_empty_tool_assistants() {
        let mut feed = Feed::default();
        feed.replace_messages(vec![
            Bubble {
                id: "sys".into(),
                role: "system".into(),
                content: "prompt".into(),
                thinking: String::new(),
                status: "done".into(),
                error_code: None,
            },
            user("hi"),
            Bubble {
                id: "tool-call".into(),
                role: "assistant".into(),
                content: String::new(),
                thinking: String::new(),
                status: "done".into(),
                error_code: None,
            },
            Bubble {
                id: "tool".into(),
                role: "tool".into(),
                content: r#"{"utc":"2026-01-01T00:00:00Z"}"#.into(),
                thinking: String::new(),
                status: "done".into(),
                error_code: None,
            },
            Bubble {
                id: "final".into(),
                role: "assistant".into(),
                content: "It is midnight UTC.".into(),
                thinking: String::new(),
                status: "done".into(),
                error_code: None,
            },
        ]);
        assert_eq!(feed.messages.len(), 3);
        assert_eq!(feed.messages[0].role, "system");
        assert_eq!(feed.messages[1].role, "user");
        assert_eq!(feed.messages[2].content, "It is midnight UTC.");
    }

    #[test]
    fn stop_and_stream_error_keep_the_text_already_shown() {
        let mut feed = Feed::default();
        feed.accept(user("hi"), assistant());
        feed.apply(
            "asst",
            &TextBatch {
                text: "partial".into(),
                thinking: String::new(),
            },
        );
        feed.finish("asst", "cancelled");
        assert_eq!(feed.messages[1].content, "partial");
        assert_eq!(feed.messages[1].status, "cancelled");
        assert!(feed.streaming_id.is_none());

        feed.messages[1].status = "streaming".into();
        feed.streaming_id = Some("asst".into());
        feed.fail_after_accept("asst", "upstream");
        assert_eq!(feed.messages[1].content, "partial");
        assert_eq!(feed.messages[1].status, "error");
        assert_eq!(feed.banner.as_deref(), Some("upstream"));
    }
}
