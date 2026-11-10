use std::time::Duration;

use alh_desktop::{
    BATCH_INTERVAL, Bubble, Command, ConversationItem, DEFAULT_SERVER_URL, DesktopSession, Feed,
    LocalDb, Login, ProviderDraft, ProviderEndpoint, SessionCreds, UiEvent, draft_key,
    spawn_session,
};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::component::{Root, Theme, ThemeMode, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    Bounds, ClickEvent, Context, Entity, Focusable, FontWeight, ScrollHandle, SharedString,
    Subscription, TitlebarOptions, Window, WindowBounds, WindowOptions, div, point, px, size,
};
use uuid::Uuid;

const WHITE: u32 = 0xffffff;
const INK: u32 = 0x0d0d0d;
const MUTED: u32 = 0x8e8ea0;
const SOFT: u32 = 0xf4f4f4;
const LINE: u32 = 0xececec;
const DANGER: u32 = 0xe02e2a;

const STARTERS: &[(&str, &str)] = &[
    ("Brainstorm", "Help me brainstorm: "),
    ("Code", "Help me write code for: "),
    ("Summarize", "Summarize this:\n"),
    ("Advice", "What should I do about: "),
];

struct ProviderPreset {
    id: &'static str,
    label: &'static str,
    name: &'static str,
    url: &'static str,
    protocol: &'static str,
    token_required: bool,
}

const PROVIDER_PRESETS: &[ProviderPreset] = &[
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

fn known_preset(id: &str) -> &'static ProviderPreset {
    PROVIDER_PRESETS
        .iter()
        .find(|preset| preset.id == id)
        .unwrap_or(&PROVIDER_PRESETS[0])
}

fn preset_for(protocol: &str, url: &str) -> &'static ProviderPreset {
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

fn paint(hex: u32) -> gpui_kit::Hsla {
    gpui_kit::rgb(hex).into()
}

fn apply_chat_theme(window: &mut Window, cx: &mut gpui_kit::App) {
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

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(80.), px(80.)),
                    size: size(px(1280.), px(820.)),
                })),
                titlebar: Some(TitlebarOptions {
                    title: Some("ALH".into()),
                    ..Default::default()
                }),
                window_min_size: Some(size(px(720.), px(480.))),
                ..Default::default()
            };
            cx.open_window(options, |window, cx| {
                apply_chat_theme(window, cx);
                let chat = cx.new(|cx| ChatWindow::new(window, cx));
                chat.update(cx, |view, cx| view.watch(cx));
                cx.new(|cx| Root::new(chat, window, cx))
            })
            .expect("open window");
        });
}

