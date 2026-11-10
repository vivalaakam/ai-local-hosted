use alh_desktop::{Command, Login};
use gpui_kit::Context;

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
        let _ = self.db.clear_session();
        self.bridge.send(Command::SetServer { base: url });
        self.banner = Some("Server changed. Sign in again.".into());
        cx.notify();
    }

    pub(crate) fn logout(&mut self, cx: &mut Context<Self>) {
        self.bridge.send(Command::Logout);
        cx.notify();
    }
}
