use std::time::Duration;

use gpui_kit::Context;

use super::{Bubble, ChatWindow, Feed, SessionCreds, UiEvent, draft_key};

impl ChatWindow {
    pub(crate) fn watch(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                if this.update(cx, |view, cx| view.poll(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn poll(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        while let Some(event) = self.bridge.try_recv() {
            self.apply(event);
            changed = true;
        }
        if changed {
            self.scroll.scroll_to_bottom();
            cx.notify();
        }
    }

    pub(crate) fn apply(&mut self, event: UiEvent) {
        match event {
            UiEvent::Conversations(items) => {
                self.conversations = items;
                let _ = self.db.replace_cache(&self.conversations);
            }
            UiEvent::Models(models) => {
                if !models.iter().any(|name| name == &self.model) {
                    self.model = models.first().cloned().unwrap_or_default();
                }
                self.models = models;
            }
            UiEvent::Providers(providers) => {
                let editing_gone = self
                    .editing_provider
                    .is_some_and(|id| !providers.iter().any(|row| row.id == id));
                self.providers = providers;
                if editing_gone {
                    self.close_provider_form();
                }
            }
            UiEvent::ProviderSaved => {
                self.close_provider_form();
                self.banner = Some("Provider saved".into());
            }
            UiEvent::Opened(detail) => {
                self.upsert(detail.item.clone());
                if self.selected == Some(detail.item.id) || self.selected.is_none() {
                    self.selected = Some(detail.item.id);
                    if !detail.item.model.is_empty() {
                        self.model = detail.item.model.clone();
                    }
                    self.feed.replace_messages(
                        detail
                            .messages
                            .into_iter()
                            .map(|message| Bubble {
                                id: message.id.to_string(),
                                role: message.role,
                                content: message.content,
                                thinking: message.thinking,
                                status: message.status,
                                error_code: message.error_code,
                            })
                            .collect(),
                    );
                    self.streaming = self.feed.streaming_id.is_some();
                }
            }
            UiEvent::Created(item) => {
                let id = item.id;
                if !item.model.is_empty() {
                    self.model = item.model.clone();
                }
                self.upsert(item);
                if self.selected != Some(id) {
                    self.selected = Some(id);
                    self.feed = Feed::default();
                    self.composer_text =
                        Some(self.db.draft(&draft_key(Some(id))).unwrap_or_default());
                }
            }
            UiEvent::Accepted {
                conversation_id,
                user_message_id,
                message_id,
                content,
            } => {
                let _ = self.db.clear_draft(&draft_key(Some(conversation_id)));
                let _ = self.db.clear_draft(&draft_key(None));
                self.selected = Some(conversation_id);
                self.sending = false;
                self.streaming = true;
                self.banner = None;
                self.composer_text = Some(String::new());
                self.feed.accept(
                    Bubble {
                        id: user_message_id.to_string(),
                        role: "user".into(),
                        content,
                        thinking: String::new(),
                        status: "done".into(),
                        error_code: None,
                    },
                    Bubble {
                        id: message_id.to_string(),
                        role: "assistant".into(),
                        content: String::new(),
                        thinking: String::new(),
                        status: "streaming".into(),
                        error_code: None,
                    },
                );
            }
            UiEvent::Batch { message_id, batch } => {
                self.feed.apply(&message_id.to_string(), &batch);
            }
            UiEvent::Finished { .. } => {
                self.streaming = false;
                self.sending = false;
            }
            UiEvent::Stopped { .. } => {
                self.streaming = false;
                self.sending = false;
                if let Some(id) = self.feed.streaming_id.clone() {
                    self.feed.finish(&id, "cancelled");
                }
            }
            UiEvent::Failed { message, accepted } => {
                self.notice = None;
                self.sending = false;
                if accepted {
                    if let Some(id) = self.feed.streaming_id.clone() {
                        self.feed.fail_after_accept(&id, message);
                    } else {
                        self.banner = Some(message);
                    }
                    self.streaming = false;
                } else {
                    self.feed.fail_before_accept(message.clone());
                    self.banner = Some(message);
                }
            }
            UiEvent::SignedIn {
                username,
                access_token,
                refresh_token,
                access_expires_at,
            } => {
                self.signed_in = true;
                self.username = username.clone();
                self.session_token_text = Some(access_token.clone());
                self.password_text = Some(String::new());
                self.banner = None;
                if let Err(err) = self.db.set_session(&SessionCreds {
                    username,
                    access_token,
                    refresh_token,
                    access_expires_at,
                }) {
                    self.banner = Some(err.to_string());
                }
            }
            UiEvent::SignedOut => {
                self.signed_in = false;
                self.settings_open = false;
                self.notice = None;
                self.close_provider_form();
                self.username.clear();
                self.session_token_text = Some(String::new());
                self.conversations.clear();
                self.models.clear();
                self.providers.clear();
                self.selected = None;
                self.feed = Feed::default();
                self.streaming = false;
                self.sending = false;
                let _ = self.db.clear_session();
                let _ = self.db.replace_cache(&[]);
            }
            UiEvent::Notice(message) => {
                self.banner = None;
                self.notice = Some(message);
            }
            UiEvent::Deleted(id) => {
                self.conversations.retain(|item| item.id != id);
                let _ = self.db.replace_cache(&self.conversations);
                let _ = self.db.clear_draft(&draft_key(Some(id)));
                if self.selected == Some(id) {
                    self.selected = None;
                    self.feed = Feed::default();
                    self.composer_text = Some(self.db.draft(&draft_key(None)).unwrap_or_default());
                }
            }
        }
        if let Some(banner) = self.feed.banner.clone() {
            self.banner = Some(banner);
        }
    }
}
