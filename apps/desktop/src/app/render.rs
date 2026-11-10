use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, Focusable, SharedString, Window, div, px};

use super::style::{INK, MUTED, SOFT, WHITE, paint};
use super::{Bubble, ChatWindow};

impl Render for ChatWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(text) = self.server_url_text.take() {
            self.url_input
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
        if let Some(text) = self.composer_text.take() {
            self.draft_input
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
        if let Some(text) = self.password_text.take() {
            self.password_input
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
        if let Some(text) = self.session_token_text.take() {
            self.session_token_input
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
        if let Some(text) = self.provider_name_text.take() {
            self.provider_name_input
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
        if let Some(text) = self.provider_url_text.take() {
            self.provider_url_input
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
        if let Some(text) = self.provider_token_text.take() {
            self.provider_token_input
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
        if let Some(text) = self.profile_name_text.take() {
            self.profile_name_input
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
        if let Some(text) = self.profile_url_text.take() {
            self.profile_url_input
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
        if !self.signed_in {
            return self.login_screen(cx).into_any_element();
        }
        let sidebar = self.sidebar(cx);
        let main = if self.settings_open {
            self.settings_panel(cx).into_any_element()
        } else {
            self.chat_column(cx).into_any_element()
        };
        let modal = self.provider_form_open.then(|| self.provider_modal(cx));
        let profile_menu = self.profile_menu_open.then(|| self.profile_menu(cx));
        let profile_form = self.profile_form_open.then(|| self.profile_form(cx));
        div()
            .relative()
            .size_full()
            .bg(paint(WHITE))
            .text_color(paint(INK))
            .child(h_flex().size_full().child(sidebar).child(main))
            .when_some(modal, |root, modal| root.child(modal))
            .when_some(profile_menu, |root, modal| root.child(modal))
            .when_some(profile_form, |root, modal| root.child(modal))
            .into_any_element()
    }
}

pub(crate) fn bubble(message: &Bubble) -> gpui_kit::Div {
    let mine = message.role == "user";
    let mut body = v_flex().gap_1();
    if mine {
        body = body
            .self_end()
            .w_auto()
            .max_w(px(520.))
            .px_3()
            .py_2()
            .rounded(px(18.))
            .bg(paint(SOFT));
    } else {
        body = body.w_full();
    }
    if !message.thinking.is_empty() {
        body = body.child(
            div()
                .text_sm()
                .text_color(paint(MUTED))
                .child(message.thinking.clone()),
        );
    }
    let text = if message.content.is_empty() && message.status == "streaming" {
        "…".to_string()
    } else {
        message.content.clone()
    };
    if mine {
        for line in text.split('\n') {
            body = body.child(
                div()
                    .w_full()
                    .whitespace_normal()
                    .child(if line.is_empty() {
                        " ".to_string()
                    } else {
                        line.to_string()
                    }),
            );
        }
    } else {
        body = body.child(
            TextView::markdown(SharedString::from(format!("message-{}", message.id)), text)
                .selectable(true)
                .style(TextViewStyle::default().paragraph_gap(gpui_kit::rems(0.75)))
                .w_full(),
        );
    }
    if message.status != "done" && message.status != "streaming" {
        let note = match &message.error_code {
            Some(code) if !code.is_empty() => format!("{} · {code}", message.status),
            _ => message.status.clone(),
        };
        body = body.child(div().text_sm().text_color(paint(MUTED)).child(note));
    }
    body
}

impl Focusable for ChatWindow {
    fn focus_handle(&self, cx: &gpui_kit::App) -> gpui_kit::FocusHandle {
        self.draft_input.focus_handle(cx)
    }
}
