use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, FontWeight, div, px};

use super::style::{INK, MUTED, WHITE, paint};
use super::{ChatWindow, field_label, pill_button};

impl ChatWindow {
    pub(crate) fn tool_modal(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let readonly = self.tool_form_readonly;
        let editing = self.editing_tool.is_some();
        let enabled = self.tool_enabled;
        let title = if readonly {
            "Tool details"
        } else if editing {
            "Edit tool"
        } else {
            "New tool"
        };
        let enabled_toggle = pill_button(
            "tool-enabled-toggle",
            if enabled { "Enabled" } else { "Disabled" },
            enabled,
            false,
            cx,
            |this, cx| {
                if !this.tool_form_readonly {
                    this.tool_enabled = !this.tool_enabled;
                    cx.notify();
                }
            },
        );
        let save = pill_button("tool-save", "Save", true, false, cx, |this, cx| {
            this.save_tool(cx);
        });
        let cancel = pill_button("tool-cancel", "Close", false, false, cx, |this, cx| {
            this.close_tool_form();
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
                    .id("tool-form")
                    .w(px(520.))
                    .max_w_full()
                    .max_h(px(720.))
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
                            .child(title),
                    )
                    .when(readonly, |panel| {
                        panel.child(
                            div()
                                .text_sm()
                                .text_color(paint(MUTED))
                                .child("System tools are read-only in the app."),
                        )
                    })
                    .child(field_label("Slug"))
                    .child(Input::new(&self.tool_slug_input).disabled(readonly || editing))
                    .child(field_label("Name"))
                    .child(Input::new(&self.tool_name_input).disabled(readonly))
                    .child(field_label("Description"))
                    .child(Input::new(&self.tool_description_input).disabled(readonly))
                    .child(field_label("Parameters (JSON schema)"))
                    .child(
                        Textarea::new(&self.tool_parameters_input)
                            .appearance(true)
                            .disabled(readonly),
                    )
                    .child(field_label("Source (Rhai)"))
                    .child(
                        Textarea::new(&self.tool_source_input)
                            .appearance(true)
                            .disabled(readonly),
                    )
                    .when(!readonly, |panel| panel.child(enabled_toggle))
                    .child(
                        h_flex()
                            .gap_2()
                            .when(!readonly, |row| row.child(save))
                            .child(cancel),
                    ),
            )
    }
}
