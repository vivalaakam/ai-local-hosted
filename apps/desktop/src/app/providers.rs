use alh_desktop::{Command, ProviderDraft};
use gpui_kit::{Context, PathPromptOptions};
use uuid::Uuid;

use super::style::{known_preset, preset_for};
use super::{ChatWindow, valid_http_url};

impl ChatWindow {
    pub(crate) fn close_provider_form(&mut self) {
        let preset = known_preset("ollama");
        self.provider_form_open = false;
        self.editing_provider = None;
        self.provider_preset = preset.id.into();
        self.provider_protocol = preset.protocol.into();
        self.provider_default = !self.providers.iter().any(|row| row.is_default);
        self.provider_enabled = true;
        self.provider_name_text = Some(preset.name.into());
        self.provider_url_text = Some(preset.url.into());
        self.provider_token_text = Some(String::new());
    }

    pub(crate) fn select_preset(&mut self, id: &str, cx: &mut Context<Self>) {
        let next = known_preset(id);
        let current = self.provider_name_input.read(cx).value().trim().to_string();
        let previous_name = known_preset(&self.provider_preset).name;
        if !next.name.is_empty() && (current.is_empty() || current == previous_name) {
            self.provider_name_text = Some(next.name.to_string());
        }
        if next.id != "custom" {
            self.provider_protocol = next.protocol.into();
            self.provider_url_text = Some(next.url.into());
        }
        self.provider_preset = next.id.into();
        cx.notify();
    }

    pub(crate) fn open_new_provider(&mut self) {
        self.close_provider_form();
        self.provider_form_open = true;
    }

    pub(crate) fn edit_provider(&mut self, id: Uuid) {
        let Some(row) = self.providers.iter().find(|row| row.id == id).cloned() else {
            return;
        };
        let preset = preset_for(&row.protocol, &row.url);
        self.provider_form_open = true;
        self.editing_provider = Some(row.id);
        self.provider_preset = preset.id.into();
        self.provider_protocol = row.protocol;
        self.provider_default = row.is_default;
        self.provider_enabled = row.enabled;
        self.provider_name_text = Some(row.display_name);
        self.provider_url_text = Some(row.url);
        self.provider_token_text = Some(String::new());
    }

    pub(crate) fn save_provider(&mut self, cx: &mut Context<Self>) {
        let name = self.provider_name_input.read(cx).value().trim().to_string();
        if name.is_empty() || name.chars().count() > 200 {
            self.banner = Some("Provider name is required".into());
            cx.notify();
            return;
        }
        let preset = known_preset(&self.provider_preset);
        let (url, protocol) = if preset.id == "custom" {
            let url = self.provider_url_input.read(cx).value().trim().to_string();
            if !valid_http_url(&url) {
                self.banner = Some("Provider URL must start with http:// or https://".into());
                cx.notify();
                return;
            }
            let protocol = if self.provider_protocol == "anthropic" {
                "anthropic"
            } else {
                "openai"
            };
            (url, protocol.to_string())
        } else {
            (preset.url.to_string(), preset.protocol.to_string())
        };
        let typed = self
            .provider_token_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if preset.token_required && typed.is_empty() && self.editing_provider.is_none() {
            self.banner = Some("Provider token is required".into());
            cx.notify();
            return;
        }
        let token = if typed.is_empty() { None } else { Some(typed) };
        self.provider_token_text = Some(String::new());
        self.bridge.send(Command::SaveProvider(ProviderDraft {
            id: self.editing_provider,
            protocol,
            url,
            display_name: name,
            token,
            is_default: self.provider_default,
            enabled: self.provider_enabled,
            update: self.editing_provider.is_some(),
            reset_form: true,
        }));
        cx.notify();
    }

    pub(crate) fn delete_provider(&mut self, id: Uuid) {
        self.bridge.send(Command::DeleteProvider(id));
    }

    pub(crate) fn make_default(&mut self, id: Uuid) {
        let Some(row) = self.providers.iter().find(|row| row.id == id).cloned() else {
            return;
        };
        self.bridge.send(Command::SaveProvider(ProviderDraft {
            id: Some(row.id),
            protocol: row.protocol,
            url: row.url,
            display_name: row.display_name,
            token: None,
            is_default: true,
            enabled: row.enabled,
            update: true,
            reset_form: false,
        }));
    }

    pub(crate) fn import_archive(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: Some("Import Claude export".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            if paths.is_empty() {
                return;
            }
            this.update(cx, |this, cx| {
                this.notice = Some("Importing Claude export…".into());
                this.banner = None;
                this.bridge.send(Command::ImportClaude { paths });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn fill_prompt(&mut self, text: &str, cx: &mut Context<Self>) {
        self.composer_text = Some(text.to_string());
        cx.notify();
    }
}
