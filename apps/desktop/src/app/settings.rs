use gpui_kit::component::input::Input;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{ClickEvent, Context, FontWeight, SharedString, div, px};

use super::style::{DANGER, INK, LINE, MUTED, paint};
use super::{ChatWindow, avatar, field_label, initial, pill_button, text_action};

impl ChatWindow {
    pub(crate) fn settings_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let username = if self.username.trim().is_empty() {
            "Account".to_string()
        } else {
            self.username.clone()
        };
        let letter = initial(&username);
        let banner = self.banner.clone();
        let notice = self.notice.clone();
        let mut providers: Vec<gpui_kit::AnyElement> = Vec::new();
        if self.providers.is_empty() {
            providers.push(
                div()
                    .text_sm()
                    .text_color(paint(MUTED))
                    .child("No providers yet.")
                    .into_any_element(),
            );
        }
        for row in &self.providers {
            let id = row.id;
            let label = if row.display_name.trim().is_empty() {
                "Untitled".to_string()
            } else {
                row.display_name.clone()
            };
            let meta = format!(
                "{} · {}{}",
                row.protocol,
                row.url,
                if row.is_default { " · default" } else { "" }
            );
            let name = div()
                .id(SharedString::from(format!("provider-{id}")))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .child(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.edit_provider(id);
                    cx.notify();
                }));
            let edit = text_action(
                SharedString::from(format!("provider-edit-{id}")),
                "Edit",
                INK,
                cx,
                move |this, _cx| this.edit_provider(id),
            )
            .into_any_element();
            let make_default = if row.is_default {
                div()
                    .text_sm()
                    .text_color(paint(MUTED))
                    .child("Default")
                    .into_any_element()
            } else {
                text_action(
                    SharedString::from(format!("provider-default-{id}")),
                    "Default",
                    INK,
                    cx,
                    move |this, _cx| this.make_default(id),
                )
                .into_any_element()
            };
            let remove = text_action(
                SharedString::from(format!("provider-delete-{id}")),
                "Delete",
                DANGER,
                cx,
                move |this, _cx| this.delete_provider(id),
            )
            .into_any_element();
            providers.push(
                v_flex()
                    .w_full()
                    .py_3()
                    .gap_1()
                    .border_b_1()
                    .border_color(paint(LINE))
                    .child(name)
                    .child(div().text_sm().text_color(paint(MUTED)).child(meta))
                    .child(
                        h_flex()
                            .gap_3()
                            .child(edit)
                            .child(make_default)
                            .child(remove),
                    )
                    .into_any_element(),
            );
        }

        let save_server = pill_button("save-server", "Save server", true, false, cx, |this, cx| {
            this.save_server(cx);
        });
        let logout = pill_button("logout", "Log out", false, false, cx, |this, cx| {
            this.logout(cx);
        });
        let import_claude = pill_button(
            "import-claude",
            "Import Claude export",
            false,
            false,
            cx,
            |this, cx| this.import_archive(cx),
        );
        let add_provider = pill_button(
            "add-provider",
            "Add provider",
            true,
            false,
            cx,
            |this, cx| {
                this.open_new_provider();
                cx.notify();
            },
        );

        v_flex()
            .id("settings")
            .flex_grow(1.)
            .min_h(px(0.))
            .w_full()
            .overflow_y_scroll()
            .py_8()
            .child(
                h_flex().w_full().justify_center().child(
                    v_flex()
                        .w(px(720.))
                        .max_w_full()
                        .px_6()
                        .gap_4()
                        .child(
                            div()
                                .text_2xl()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Settings"),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(avatar(&letter))
                                .child(format!("Signed in as {username}")),
                        )
                        .when_some(banner, |column, text| {
                            column.child(div().text_sm().text_color(paint(DANGER)).child(text))
                        })
                        .child(field_label("Server"))
                        .child(Input::new(&self.url_input))
                        .child(h_flex().gap_2().child(save_server).child(logout))
                        .child(field_label("Access token"))
                        .child(Input::new(&self.session_token_input).mask_toggle())
                        .child(
                            div().text_sm().text_color(paint(MUTED)).child(
                                "Copy this token into a coding agent. It stays in the local SQLite file.",
                            ),
                        )
                        .child(field_label("Claude export"))
                        .child(
                            div().text_sm().text_color(paint(MUTED)).child(
                                "A zip from Claude settings, conversations.json, or a folder of those files. Chats already imported are skipped.",
                            ),
                        )
                        .child(import_claude)
                        .when_some(notice, |column, text| {
                            column.child(div().text_sm().text_color(paint(INK)).child(text))
                        })
                        .child(
                            h_flex()
                                .items_center()
                                .justify_between()
                                .child(div().font_weight(FontWeight::MEDIUM).child("Providers"))
                                .child(add_provider),
                        )
                        .children(providers),
                ),
            )
    }
}
