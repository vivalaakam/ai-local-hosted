mod auth;
mod chat;
mod components;
mod conversation;
mod events;
mod navigation;
mod provider_modal;
mod providers;
mod render;
mod settings;
mod style;

use alh_desktop::{
    BATCH_INTERVAL, Bubble, ConversationItem, DEFAULT_SERVER_URL, DesktopSession, Feed, LocalDb,
    ProviderEndpoint, SessionCreds, UiEvent, draft_key, spawn_session,
};
use gpui_kit::component::Root;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::{
    Bounds, Context, Entity, ScrollHandle, Subscription, TitlebarOptions, Window, WindowBounds,
    WindowOptions, point, px, size,
};
use uuid::Uuid;

use style::apply_chat_theme;

use components::{
    avatar, field_label, initial, mark, model_label, pill_button, side_row, text_action,
    valid_http_url, valid_slug,
};
use render::bubble;

pub(crate) fn run() {
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

pub(crate) struct ChatWindow {
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
}
