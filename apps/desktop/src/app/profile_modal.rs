use gpui_kit::component::input::Input;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{ClickEvent, Context, FontWeight, SharedString, div, px};

use super::style::{DANGER, INK, SOFT, WHITE, paint};
use super::{ChatWindow, field_label, pill_button};

impl ChatWindow {
    pub(crate) fn profile_menu(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let mut profiles: Vec<gpui_kit::AnyElement> = Vec::new();
        for profile in &self.profiles {
            let id = profile.id;
            let active = id == self.active_profile.id;
            let label = if active {
                format!("{} · current", profile.name)
            } else {
                profile.name.clone()
            };
            profiles.push(
                div()
                    .id(SharedString::from(format!("profile-picker-{id}")))
                    .w_full()
                    .px_3()
                    .py_2()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .bg(paint(if active { SOFT } else { WHITE }))
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.profile_menu_open = false;
                        this.switch_profile(id, cx);
                    }))
                    .into_any_element(),
            );
        }
        let add = pill_button("profile-add", "Add profile", true, true, cx, |this, cx| {
            this.profile_menu_open = false;
            this.profile_form_open = true;
            this.profile_name_text = Some(String::new());
            this.profile_url_text = Some(String::new());
            cx.notify();
        });
        let close = pill_button("profile-close", "Close", false, true, cx, |this, cx| {
            this.profile_menu_open = false;
            cx.notify();
        });
        overlay("profile-menu", "Choose a profile", cx, move |body| {
            body.children(profiles).child(add).child(close)
        })
    }

    pub(crate) fn profile_form(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let banner = self.banner.clone();
        let save = pill_button(
            "profile-save",
            "Add profile",
            true,
            false,
            cx,
            |this, cx| {
                this.save_profile(cx);
            },
        );
        let cancel = pill_button("profile-cancel", "Cancel", false, false, cx, |this, cx| {
            this.profile_form_open = false;
            cx.notify();
        });
        overlay("profile-form", "Add server profile", cx, move |body| {
            body.child(field_label("Name"))
                .child(Input::new(&self.profile_name_input))
                .child(field_label("Server URL"))
                .child(Input::new(&self.profile_url_input))
                .when_some(banner, |body, text| {
                    body.child(div().text_sm().text_color(paint(DANGER)).child(text))
                })
                .child(h_flex().gap_2().child(save).child(cancel))
        })
    }
}

fn overlay(
    id: &'static str,
    title: &'static str,
    cx: &mut Context<ChatWindow>,
    content: impl FnOnce(gpui_kit::Div) -> gpui_kit::Div,
) -> gpui_kit::AnyElement {
    div()
        .id(id)
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui_kit::rgba(0x00000066))
        .child(content(
            v_flex()
                .w(px(420.))
                .max_w_full()
                .p_6()
                .gap_3()
                .rounded(px(12.))
                .bg(paint(WHITE))
                .text_color(paint(INK))
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                ),
        ))
        .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.stop_propagation()))
        .into_any_element()
}
