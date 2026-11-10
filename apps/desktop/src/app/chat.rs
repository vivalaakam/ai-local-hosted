use gpui_kit::component::input::Textarea;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{ClickEvent, Context, FontWeight, SharedString, div, px};

use super::style::{DANGER, INK, LINE, MUTED, SOFT, STARTERS, WHITE, paint};
use super::{ChatWindow, bubble, model_label};

impl ChatWindow {
    pub(crate) fn chat_column(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let header = self.model_header(cx);
        let banner = self.banner.clone();
        let body = if self.feed.messages.is_empty() {
            self.empty_chat(cx).into_any_element()
        } else {
            self.thread(cx).into_any_element()
        };
        let menu = self.model_menu_open.then(|| self.model_menu(cx));
        v_flex()
            .relative()
            .flex_grow(1.)
            .h_full()
            .min_w(px(0.))
            .bg(paint(WHITE))
            .child(header)
            .when_some(banner, |column, text| {
                column.child(
                    div()
                        .w_full()
                        .px_4()
                        .py_2()
                        .text_sm()
                        .text_color(paint(DANGER))
                        .child(text),
                )
            })
            .child(body)
            .when_some(menu, |column, menu| column.child(menu))
    }

    pub(crate) fn model_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let label = if self.model.is_empty() {
            "Select a model".to_string()
        } else {
            model_label(&self.model, &self.providers)
        };
        h_flex()
            .w_full()
            .h(px(52.))
            .justify_center()
            .border_b_1()
            .border_color(paint(LINE))
            .child(
                h_flex()
                    .id("model-menu-toggle")
                    .max_w(px(360.))
                    .gap_1()
                    .px_3()
                    .py_1()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .child(
                        div()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .font_weight(FontWeight::MEDIUM)
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(paint(MUTED))
                            .child(if self.model_menu_open { "▴" } else { "▾" }),
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.model_menu_open = !this.model_menu_open;
                        cx.notify();
                    })),
            )
    }

    pub(crate) fn model_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = self.model.clone();
        let mut items: Vec<gpui_kit::AnyElement> = Vec::new();
        if self.models.is_empty() {
            items.push(
                div()
                    .px_3()
                    .py_2()
                    .text_sm()
                    .text_color(paint(MUTED))
                    .child("Add a provider in Settings to load models.")
                    .into_any_element(),
            );
        }
        for name in &self.models {
            let name = name.clone();
            let active = name == current;
            let label = model_label(&name, &self.providers);
            items.push(
                div()
                    .id(SharedString::from(format!("model-{name}")))
                    .w_full()
                    .px_3()
                    .py_2()
                    .rounded(px(8.))
                    .text_sm()
                    .cursor_pointer()
                    .when(active, |row| row.bg(paint(SOFT)))
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.choose_model(name.clone());
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        div()
            .absolute()
            .top(px(46.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                v_flex()
                    .id("model-menu")
                    .occlude()
                    .w(px(280.))
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .p_1()
                    .gap_1()
                    .rounded(px(12.))
                    .bg(paint(WHITE))
                    .border_1()
                    .border_color(paint(LINE))
                    .shadow_lg()
                    .children(items),
            )
    }

    pub(crate) fn empty_chat(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut chips: Vec<gpui_kit::AnyElement> = Vec::new();
        for &(label, prompt) in STARTERS {
            let prompt = prompt.to_string();
            chips.push(
                div()
                    .id(SharedString::from(format!("starter-{label}")))
                    .px_3()
                    .py_1()
                    .rounded(px(999.))
                    .border_1()
                    .border_color(paint(LINE))
                    .text_sm()
                    .cursor_pointer()
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.fill_prompt(&prompt, cx);
                    }))
                    .into_any_element(),
            );
        }
        let composer = self.composer(cx);
        v_flex()
            .flex_grow(1.)
            .min_h(px(0.))
            .w_full()
            .items_center()
            .justify_center()
            .gap_4()
            .px_4()
            .child(
                div()
                    .text_3xl()
                    .font_weight(FontWeight::MEDIUM)
                    .child("What can I help with?"),
            )
            .child(composer)
            .child(
                h_flex()
                    .w(px(720.))
                    .max_w_full()
                    .flex_wrap()
                    .justify_center()
                    .gap_2()
                    .children(chips),
            )
    }

    pub(crate) fn thread(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let messages = self.feed.messages.clone();
        let composer = self.composer(cx);
        v_flex()
            .flex_grow(1.)
            .min_h(px(0.))
            .w_full()
            .child(
                v_flex()
                    .id("transcript")
                    .flex_grow(1.)
                    .min_h(px(0.))
                    .w_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .py_4()
                    .child(
                        h_flex().w_full().justify_center().child(
                            v_flex()
                                .w(px(720.))
                                .max_w_full()
                                .px_4()
                                .gap_4()
                                .children(messages.iter().map(bubble)),
                        ),
                    ),
            )
            .child(div().w_full().pt_2().pb_3().child(composer))
    }

    pub(crate) fn composer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let streaming = self.streaming;
        let sending = self.sending;
        let send_button = if sending {
            h_flex()
                .id("send")
                .flex_shrink_0()
                .w(px(32.))
                .h(px(32.))
                .rounded(px(16.))
                .justify_center()
                .bg(paint(MUTED))
                .text_color(paint(WHITE))
                .child("…")
                .into_any_element()
        } else {
            h_flex()
                .id("send")
                .flex_shrink_0()
                .w(px(32.))
                .h(px(32.))
                .rounded(px(16.))
                .justify_center()
                .cursor_pointer()
                .bg(paint(INK))
                .text_color(paint(WHITE))
                .child(if streaming { "■" } else { "↑" })
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.send(cx);
                }))
                .into_any_element()
        };
        let composer_hint = if sending {
            "Sending message…"
        } else {
            "AI can make mistakes. Check important information."
        };
        h_flex().w_full().justify_center().child(
            v_flex()
                .w(px(720.))
                .max_w_full()
                .px_4()
                .gap_2()
                .child(
                    h_flex()
                        .w_full()
                        .items_end()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .rounded(px(24.))
                        .bg(paint(SOFT))
                        .child(
                            div().flex_1().min_w(px(0.)).child(
                                Textarea::new(&self.draft_input)
                                    .appearance(false)
                                    .bordered(false),
                            ),
                        )
                        .child(send_button),
                )
                .child(
                    div()
                        .w_full()
                        .text_center()
                        .text_xs()
                        .text_color(paint(MUTED))
                        .child(composer_hint),
                ),
        )
    }
}
