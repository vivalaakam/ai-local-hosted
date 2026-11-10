use alh_desktop::ProviderEndpoint;
use gpui_kit::component::h_flex;
use gpui_kit::prelude::*;
use gpui_kit::{ClickEvent, Context, FontWeight, SharedString, div, px};
use uuid::Uuid;

use super::ChatWindow;
use super::style::{INK, MUTED, SOFT, WHITE, paint};

pub(crate) fn pill_button<T, Click>(
    id: T,
    label: &'static str,
    filled: bool,
    wide: bool,
    cx: &mut Context<ChatWindow>,
    click: Click,
) -> impl IntoElement + use<T, Click>
where
    T: Into<SharedString>,
    Click: Fn(&mut ChatWindow, &mut Context<ChatWindow>) + 'static,
{
    h_flex()
        .id(id.into())
        .px_4()
        .py_2()
        .rounded(px(999.))
        .justify_center()
        .cursor_pointer()
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .bg(paint(if filled { INK } else { SOFT }))
        .text_color(paint(if filled { WHITE } else { INK }))
        .when(wide, |button| button.w_full())
        .child(label)
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            click(this, cx);
            cx.notify();
        }))
}

pub(crate) fn text_action<T, Click>(
    id: T,
    label: &'static str,
    color: u32,
    cx: &mut Context<ChatWindow>,
    click: Click,
) -> impl IntoElement + use<T, Click>
where
    T: Into<SharedString>,
    Click: Fn(&mut ChatWindow, &mut Context<ChatWindow>) + 'static,
{
    div()
        .id(id.into())
        .text_sm()
        .text_color(paint(color))
        .cursor_pointer()
        .child(label)
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            cx.stop_propagation();
            click(this, cx);
            cx.notify();
        }))
}

pub(crate) fn side_row<Click>(
    id: &'static str,
    label: &'static str,
    active: bool,
    cx: &mut Context<ChatWindow>,
    click: Click,
) -> impl IntoElement + use<Click>
where
    Click: Fn(&mut ChatWindow, &mut Context<ChatWindow>) + 'static,
{
    h_flex()
        .id(id)
        .w_full()
        .px_3()
        .py_2()
        .rounded(px(8.))
        .cursor_pointer()
        .text_sm()
        .when(active, |row| row.bg(paint(SOFT)))
        .child(label)
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            click(this, cx);
            cx.notify();
        }))
}

pub(crate) fn field_label(text: &'static str) -> gpui_kit::Div {
    div().text_sm().text_color(paint(MUTED)).child(text)
}

pub(crate) fn mark() -> impl IntoElement + use<> {
    h_flex()
        .flex_shrink_0()
        .w(px(22.))
        .h(px(22.))
        .rounded(px(6.))
        .justify_center()
        .bg(paint(INK))
        .text_color(paint(WHITE))
        .text_xs()
        .font_weight(FontWeight::BOLD)
        .child("A")
}

pub(crate) fn avatar(letter: &str) -> impl IntoElement + use<> {
    h_flex()
        .flex_shrink_0()
        .w(px(28.))
        .h(px(28.))
        .rounded(px(14.))
        .justify_center()
        .bg(paint(INK))
        .text_color(paint(WHITE))
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .child(letter.to_string())
}

pub(crate) fn initial(name: &str) -> String {
    name.chars()
        .next()
        .map(|ch| ch.to_uppercase().to_string())
        .unwrap_or_else(|| "?".into())
}

pub(crate) fn model_label(id: &str, providers: &[ProviderEndpoint]) -> String {
    let Some((provider, name)) = id.split_once("::") else {
        return id.to_string();
    };
    let Ok(provider_id) = Uuid::parse_str(provider) else {
        return id.to_string();
    };
    let Some(row) = providers.iter().find(|row| row.id == provider_id) else {
        return id.to_string();
    };
    let label = if row.display_name.trim().is_empty() {
        "Untitled"
    } else {
        row.display_name.as_str()
    };
    format!("{label} · {name}")
}

pub(crate) fn valid_slug(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

pub(crate) fn valid_http_url(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty() && !url.contains(' ') && !url.contains('@') && url.len() <= 2000
}
