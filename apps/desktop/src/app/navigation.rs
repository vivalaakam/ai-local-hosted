use gpui_kit::component::input::Input;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{ClickEvent, Context, FontWeight, SharedString, div, px};

use super::style::{DANGER, INK, LINE, MUTED, SOFT, WHITE, paint};
use super::{ChatWindow, avatar, field_label, initial, mark, pill_button, side_row, text_action};

impl ChatWindow {
    pub(crate) fn login_screen(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let banner = self.banner.clone();
        let mut profiles: Vec<gpui_kit::AnyElement> = Vec::new();
        for profile in &self.profiles {
            let id = profile.id;
            let label = if id == self.active_profile.id {
                format!("{} (current)", profile.name)
            } else {
                profile.name.clone()
            };
            profiles.push(
                text_action(
                    SharedString::from(format!("login-profile-{id}")),
                    label,
                    if id == self.active_profile.id {
                        MUTED
                    } else {
                        INK
                    },
                    cx,
                    move |this, cx| this.switch_profile(id, cx),
                )
                .into_any_element(),
            );
        }
        let sign_in = pill_button("sign-in", "Sign in", true, true, cx, |this, cx| {
            this.submit_login(false, cx);
        });
        let create = pill_button(
            "create-account",
            "Create account",
            false,
            true,
            cx,
            |this, cx| this.submit_login(true, cx),
        );
        v_flex()
            .size_full()
            .bg(paint(WHITE))
            .text_color(paint(INK))
            .items_center()
            .justify_center()
            .px_4()
            .child(
                v_flex()
                    .w(px(380.))
                    .max_w_full()
                    .gap_3()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Log in"),
                    )
                    .child(field_label("Server profiles"))
                    .child(h_flex().gap_3().flex_wrap().children(profiles))
                    .child(field_label("Server"))
                    .child(Input::new(&self.url_input))
                    .child(field_label("Username"))
                    .child(Input::new(&self.username_input))
                    .child(field_label("Password"))
                    .child(Input::new(&self.password_input).mask_toggle())
                    .child(sign_in)
                    .child(create)
                    .when_some(banner, |column, text| {
                        column.child(div().text_sm().text_color(paint(DANGER)).child(text))
                    }),
            )
    }

    pub(crate) fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let selected = self.selected;
        let settings_open = self.settings_open;
        let username = if self.username.trim().is_empty() {
            "Account".to_string()
        } else {
            self.username.clone()
        };
        let letter = initial(&username);
        let mut rows: Vec<gpui_kit::AnyElement> = Vec::new();
        for item in &self.conversations {
            let id = item.id;
            let title = if item.title.trim().is_empty() {
                "New chat".to_string()
            } else {
                item.title.clone()
            };
            let active = selected == Some(id);
            let delete = active.then(|| {
                div()
                    .id(SharedString::from(format!("chat-delete-{id}")))
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(paint(DANGER))
                    .cursor_pointer()
                    .child("Delete")
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.delete_selected();
                        cx.notify();
                    }))
                    .into_any_element()
            });
            rows.push(
                h_flex()
                    .id(SharedString::from(format!("chat-{id}")))
                    .w_full()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .when(active, |row| row.bg(paint(SOFT)))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_sm()
                            .child(title),
                    )
                    .when_some(delete, |row, delete| row.child(delete))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.select(id, cx);
                    }))
                    .into_any_element(),
            );
        }
        if rows.is_empty() {
            rows.push(
                div()
                    .px_2()
                    .py_1()
                    .text_sm()
                    .text_color(paint(MUTED))
                    .child("No chats yet")
                    .into_any_element(),
            );
        }
        let new_chat = side_row("new-chat", "New chat", false, cx, |this, cx| {
            this.new_chat(cx);
        });
        let settings = side_row(
            "open-settings",
            "Settings",
            settings_open,
            cx,
            |this, cx| {
                this.settings_open = !this.settings_open;
                this.model_menu_open = false;
                cx.notify();
            },
        );
        let account = h_flex()
            .id("account")
            .w_full()
            .px_3()
            .py_3()
            .gap_2()
            .border_t_1()
            .border_color(paint(LINE))
            .cursor_pointer()
            .child(avatar(&letter))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_sm()
                    .child(username),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.profile_menu_open = true;
                this.model_menu_open = false;
                cx.notify();
            }));

        v_flex()
            .w(px(260.))
            .h_full()
            .flex_shrink_0()
            .bg(paint(WHITE))
            .text_color(paint(INK))
            .border_r_1()
            .border_color(paint(LINE))
            .child(
                h_flex()
                    .px_3()
                    .pt_4()
                    .pb_2()
                    .gap_2()
                    .child(mark())
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("ALH")),
            )
            .child(v_flex().px_2().gap_1().child(new_chat).child(settings))
            .child(
                div()
                    .px_3()
                    .pt_4()
                    .pb_1()
                    .text_xs()
                    .text_color(paint(MUTED))
                    .child("Chats"),
            )
            .child(
                v_flex()
                    .id("chats")
                    .flex_grow(1.)
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .px_2()
                    .gap_1()
                    .children(rows),
            )
            .child(account)
    }
}
