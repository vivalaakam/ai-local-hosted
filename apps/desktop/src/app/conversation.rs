use alh_desktop::{Command, ConversationItem, draft_key};
use gpui_kit::Context;
use uuid::Uuid;

use super::ChatWindow;

impl ChatWindow {
    pub(crate) fn upsert(&mut self, item: ConversationItem) {
        if let Some(existing) = self.conversations.iter_mut().find(|row| row.id == item.id) {
            *existing = item;
        } else {
            self.conversations.insert(0, item);
        }
        let _ = self.db.replace_cache(&self.conversations);
    }

    pub(crate) fn persist_draft(&mut self, cx: &mut Context<Self>) {
        let text = self.draft_input.read(cx).value().to_string();
        self.feed.set_draft(text.clone());
        let _ = self.db.set_draft(&draft_key(self.selected), &text);
    }

    pub(crate) fn select(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.model_menu_open = false;
        if self.selected == Some(id) {
            cx.notify();
            return;
        }
        self.persist_draft(cx);
        if self.streaming || self.sending {
            self.bridge.send(Command::Stop);
            self.streaming = false;
            self.sending = false;
        }
        self.selected = Some(id);
        if let Some(item) = self.conversations.iter().find(|item| item.id == id)
            && !item.model.is_empty()
        {
            self.model = item.model.clone();
        }
        self.composer_text = Some(self.db.draft(&draft_key(Some(id))).unwrap_or_default());
        self.bridge.send(Command::Open(id));
        cx.notify();
    }

    pub(crate) fn new_chat(&mut self, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.model_menu_open = false;
        if self.model.is_empty() {
            self.banner = Some("Pick a model first".into());
            cx.notify();
            return;
        }
        self.persist_draft(cx);
        self.bridge.send(Command::Create {
            model: self.model.clone(),
        });
    }

    pub(crate) fn delete_selected(&mut self) {
        let Some(id) = self.selected else {
            return;
        };
        self.bridge.send(Command::Delete(id));
    }

    pub(crate) fn choose_model(&mut self, model: String) {
        self.model_menu_open = false;
        self.model = model.clone();
        if let Some(id) = self.selected {
            self.bridge.send(Command::SetModel {
                conversation: id,
                model,
            });
        }
    }

    pub(crate) fn send(&mut self, cx: &mut Context<Self>) {
        if self.streaming {
            self.bridge.send(Command::Stop);
            self.streaming = false;
            return;
        }
        let content = self.draft_input.read(cx).value().trim().to_string();
        if content.is_empty() || self.sending {
            return;
        }
        if self.model.is_empty() {
            self.banner = Some("Pick a model first".into());
            cx.notify();
            return;
        }
        self.sending = true;
        self.banner = None;
        self.bridge.send(Command::Send {
            conversation: self.selected,
            content,
            model: self.model.clone(),
        });
        cx.notify();
    }
}
