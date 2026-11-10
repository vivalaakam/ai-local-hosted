use alh_desktop::{BATCH_INTERVAL, Command, DesktopSession, Login, draft_key, spawn_session};
use gpui_kit::Context;
use uuid::Uuid;

use super::{ChatWindow, valid_http_url, valid_slug};

impl ChatWindow {
    pub(crate) fn submit_login(&mut self, register: bool, cx: &mut Context<Self>) {
        let url = self.url_input.read(cx).value().trim().to_string();
        let username = self.username_input.read(cx).value().trim().to_string();
        let password = self.password_input.read(cx).value().to_string();
        if !valid_http_url(&url) {
            self.banner = Some("Server URL must start with http:// or https://".into());
            cx.notify();
            return;
        }
        if !valid_slug(&username) {
            self.banner = Some("Username must be 1–64 letters, digits, _ or -".into());
            cx.notify();
            return;
        }
        if password.len() < 8 {
            self.banner = Some("Password must be at least 8 characters".into());
            cx.notify();
            return;
        }
        if let Err(err) = self.db.set_server_url(&url) {
            self.banner = Some(err.to_string());
            cx.notify();
            return;
        }
        self.refresh_profile_url(&url);
        self.banner = None;
        self.bridge.send(Command::Login(Login {
            base: url,
            username,
            password,
            register,
        }));
        self.password_text = Some(String::new());
        cx.notify();
    }

    pub(crate) fn save_server(&mut self, cx: &mut Context<Self>) {
        let url = self.url_input.read(cx).value().trim().to_string();
        if !valid_http_url(&url) {
            self.banner = Some("Server URL must start with http:// or https://".into());
            cx.notify();
            return;
        }
        if let Err(err) = self.db.set_server_url(&url) {
            self.banner = Some(err.to_string());
            cx.notify();
            return;
        }
        self.refresh_profile_url(&url);
        let _ = self.db.clear_session();
        self.bridge.send(Command::SetServer { base: url });
        self.banner = Some("Server changed. Sign in again.".into());
        cx.notify();
    }

    pub(crate) fn logout(&mut self, cx: &mut Context<Self>) {
        self.bridge.send(Command::Logout);
        cx.notify();
    }

    pub(crate) fn save_profile(&mut self, cx: &mut Context<Self>) {
        let name = self.profile_name_input.read(cx).value().trim().to_string();
        let url = self.profile_url_input.read(cx).value().trim().to_string();
        if name.is_empty() || name.chars().count() > 100 {
            self.banner = Some("Profile name is required".into());
            cx.notify();
            return;
        }
        if !valid_http_url(&url) {
            self.banner = Some("Server URL must start with http:// or https://".into());
            cx.notify();
            return;
        }
        match self.db.create_profile(&name, &url) {
            Ok(profile) => {
                self.profile_form_open = false;
                self.switch_profile(profile.id, cx);
            }
            Err(err) => {
                self.banner = Some(err.to_string());
                cx.notify();
            }
        }
    }

    pub(crate) fn switch_profile(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.active_profile.id == id {
            return;
        }
        self.persist_draft(cx);
        if let Err(err) = self.db.select_profile(id) {
            self.banner = Some(err.to_string());
            cx.notify();
            return;
        }
        let profile = match self.db.active_profile() {
            Ok(profile) => profile,
            Err(err) => {
                self.banner = Some(err.to_string());
                cx.notify();
                return;
            }
        };
        let session = self.db.session().unwrap_or_else(|err| {
            self.banner = Some(err.to_string());
            None
        });
        let username = session
            .as_ref()
            .map(|session| session.username.clone())
            .unwrap_or_default();
        let access_token = session
            .as_ref()
            .map(|session| session.access_token.clone())
            .unwrap_or_default();
        self.bridge = spawn_session(
            profile.url.clone(),
            DesktopSession {
                username: username.clone(),
                access_token: access_token.clone(),
                refresh_token: session
                    .as_ref()
                    .map(|session| session.refresh_token.clone())
                    .unwrap_or_default(),
                access_expires_at: session
                    .as_ref()
                    .map(|session| session.access_expires_at)
                    .unwrap_or(0),
            },
            BATCH_INTERVAL,
        );
        self.active_profile = profile;
        self.profiles = self
            .db
            .profiles()
            .unwrap_or_else(|_| vec![self.active_profile.clone()]);
        self.conversations = self.db.cached_conversations().unwrap_or_default();
        self.models.clear();
        self.providers.clear();
        self.selected = None;
        self.feed = Default::default();
        self.streaming = false;
        self.sending = false;
        self.signed_in = session.is_some();
        self.username = username;
        self.session_token_text = Some(access_token);
        self.server_url_text = Some(self.active_profile.url.clone());
        self.composer_text = Some(self.db.draft(&draft_key(None)).unwrap_or_default());
        self.banner = None;
        cx.notify();
    }

    fn refresh_profile_url(&mut self, url: &str) {
        self.active_profile.url = url.to_string();
        self.profiles = self
            .db
            .profiles()
            .unwrap_or_else(|_| vec![self.active_profile.clone()]);
    }
}