struct ChatWindow {
    db: LocalDb,
    bridge: alh_desktop::Bridge,
    conversations: Vec<ConversationItem>,
    models: Vec<String>,
    providers: Vec<ProviderEndpoint>,
    selected: Option<Uuid>,
    model: String,
    feed: Feed,
    banner: Option<String>,
    notice: Option<String>,
    signed_in: bool,
    username: String,
    settings_open: bool,
    streaming: bool,
    sending: bool,
    composer_text: Option<String>,
    password_text: Option<String>,
    session_token_text: Option<String>,
    provider_form_open: bool,
    model_menu_open: bool,
    editing_provider: Option<Uuid>,
    provider_preset: String,
    provider_protocol: String,
    provider_default: bool,
    provider_enabled: bool,
    provider_name_text: Option<String>,
    provider_url_text: Option<String>,
    provider_token_text: Option<String>,
    url_input: Entity<InputState>,
    username_input: Entity<InputState>,
    password_input: Entity<InputState>,
    session_token_input: Entity<InputState>,
    provider_name_input: Entity<InputState>,
    provider_url_input: Entity<InputState>,
    provider_token_input: Entity<InputState>,
    draft_input: Entity<TextareaState>,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl ChatWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (db, mut banner) = match LocalDb::open(&alh_desktop::default_db_path()) {
            Ok(db) => (db, None),
            Err(err) => (
                LocalDb::open_memory().expect("memory database"),
                Some(err.to_string()),
            ),
        };
        let url = db.server_url().unwrap_or_else(|err| {
            banner = Some(err.to_string());
            DEFAULT_SERVER_URL.to_string()
        });
        let saved_session = match db.session() {
            Ok(session) => session,
            Err(err) => {
                banner = Some(err.to_string());
                None
            }
        };
        let signed_in = saved_session.is_some();
        let username = saved_session
            .as_ref()
            .map(|session| session.username.clone())
            .unwrap_or_default();
        let access_token = saved_session
            .as_ref()
            .map(|session| session.access_token.clone())
            .unwrap_or_default();
        let refresh_token = saved_session
            .as_ref()
            .map(|session| session.refresh_token.clone())
            .unwrap_or_default();
        let access_expires_at = saved_session
            .as_ref()
            .map(|session| session.access_expires_at)
            .unwrap_or(0);
        let conversations = db.cached_conversations().unwrap_or_default();
        let composer_text = db
            .draft(&draft_key(None))
            .ok()
            .filter(|text| !text.is_empty());
        let url_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("http://127.0.0.1:8080");
            state.set_value(url.clone(), window, cx);
            state
        });
        let username_input = cx.new(|cx| InputState::new(window, cx).placeholder("Username"));
        let password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Password")
                .masked(true)
        });
        let session_token_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Access token")
                .masked(true)
        });
        let provider_name_input = cx.new(|cx| InputState::new(window, cx).placeholder("Name"));
        let provider_url_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("http://127.0.0.1:11434/v1"));
        let provider_token_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Provider token")
                .masked(true)
        });
        let draft_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask anything")
                .auto_grow(1, 6)
                .submit_on_enter(true)
        });
        let draft_sub = cx.subscribe(
            &draft_input,
            |this, _, event: &InputEvent, cx| match event {
                InputEvent::Change => this.persist_draft(cx),
                InputEvent::PressEnter { shift: false, .. } if !this.streaming && !this.sending => {
                    this.send(cx);
                }
                _ => {}
            },
        );
        let bridge = spawn_session(
            url,
            DesktopSession {
                username: username.clone(),
                access_token: access_token.clone(),
                refresh_token,
                access_expires_at,
            },
            BATCH_INTERVAL,
        );
        Self {
            db,
            bridge,
            conversations,
            models: Vec::new(),
            providers: Vec::new(),
            selected: None,
            model: String::new(),
            feed: Feed::default(),
            banner,
            notice: None,
            signed_in,
            username,
            settings_open: false,
            streaming: false,
            sending: false,
            composer_text,
            password_text: None,
            session_token_text: signed_in.then_some(access_token),
            provider_form_open: false,
            model_menu_open: false,
            editing_provider: None,
            provider_preset: "ollama".into(),
            provider_protocol: "openai".into(),
            provider_default: true,
            provider_enabled: true,
            provider_name_text: None,
            provider_url_text: None,
            provider_token_text: None,
            url_input,
            username_input,
            password_input,
            session_token_input,
            provider_name_input,
            provider_url_input,
            provider_token_input,
            draft_input,
            scroll: ScrollHandle::new(),
            _subscriptions: vec![draft_sub],
        }
    }

    fn watch(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                if this.update(cx, |view, cx| view.poll(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        while let Some(event) = self.bridge.try_recv() {
            self.apply(event);
            changed = true;
        }
        if changed {
            self.scroll.scroll_to_bottom();
            cx.notify();
        }
    }

    fn apply(&mut self, event: UiEvent) {
        match event {
            UiEvent::Conversations(items) => {
                self.conversations = items;
                let _ = self.db.replace_cache(&self.conversations);
            }
            UiEvent::Models(models) => {
                if !models.iter().any(|name| name == &self.model) {
                    self.model = models.first().cloned().unwrap_or_default();
                }
                self.models = models;
            }
            UiEvent::Providers(providers) => {
                let editing_gone = self
                    .editing_provider
                    .is_some_and(|id| !providers.iter().any(|row| row.id == id));
                self.providers = providers;
                if editing_gone {
                    self.close_provider_form();
                }
            }
            UiEvent::ProviderSaved => {
                self.close_provider_form();
                self.banner = Some("Provider saved".into());
            }
            UiEvent::Opened(detail) => {
                self.upsert(detail.item.clone());
                if self.selected == Some(detail.item.id) || self.selected.is_none() {
                    self.selected = Some(detail.item.id);
                    if !detail.item.model.is_empty() {
                        self.model = detail.item.model.clone();
                    }
                    self.feed.replace_messages(
                        detail
                            .messages
                            .into_iter()
                            .map(|message| Bubble {
                                id: message.id.to_string(),
                                role: message.role,
                                content: message.content,
                                thinking: message.thinking,
                                status: message.status,
                                error_code: message.error_code,
                            })
                            .collect(),
                    );
                    self.streaming = self.feed.streaming_id.is_some();
                }
            }
            UiEvent::Created(item) => {
                let id = item.id;
                if !item.model.is_empty() {
                    self.model = item.model.clone();
                }
                self.upsert(item);
                if self.selected != Some(id) {
                    self.selected = Some(id);
                    self.feed = Feed::default();
                    self.composer_text =
                        Some(self.db.draft(&draft_key(Some(id))).unwrap_or_default());
                }
            }
            UiEvent::Accepted {
                conversation_id,
                user_message_id,
                message_id,
                content,
            } => {
                let _ = self.db.clear_draft(&draft_key(Some(conversation_id)));
                let _ = self.db.clear_draft(&draft_key(None));
                self.selected = Some(conversation_id);
                self.sending = false;
                self.streaming = true;
                self.banner = None;
                self.composer_text = Some(String::new());
                self.feed.accept(
                    Bubble {
                        id: user_message_id.to_string(),
                        role: "user".into(),
                        content,
                        thinking: String::new(),
                        status: "done".into(),
                        error_code: None,
                    },
                    Bubble {
                        id: message_id.to_string(),
                        role: "assistant".into(),
                        content: String::new(),
                        thinking: String::new(),
                        status: "streaming".into(),
                        error_code: None,
                    },
                );
            }
            UiEvent::Batch { message_id, batch } => {
                self.feed.apply(&message_id.to_string(), &batch);
            }
            UiEvent::Finished { .. } => {
                self.streaming = false;
                self.sending = false;
            }
            UiEvent::Stopped { .. } => {
                self.streaming = false;
                self.sending = false;
                if let Some(id) = self.feed.streaming_id.clone() {
                    self.feed.finish(&id, "cancelled");
                }
            }
            UiEvent::Failed { message, accepted } => {
                self.notice = None;
                self.sending = false;
                if accepted {
                    if let Some(id) = self.feed.streaming_id.clone() {
                        self.feed.fail_after_accept(&id, message);
                    } else {
                        self.banner = Some(message);
                    }
                    self.streaming = false;
                } else {
                    self.feed.fail_before_accept(message.clone());
                    self.banner = Some(message);
                }
            }
            UiEvent::SignedIn {
                username,
                access_token,
                refresh_token,
                access_expires_at,
            } => {
                self.signed_in = true;
                self.username = username.clone();
                self.session_token_text = Some(access_token.clone());
                self.password_text = Some(String::new());
                self.banner = None;
                if let Err(err) = self.db.set_session(&SessionCreds {
                    username,
                    access_token,
                    refresh_token,
                    access_expires_at,
                }) {
                    self.banner = Some(err.to_string());
                }
            }
            UiEvent::SignedOut => {
                self.signed_in = false;
                self.settings_open = false;
                self.notice = None;
                self.close_provider_form();
                self.username.clear();
                self.session_token_text = Some(String::new());
                self.conversations.clear();
                self.models.clear();
                self.providers.clear();
                self.selected = None;
                self.feed = Feed::default();
                self.streaming = false;
                self.sending = false;
                let _ = self.db.clear_session();
                let _ = self.db.replace_cache(&[]);
            }
            UiEvent::Notice(message) => {
                self.banner = None;
                self.notice = Some(message);
            }
            UiEvent::Deleted(id) => {
                self.conversations.retain(|item| item.id != id);
                let _ = self.db.replace_cache(&self.conversations);
                let _ = self.db.clear_draft(&draft_key(Some(id)));
                if self.selected == Some(id) {
                    self.selected = None;
                    self.feed = Feed::default();
                    self.composer_text = Some(self.db.draft(&draft_key(None)).unwrap_or_default());
                }
            }
        }
        if let Some(banner) = self.feed.banner.clone() {
            self.banner = Some(banner);
        }
    }

    fn upsert(&mut self, item: ConversationItem) {
        if let Some(existing) = self.conversations.iter_mut().find(|row| row.id == item.id) {
            *existing = item;
        } else {
            self.conversations.insert(0, item);
        }
        let _ = self.db.replace_cache(&self.conversations);
    }

    fn persist_draft(&mut self, cx: &mut Context<Self>) {
        let text = self.draft_input.read(cx).value().to_string();
        self.feed.set_draft(text.clone());
        let _ = self.db.set_draft(&draft_key(self.selected), &text);
    }

    fn select(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.model_menu_open = false;
        if self.selected == Some(id) {
            cx.notify();
            return;
        }
        self.persist_draft(cx);
        if self.streaming || self.sending {
            self.bridge.send(Command::Stop);
            self.streaming = false;
            self.sending = false;
        }
        self.selected = Some(id);
        if let Some(item) = self.conversations.iter().find(|item| item.id == id)
            && !item.model.is_empty()
        {
            self.model = item.model.clone();
        }
        self.composer_text = Some(self.db.draft(&draft_key(Some(id))).unwrap_or_default());
        self.bridge.send(Command::Open(id));
        cx.notify();
    }

    fn new_chat(&mut self, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.model_menu_open = false;
        if self.model.is_empty() {
            self.banner = Some("Pick a model first".into());
            cx.notify();
            return;
        }
        self.persist_draft(cx);
        self.bridge.send(Command::Create {
            model: self.model.clone(),
        });
    }

    fn delete_selected(&mut self) {
        let Some(id) = self.selected else {
            return;
        };
        self.bridge.send(Command::Delete(id));
    }

    fn choose_model(&mut self, model: String) {
        self.model_menu_open = false;
        self.model = model.clone();
        if let Some(id) = self.selected {
            self.bridge.send(Command::SetModel {
                conversation: id,
                model,
            });
        }
    }

    fn send(&mut self, cx: &mut Context<Self>) {
        if self.streaming {
            self.bridge.send(Command::Stop);
            self.streaming = false;
            return;
        }
        let content = self.draft_input.read(cx).value().trim().to_string();
        if content.is_empty() || self.sending {
            return;
        }
        if self.model.is_empty() {
            self.banner = Some("Pick a model first".into());
            cx.notify();
            return;
        }
        self.sending = true;
        self.banner = None;
        self.bridge.send(Command::Send {
            conversation: self.selected,
            content,
            model: self.model.clone(),
        });
        cx.notify();
    }

    fn submit_login(&mut self, register: bool, cx: &mut Context<Self>) {
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

    fn save_server(&mut self, cx: &mut Context<Self>) {
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

    fn logout(&mut self, cx: &mut Context<Self>) {
        self.bridge.send(Command::Logout);
        cx.notify();
    }

    fn close_provider_form(&mut self) {
        let preset = known_preset("ollama");
        self.provider_form_open = false;
        self.editing_provider = None;
        self.provider_preset = preset.id.into();
        self.provider_protocol = preset.protocol.into();
        self.provider_default = !self.providers.iter().any(|row| row.is_default);
        self.provider_enabled = true;
        self.provider_name_text = Some(preset.name.into());
        self.provider_url_text = Some(preset.url.into());
        self.provider_token_text = Some(String::new());
    }

    fn select_preset(&mut self, id: &str, cx: &mut Context<Self>) {
        let next = known_preset(id);
        let current = self.provider_name_input.read(cx).value().trim().to_string();
        let previous_name = known_preset(&self.provider_preset).name;
        if !next.name.is_empty() && (current.is_empty() || current == previous_name) {
            self.provider_name_text = Some(next.name.to_string());
        }
        if next.id != "custom" {
            self.provider_protocol = next.protocol.into();
            self.provider_url_text = Some(next.url.into());
        }
        self.provider_preset = next.id.into();
        cx.notify();
    }

    fn open_new_provider(&mut self) {
        self.close_provider_form();
        self.provider_form_open = true;
    }

    fn edit_provider(&mut self, id: Uuid) {
        let Some(row) = self.providers.iter().find(|row| row.id == id).cloned() else {
            return;
        };
        let preset = preset_for(&row.protocol, &row.url);
        self.provider_form_open = true;
        self.editing_provider = Some(row.id);
        self.provider_preset = preset.id.into();
        self.provider_protocol = row.protocol;
        self.provider_default = row.is_default;
        self.provider_enabled = row.enabled;
        self.provider_name_text = Some(row.display_name);
        self.provider_url_text = Some(row.url);
        self.provider_token_text = Some(String::new());
    }

    fn save_provider(&mut self, cx: &mut Context<Self>) {
        let name = self.provider_name_input.read(cx).value().trim().to_string();
        if name.is_empty() || name.chars().count() > 200 {
            self.banner = Some("Provider name is required".into());
            cx.notify();
            return;
        }
        let preset = known_preset(&self.provider_preset);
        let (url, protocol) = if preset.id == "custom" {
            let url = self.provider_url_input.read(cx).value().trim().to_string();
            if !valid_http_url(&url) {
                self.banner = Some("Provider URL must start with http:// or https://".into());
                cx.notify();
                return;
            }
            let protocol = if self.provider_protocol == "anthropic" {
                "anthropic"
            } else {
                "openai"
            };
            (url, protocol.to_string())
        } else {
            (preset.url.to_string(), preset.protocol.to_string())
        };
        let typed = self
            .provider_token_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if preset.token_required && typed.is_empty() && self.editing_provider.is_none() {
            self.banner = Some("Provider token is required".into());
            cx.notify();
            return;
        }
        let token = if typed.is_empty() { None } else { Some(typed) };
        self.provider_token_text = Some(String::new());
        self.bridge.send(Command::SaveProvider(ProviderDraft {
            id: self.editing_provider,
            protocol,
            url,
            display_name: name,
            token,
            is_default: self.provider_default,
            enabled: self.provider_enabled,
            update: self.editing_provider.is_some(),
            reset_form: true,
        }));
        cx.notify();
    }

    fn delete_provider(&mut self, id: Uuid) {
        self.bridge.send(Command::DeleteProvider(id));
    }

    fn make_default(&mut self, id: Uuid) {
        let Some(row) = self.providers.iter().find(|row| row.id == id).cloned() else {
            return;
        };
        self.bridge.send(Command::SaveProvider(ProviderDraft {
            id: Some(row.id),
            protocol: row.protocol,
            url: row.url,
            display_name: row.display_name,
            token: None,
            is_default: true,
            enabled: row.enabled,
            update: true,
            reset_form: false,
        }));
    }

    fn import_archive(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: Some("Import Claude export".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            if paths.is_empty() {
                return;
            }
            this.update(cx, |this, cx| {
                this.notice = Some("Importing Claude export…".into());
                this.banner = None;
                this.bridge.send(Command::ImportClaude { paths });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn fill_prompt(&mut self, text: &str, cx: &mut Context<Self>) {
        self.composer_text = Some(text.to_string());
        cx.notify();
    }

    fn settings_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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

    fn provider_modal(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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

    fn login_screen(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let banner = self.banner.clone();
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

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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
                this.settings_open = true;
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

    fn chat_column(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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

    fn model_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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

    fn model_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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

    fn empty_chat(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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

    fn thread(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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

    fn composer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let streaming = self.streaming;
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
                        .child(
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
                                })),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .text_center()
                        .text_xs()
                        .text_color(paint(MUTED))
                        .child("AI can make mistakes. Check important information."),
                ),
        )
    }
}

fn pill_button<T, Click>(
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

fn text_action<T, Click>(
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

fn side_row<Click>(
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

fn field_label(text: &'static str) -> gpui_kit::Div {
    div().text_sm().text_color(paint(MUTED)).child(text)
}

fn mark() -> impl IntoElement + use<> {
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

fn avatar(letter: &str) -> impl IntoElement + use<> {
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

fn initial(name: &str) -> String {
    name.chars()
        .next()
        .map(|ch| ch.to_uppercase().to_string())
        .unwrap_or_else(|| "?".into())
}

fn model_label(id: &str, providers: &[ProviderEndpoint]) -> String {
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

fn valid_slug(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn valid_http_url(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty() && !url.contains(' ') && !url.contains('@') && url.len() <= 2000
}

impl Render for ChatWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        div()
            .relative()
            .size_full()
            .bg(paint(WHITE))
            .text_color(paint(INK))
            .child(h_flex().size_full().child(sidebar).child(main))
            .when_some(modal, |root, modal| root.child(modal))
            .into_any_element()
    }
}

fn bubble(message: &Bubble) -> gpui_kit::Div {
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
