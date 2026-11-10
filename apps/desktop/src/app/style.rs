use gpui_kit::Window;
use gpui_kit::component::{Theme, ThemeMode};

pub(crate) const WHITE: u32 = 0xffffff;
pub(crate) const INK: u32 = 0x0d0d0d;
pub(crate) const MUTED: u32 = 0x8e8ea0;
pub(crate) const SOFT: u32 = 0xf4f4f4;
pub(crate) const LINE: u32 = 0xececec;
pub(crate) const DANGER: u32 = 0xe02e2a;

pub(crate) const STARTERS: &[(&str, &str)] = &[
    ("Brainstorm", "Help me brainstorm: "),
    ("Code", "Help me write code for: "),
    ("Summarize", "Summarize this:\n"),
    ("Advice", "What should I do about: "),
];

pub(crate) struct ProviderPreset {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) name: &'static str,
    pub(crate) url: &'static str,
    pub(crate) protocol: &'static str,
    pub(crate) token_required: bool,
}

pub(crate) const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        id: "ollama",
        label: "Ollama",
        name: "Ollama",
        url: "http://127.0.0.1:11434/v1",
        protocol: "openai",
        token_required: false,
    },
    ProviderPreset {
        id: "ollama-cloud",
        label: "Ollama Cloud",
        name: "Ollama Cloud",
        url: "https://ollama.com/v1",
        protocol: "openai",
        token_required: true,
    },
    ProviderPreset {
        id: "opencode",
        label: "OpenCode",
        name: "OpenCode",
        url: "https://opencode.ai/zen/v1",
        protocol: "openai",
        token_required: true,
    },
    ProviderPreset {
        id: "custom",
        label: "Custom",
        name: "",
        url: "",
        protocol: "openai",
        token_required: false,
    },
];

pub(crate) fn known_preset(id: &str) -> &'static ProviderPreset {
    PROVIDER_PRESETS
        .iter()
        .find(|preset| preset.id == id)
        .unwrap_or(&PROVIDER_PRESETS[0])
}

pub(crate) fn preset_for(protocol: &str, url: &str) -> &'static ProviderPreset {
    let url = url.trim().trim_end_matches('/');
    PROVIDER_PRESETS
        .iter()
        .find(|preset| {
            !preset.url.is_empty()
                && preset.protocol == protocol
                && preset.url.trim_end_matches('/') == url
        })
        .unwrap_or_else(|| {
            PROVIDER_PRESETS
                .iter()
                .find(|preset| preset.id == "custom")
                .unwrap_or(&PROVIDER_PRESETS[0])
        })
}

pub(crate) fn paint(hex: u32) -> gpui_kit::Hsla {
    gpui_kit::rgb(hex).into()
}

pub(crate) fn apply_chat_theme(window: &mut Window, cx: &mut gpui_kit::App) {
    Theme::change(ThemeMode::Light, Some(window), cx);
    let colors = &mut cx.global_mut::<Theme>().colors;
    colors.background = paint(WHITE);
    colors.foreground = paint(INK);
    colors.border = paint(LINE);
    colors.input = paint(WHITE);
    colors.muted = paint(SOFT);
    colors.muted_foreground = paint(MUTED);
    colors.primary = paint(INK);
    colors.primary_foreground = paint(WHITE);
    colors.accent = paint(SOFT);
    colors.accent_foreground = paint(INK);
    colors.popover = paint(WHITE);
    colors.popover_foreground = paint(INK);
    colors.sidebar = paint(WHITE);
    colors.sidebar_foreground = paint(INK);
    colors.sidebar_border = paint(LINE);
    colors.sidebar_accent = paint(SOFT);
    colors.sidebar_accent_foreground = paint(INK);
    colors.danger = paint(DANGER);
    colors.danger_foreground = paint(WHITE);
}
