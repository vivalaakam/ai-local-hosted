use gpui_kit::component::input::Input;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, FontWeight, SharedString, div, px};

use super::style::{INK, MUTED, PROVIDER_PRESETS, WHITE, known_preset, paint};
use super::{ChatWindow, field_label, pill_button};

impl ChatWindow {
    pub(crate) fn provider_modal(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let preset = known_preset(&self.provider_preset);
        let custom = preset.id == "custom";
        let protocol = self.provider_protocol.clone();
        let provider_default = self.provider_default;
        let provider_enabled = self.provider_enabled;
        let editing = self.editing_provider.is_some();
        let fixed_url = preset.url;
        let token_hint = if editing {
            "Leave the token empty to keep the saved one."
        } else if preset.token_required {
            "An API token is required."
        } else if preset.id == "ollama" {
            "A token is optional for local Ollama."
        } else {
            "A token is optional."
        };
        let mut chips: Vec<gpui_kit::AnyElement> = Vec::new();
        for item in PROVIDER_PRESETS {
            let id = item.id;
            chips.push(
                pill_button(
                    SharedString::from(format!("preset-{id}")),
                    item.label,
                    self.provider_preset == id,
                    false,
                    cx,
                    move |this, cx| this.select_preset(id, cx),
                )
                .into_any_element(),
            );
        }
        let openai = pill_button(
            "protocol-openai",
            "OpenAI",
            protocol == "openai",
            false,
            cx,
            |this, cx| {
                this.provider_protocol = "openai".into();
                cx.notify();
            },
        );
        let anthropic = pill_button(
            "protocol-anthropic",
            "Anthropic",
            protocol == "anthropic",
            false,
            cx,
            |this, cx| {
                this.provider_protocol = "anthropic".into();
                cx.notify();
            },
        );
        let default_toggle = pill_button(
            "provider-default-toggle",
            "Default",
            provider_default,
            false,
            cx,
            |this, cx| {
                this.provider_default = !this.provider_default;
                cx.notify();
            },
        );
        let enabled_toggle = pill_button(
            "provider-enabled-toggle",
            "Enabled",
            provider_enabled,
            false,
            cx,
            |this, cx| {
                this.provider_enabled = !this.provider_enabled;
                cx.notify();
            },
        );
        let save = pill_button("provider-save", "Save", true, false, cx, |this, cx| {
            this.save_provider(cx);
        });
        let cancel = pill_button("provider-cancel", "Cancel", false, false, cx, |this, cx| {
            this.close_provider_form();
            cx.notify();
        });

        div()
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui_kit::hsla(0., 0., 0., 0.32))
            .child(
                v_flex()
                    .id("provider-form")
                    .w(px(440.))
                    .max_w_full()
                    .max_h(px(640.))
                    .overflow_y_scroll()
                    .p_5()
                    .gap_3()
                    .rounded(px(16.))
                    .bg(paint(WHITE))
                    .text_color(paint(INK))
                    .shadow_lg()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(if editing {
                                "Edit provider"
                            } else {
                                "New provider"
                            }),
                    )
                    .child(field_label("Preset"))
                    .child(h_flex().flex_wrap().gap_2().children(chips))
                    .child(field_label("Name"))
                    .child(Input::new(&self.provider_name_input))
                    .when(custom, |panel| {
                        panel
                            .child(field_label("URL"))
                            .child(Input::new(&self.provider_url_input))
                            .child(h_flex().gap_2().child(openai).child(anthropic))
                            .when(protocol == "anthropic", |inner| {
                                inner.child(
                                    div()
                                        .text_sm()
                                        .text_color(paint(MUTED))
                                        .child("Stored only. Calls are not sent yet."),
                                )
                            })
                    })
                    .when(!custom, |panel| {
                        panel.child(
                            div()
                                .text_sm()
                                .text_color(paint(MUTED))
                                .child(format!("{fixed_url} · OpenAI")),
                        )
                    })
                    .child(field_label("Token"))
                    .child(Input::new(&self.provider_token_input).mask_toggle())
                    .child(div().text_sm().text_color(paint(MUTED)).child(token_hint))
                    .child(h_flex().gap_2().child(default_toggle).child(enabled_toggle))
                    .child(h_flex().gap_2().child(save).child(cancel)),
            )
    }
}
