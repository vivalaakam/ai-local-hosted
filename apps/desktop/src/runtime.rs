use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::api::{Api, AuthTokens, ChatDetail, ImportResult, ProviderDraft, ProviderEndpoint};
use crate::batch::{DeltaBatch, TextBatch};
use crate::sse::{SseBuffer, StreamPiece};
use crate::store::ConversationItem;

/// Work the window asks the network thread to do.
#[derive(Debug, Clone)]
pub enum Command {
    Reload,
    Open(Uuid),
    Create {
        model: String,
    },
    Send {
        conversation: Option<Uuid>,
        content: String,
        model: String,
    },
    Stop,
    SetModel {
        conversation: Uuid,
        model: String,
    },
    Delete(Uuid),
    Login(Login),
    Logout,
    SetServer {
        base: String,
    },
    SaveProvider(ProviderDraft),
    DeleteProvider(Uuid),
    ImportClaude {
        paths: Vec<PathBuf>,
    },
}

/// Sign-in request. The password is not included in Debug output.
#[derive(Clone)]
pub struct Login {
    pub base: String,
    pub username: String,
    pub password: String,
    pub register: bool,
}

impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Login")
            .field("base", &self.base)
            .field("username", &self.username)
            .field("register", &self.register)
            .finish()
    }
}

/// Tokens loaded from the desktop SQLite file, or an empty session.
#[derive(Debug, Clone)]
pub struct DesktopSession {
    pub username: String,
    pub access_token: String,
    pub refresh_token: String,
    pub access_expires_at: i64,
}

/// What the window should apply. Deltas arrive already batched.
#[derive(Debug, Clone)]
pub enum UiEvent {
    Conversations(Vec<ConversationItem>),
    Models(Vec<String>),
    Providers(Vec<ProviderEndpoint>),
    ProviderSaved,
    Opened(ChatDetail),
    Created(ConversationItem),
    Accepted {
        conversation_id: Uuid,
        user_message_id: Uuid,
        message_id: Uuid,
        content: String,
    },
    Batch {
        message_id: Uuid,
        batch: TextBatch,
    },
    Finished {
        conversation_id: Uuid,
    },
    Stopped {
        conversation_id: Uuid,
    },
    Failed {
        message: String,
        accepted: bool,
    },
    Deleted(Uuid),
    SignedIn {
        username: String,
        access_token: String,
        refresh_token: String,
        access_expires_at: i64,
    },
    SignedOut,
    Notice(String),
}

enum PumpEnd {
    Done,
    Dropped,
    Error(String),
}

enum PumpMsg {
    Batch {
        turn: u64,
        message_id: Uuid,
        batch: TextBatch,
    },
    Ended {
        turn: u64,
        conversation_id: Uuid,
        end: PumpEnd,
    },
    Expired {
        turn: u64,
    },
}

struct Live {
    turn: u64,
    handle: JoinHandle<()>,
}

/// Handle the window uses. Commands are non-blocking. Events are polled.
pub struct Bridge {
    commands: UnboundedSender<Command>,
    events: Receiver<UiEvent>,
}

impl Bridge {
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub fn try_recv(&self) -> Option<UiEvent> {
        self.events.try_recv().ok()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<UiEvent, RecvTimeoutError> {
        self.events.recv_timeout(timeout)
    }
}

pub fn spawn_bridge(base: String, token: String, batch_interval: Duration) -> Bridge {
    spawn_session(
        base,
        DesktopSession {
            username: String::new(),
            access_token: token,
            refresh_token: String::new(),
            access_expires_at: i64::MAX,
        },
        batch_interval,
    )
}

pub fn spawn_session(base: String, creds: DesktopSession, batch_interval: Duration) -> Bridge {
    let (commands, command_rx) = unbounded_channel();
    let (event_tx, events) = mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_io()
            .enable_time()
            .build()
            .expect("tokio runtime");
        runtime.block_on(run(base, creds, batch_interval, command_rx, event_tx));
    });
    Bridge { commands, events }
}

async fn run(
    base: String,
    creds: DesktopSession,
    batch_interval: Duration,
    mut command_rx: UnboundedReceiver<Command>,
    event_tx: Sender<UiEvent>,
) {
    let Ok(mut session) = Session::new(base, creds, batch_interval, event_tx) else {
        return;
    };
    if session.signed_in {
        if session.access_expired() {
            if session.try_refresh().await {
                session.reload().await;
            }
        } else {
            session.reload().await;
        }
    }
    let (pump_tx, mut pump_rx) = unbounded_channel();
    loop {
        tokio::select! {
            command = command_rx.recv() => {
                let Some(command) = command else { break };
                session.command(command, &pump_tx).await;
            }
            message = pump_rx.recv() => {
                let Some(message) = message else { break };
                session.on_pump(message).await;
            }
        }
    }
}

struct Session {
    api: Api,
    username: String,
    refresh_token: String,
    access_expires_at: i64,
    signed_in: bool,
    batch_interval: Duration,
    events: Sender<UiEvent>,
    live: Option<Live>,
    next_turn: u64,
}

impl Session {
    fn new(
        base: String,
        creds: DesktopSession,
        batch_interval: Duration,
        events: Sender<UiEvent>,
    ) -> Result<Self, crate::api::ApiError> {
        let signed_in = !creds.access_token.trim().is_empty();
        Ok(Self {
            api: Api::new(base, creds.access_token)?,
            username: creds.username,
            refresh_token: creds.refresh_token,
            access_expires_at: creds.access_expires_at,
            signed_in,
            batch_interval,
            events,
            live: None,
            next_turn: 1,
        })
    }

    fn emit(&self, event: UiEvent) {
        let _ = self.events.send(event);
    }

    fn stop_live(&mut self) {
        if let Some(live) = self.live.take() {
            live.handle.abort();
        }
    }

    async fn command(&mut self, command: Command, pump_tx: &UnboundedSender<PumpMsg>) {
        match command {
            Command::Reload => self.reload().await,
            Command::Open(id) => {
                if let Some(detail) = self
                    .call(false, async |api| api.get_conversation(id).await)
                    .await
                {
                    self.emit(UiEvent::Opened(detail));
                }
            }
            Command::Create { model } => {
                if let Some(item) = self
                    .call(false, async |api| api.create_conversation(&model).await)
                    .await
                {
                    self.emit(UiEvent::Created(item));
                }
            }
            Command::Send {
                conversation,
                content,
                model,
            } => {
                if !self.signed_in {
                    self.fail(connect_first(), false);
                    return;
                }
                if self.access_expired() && !self.try_refresh().await {
                    return;
                }
                self.start_send(conversation, content, model, pump_tx);
            }
            Command::Stop => self.stop_live(),
            Command::SetModel {
                conversation,
                model,
            } => {
                if let Some(item) = self
                    .call(false, async |api| api.set_model(conversation, &model).await)
                    .await
                {
                    self.emit(UiEvent::Created(item));
                }
            }
            Command::Delete(id) => {
                self.stop_live();
                if self
                    .call(false, async |api| api.delete_conversation(id).await)
                    .await
                    .is_some()
                {
                    self.emit(UiEvent::Deleted(id));
                }
            }
            Command::Login(login) => self.sign_in(login).await,
            Command::Logout => self.sign_out(),
            Command::SetServer { base } => match Api::new(base, String::new()) {
                Ok(api) => {
                    self.stop_live();
                    self.api = api;
                    self.signed_in = false;
                    self.refresh_token.clear();
                    self.username.clear();
                    self.access_expires_at = 0;
                    self.emit(UiEvent::SignedOut);
                }
                Err(err) => self.fail(err, false),
            },
            Command::SaveProvider(draft) => {
                let reset_form = draft.reset_form;
                if self
                    .call(false, async |api| api.save_provider(&draft).await)
                    .await
                    .is_some()
                {
                    if reset_form {
                        self.emit(UiEvent::ProviderSaved);
                    }
                    self.reload().await;
                }
            }
            Command::DeleteProvider(id) => {
                if self
                    .call(false, async |api| api.delete_provider(id).await)
                    .await
                    .is_some()
                {
                    self.reload().await;
                }
            }
            Command::ImportClaude { paths } => self.import_claude(paths).await,
        }
    }

    async fn import_claude(&mut self, paths: Vec<PathBuf>) {
        if !self.signed_in {
            self.fail(connect_first(), false);
            return;
        }
        let files = match import_files(paths) {
            Ok(files) => files,
            Err(message) => {
                self.fail(import_error(message), false);
                return;
            }
        };
        let mut counts = ImportResult {
            conversations: 0,
            skipped: 0,
            messages: 0,
        };
        for path in files {
            let bytes = match tokio::fs::read(&path).await {
                Ok(bytes) => bytes,
                Err(err) => {
                    self.report_import(
                        counts,
                        Some(format!("could not read {}: {err}", path.display())),
                    )
                    .await;
                    return;
                }
            };
            if bytes.len() > 512 * 1024 * 1024 {
                self.report_import(counts, Some("Claude export is larger than 512 MB".into()))
                    .await;
                return;
            }
            match self
                .call(false, async |api| api.import_claude(bytes.clone()).await)
                .await
            {
                Some(part) => {
                    counts.conversations += part.conversations;
                    counts.skipped += part.skipped;
                    counts.messages += part.messages;
                }
                None => {
                    if counts.conversations > 0 || counts.skipped > 0 {
                        self.reload().await;
                    }
                    return;
                }
            }
        }
        self.report_import(counts, None).await;
    }

    async fn report_import(&mut self, counts: ImportResult, error: Option<String>) {
        if counts.conversations > 0 || counts.skipped > 0 || counts.messages > 0 {
            self.reload().await;
        }
        if let Some(message) = error {
            if counts.conversations == 0 && counts.skipped == 0 {
                self.fail(import_error(message), false);
            } else {
                self.emit(UiEvent::Notice(format!(
                    "Imported {} conversations and {} messages, then stopped: {message}",
                    counts.conversations, counts.messages
                )));
            }
            return;
        }
        self.emit(UiEvent::Notice(format!(
            "Imported {} conversations and {} messages. Skipped {} already saved.",
            counts.conversations, counts.messages, counts.skipped
        )));
    }

    async fn sign_in(&mut self, login: Login) {
        self.stop_live();
        let api = match Api::new(login.base, String::new()) {
            Ok(api) => api,
            Err(err) => {
                self.fail(err, false);
                return;
            }
        };
        let result = if login.register {
            api.register(&login.username, &login.password).await
        } else {
            api.login(&login.username, &login.password).await
        };
        match result {
            Ok(tokens) => {
                self.api = api;
                self.username = login.username;
                self.store_tokens(tokens);
                self.reload().await;
            }
            Err(err) => self.fail(err, false),
        }
    }

    fn store_tokens(&mut self, tokens: AuthTokens) {
        let access_expires_at = now_unix().saturating_add(tokens.expires_in);
        self.api.set_token(tokens.access_token.clone());
        self.refresh_token = tokens.refresh_token.clone();
        self.access_expires_at = access_expires_at;
        self.signed_in = true;
        self.emit(UiEvent::SignedIn {
            username: self.username.clone(),
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            access_expires_at,
        });
    }

    fn sign_out(&mut self) {
        self.stop_live();
        self.api.set_token(String::new());
        self.refresh_token.clear();
        self.username.clear();
        self.access_expires_at = 0;
        self.signed_in = false;
        self.emit(UiEvent::SignedOut);
    }

    fn access_expired(&self) -> bool {
        self.access_expires_at <= now_unix()
    }

    async fn try_refresh(&mut self) -> bool {
        if self.refresh_token.is_empty() {
            self.sign_out();
            return false;
        }
        match self.api.refresh(&self.refresh_token).await {
            Ok(tokens) => {
                self.store_tokens(tokens);
                true
            }
            Err(_) => {
                self.sign_out();
                false
            }
        }
    }

    async fn call<T>(
        &mut self,
        accepted: bool,
        op: impl AsyncFn(&Api) -> Result<T, crate::api::ApiError>,
    ) -> Option<T> {
        if !self.signed_in {
            self.fail(connect_first(), false);
            return None;
        }
        if self.access_expired() && !self.try_refresh().await {
            return None;
        }
        match op(&self.api).await {
            Ok(value) => Some(value),
            Err(err) if err.status == 401 && self.try_refresh().await => {
                match op(&self.api).await {
                    Ok(value) => Some(value),
                    Err(err) => {
                        self.fail(err, accepted);
                        None
                    }
                }
            }
            Err(err) => {
                self.fail(err, accepted);
                None
            }
        }
    }

    async fn reload(&mut self) {
        let Some(items) = self
            .call(false, async |api| api.list_conversations().await)
            .await
        else {
            return;
        };
        self.emit(UiEvent::Conversations(items));
        if let Some(models) = self.call(false, async |api| api.list_models().await).await {
            self.emit(UiEvent::Models(models));
        }
        if let Some(providers) = self
            .call(false, async |api| api.list_providers().await)
            .await
        {
            self.emit(UiEvent::Providers(providers));
        }
    }

    fn start_send(
        &mut self,
        conversation: Option<Uuid>,
        content: String,
        model: String,
        pump_tx: &UnboundedSender<PumpMsg>,
    ) {
        self.stop_live();
        let turn = self.next_turn;
        self.next_turn = self.next_turn.wrapping_add(1);
        let api = self.api.clone();
        let events = self.events.clone();
        let pump_tx = pump_tx.clone();
        let interval = self.batch_interval;
        let handle = tokio::spawn(async move {
            let conversation_id = match conversation {
                Some(id) => id,
                None => match api.create_conversation(&model).await {
                    Ok(item) => {
                        let _ = events.send(UiEvent::Created(item.clone()));
                        item.id
                    }
                    Err(err) if err.status == 401 => {
                        let _ = pump_tx.send(PumpMsg::Expired { turn });
                        return;
                    }
                    Err(err) => {
                        let _ = events.send(UiEvent::Failed {
                            message: err.message,
                            accepted: false,
                        });
                        return;
                    }
                },
            };
            match api.send_message(conversation_id, &content, &model).await {
                Ok(live) => {
                    let message_id = live.message_id;
                    let user_message_id = live.user_message_id;
                    let _ = events.send(UiEvent::Accepted {
                        conversation_id,
                        user_message_id,
                        message_id,
                        content,
                    });
                    pump(live, turn, interval, pump_tx).await;
                }
                Err(err) if err.status == 401 => {
                    let _ = pump_tx.send(PumpMsg::Expired { turn });
                }
                Err(err) => {
                    let _ = events.send(UiEvent::Failed {
                        message: err.message,
                        accepted: false,
                    });
                }
            }
        });
        self.live = Some(Live { turn, handle });
    }

    fn fail(&self, err: crate::api::ApiError, accepted: bool) {
        self.emit(UiEvent::Failed {
            message: err.message,
            accepted,
        });
    }

    async fn on_pump(&mut self, message: PumpMsg) {
        match message {
            PumpMsg::Batch {
                turn,
                message_id,
                batch,
            } => {
                if self.live.as_ref().is_some_and(|live| live.turn == turn) {
                    self.emit(UiEvent::Batch { message_id, batch });
                }
            }
            PumpMsg::Expired { turn } => {
                if self.live.as_ref().is_some_and(|live| live.turn != turn) {
                    return;
                }
                self.live = None;
                if self.try_refresh().await {
                    self.fail(
                        crate::api::ApiError {
                            status: 401,
                            code: "unauthorized".into(),
                            message: "Session refreshed. Send the message again.".into(),
                        },
                        false,
                    );
                }
            }
            PumpMsg::Ended {
                turn,
                conversation_id,
                end,
            } => {
                if self.live.as_ref().is_some_and(|live| live.turn != turn) {
                    return;
                }
                self.live = None;
                let done = matches!(end, PumpEnd::Done);
                match end {
                    PumpEnd::Error(message) => self.emit(UiEvent::Failed {
                        message,
                        accepted: true,
                    }),
                    PumpEnd::Dropped => {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        self.emit(UiEvent::Stopped { conversation_id });
                    }
                    PumpEnd::Done => {}
                }
                if let Ok(detail) = wait_until_settled(&self.api, conversation_id).await {
                    self.emit(UiEvent::Opened(detail));
                }
                if done {
                    self.emit(UiEvent::Finished { conversation_id });
                }
            }
        }
    }
}

async fn wait_until_settled(api: &Api, id: Uuid) -> Result<ChatDetail, crate::api::ApiError> {
    let mut latest = None;
    for _ in 0..8 {
        let detail = api.get_conversation(id).await?;
        let streaming = detail
            .messages
            .iter()
            .any(|message| message.status == "streaming");
        if !streaming {
            return Ok(detail);
        }
        latest = Some(detail);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    latest.ok_or_else(|| crate::api::ApiError {
        status: 0,
        code: "interrupted".into(),
        message: "conversation did not settle".into(),
    })
}

async fn pump(
    mut live: crate::api::LiveTurn,
    turn: u64,
    interval: Duration,
    tx: UnboundedSender<PumpMsg>,
) {
    let mut guard = EndGuard {
        tx: tx.clone(),
        turn,
        conversation_id: live.conversation_id,
        end: PumpEnd::Dropped,
    };
    let mut sse = SseBuffer::new();
    let mut batch = DeltaBatch::new(interval, Instant::now());
    let mut deadline: Option<tokio::time::Instant> = None;
    loop {
        let idle = async {
            match deadline {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            chunk = live.chunk() => {
                match chunk {
                    Ok(Some(bytes)) => {
                        let text = String::from_utf8_lossy(&bytes);
                        for piece in sse.push(&text) {
                            if matches!(piece, StreamPiece::Done) {
                                flush_batch(&mut batch, turn, live.message_id, &tx);
                                guard.end = PumpEnd::Done;
                                return;
                            }
                            batch.push(&piece);
                            if deadline.is_none() {
                                deadline = Some(tokio::time::Instant::now() + interval);
                            }
                        }
                    }
                    Ok(None) => {
                        flush_batch(&mut batch, turn, live.message_id, &tx);
                        guard.end = PumpEnd::Done;
                        return;
                    }
                    Err(err) => {
                        flush_batch(&mut batch, turn, live.message_id, &tx);
                        guard.end = PumpEnd::Error(err.message);
                        return;
                    }
                }
            }
            _ = idle => {
                flush_batch(&mut batch, turn, live.message_id, &tx);
                deadline = None;
            }
        }
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn import_error(message: String) -> crate::api::ApiError {
    crate::api::ApiError {
        status: 0,
        code: "invalid_request".into(),
        message,
    }
}

fn import_files(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            let entries = std::fs::read_dir(&path)
                .map_err(|err| format!("could not read {}: {err}", path.display()))?;
            let mut found = Vec::new();
            for entry in entries {
                let entry =
                    entry.map_err(|err| format!("could not read {}: {err}", path.display()))?;
                let child = entry.path();
                if !child.is_file() {
                    continue;
                }
                let name = child
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if name.starts_with('.') {
                    continue;
                }
                if name.contains("conversation")
                    && (name.ends_with(".json") || name.ends_with(".zip"))
                {
                    found.push(child);
                }
            }
            found.sort();
            if found.is_empty() {
                return Err(format!("No conversations archive in {}", path.display()));
            }
            files.extend(found);
        } else {
            files.push(path);
        }
    }
    if files.is_empty() {
        return Err("Choose a Claude export archive.".into());
    }
    Ok(files)
}

fn connect_first() -> crate::api::ApiError {
    crate::api::ApiError {
        status: 0,
        code: "unauthorized".into(),
        message: "Connect to the server first".into(),
    }
}

fn flush_batch(batch: &mut DeltaBatch, turn: u64, message_id: Uuid, tx: &UnboundedSender<PumpMsg>) {
    if let Some(batch) = batch.take_pending(Instant::now()) {
        let _ = tx.send(PumpMsg::Batch {
            turn,
            message_id,
            batch,
        });
    }
}

struct EndGuard {
    tx: UnboundedSender<PumpMsg>,
    turn: u64,
    conversation_id: Uuid,
    end: PumpEnd,
}

impl Drop for EndGuard {
    fn drop(&mut self) {
        let end = std::mem::replace(&mut self.end, PumpEnd::Dropped);
        let _ = self.tx.send(PumpMsg::Ended {
            turn: self.turn,
            conversation_id: self.conversation_id,
            end,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::BATCH_INTERVAL;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const CHAT: &str = "11111111-1111-1111-1111-111111111111";
    const MESSAGE: &str = "22222222-2222-2222-2222-222222222222";
    const USER_MESSAGE: &str = "33333333-3333-3333-3333-333333333333";

    #[test]
    fn streamed_send_batches_text_and_reloads_the_saved_turn() {
        let base = serve(Script::Stream);
        let bridge = spawn_bridge(base, "test-token".into(), Duration::from_millis(15));
        bridge.send(Command::Send {
            conversation: None,
            content: "hello there".into(),
            model: "stub-model".into(),
        });
        let mut text = String::new();
        let mut saved = false;
        let mut finished = false;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(4) {
            let Ok(event) = bridge.recv_timeout(Duration::from_millis(200)) else {
                continue;
            };
            match event {
                UiEvent::Batch { batch, .. } => text.push_str(&batch.text),
                UiEvent::Opened(detail) => {
                    saved = detail
                        .messages
                        .iter()
                        .any(|message| message.content == "hello");
                }
                UiEvent::Finished { .. } => finished = true,
                UiEvent::Failed { message, .. } => panic!("{message}"),
                _ => {}
            }
            if finished && saved && text == "hello" {
                return;
            }
        }
        panic!("stream did not finish: text={text:?} saved={saved} finished={finished}");
    }

    #[test]
    fn rejected_send_reports_the_server_message() {
        let base = serve(Script::Reject);
        let bridge = spawn_bridge(base, "test-token".into(), BATCH_INTERVAL);
        bridge.send(Command::Send {
            conversation: Some(Uuid::parse_str(CHAT).unwrap()),
            content: "nope".into(),
            model: "stub-model".into(),
        });
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(3) {
            let Ok(event) = bridge.recv_timeout(Duration::from_millis(200)) else {
                continue;
            };
            if let UiEvent::Failed { message, accepted } = event {
                assert!(!accepted);
                assert_eq!(message, "busy");
                return;
            }
        }
        panic!("missing failure");
    }

    #[derive(Clone, Copy)]
    enum Script {
        Stream,
        Reject,
    }

    fn serve(script: Script) -> String {
        let (ready, address) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                ready.send(listener.local_addr().unwrap()).unwrap();
                loop {
                    let Ok((mut socket, _)) = listener.accept().await else {
                        break;
                    };
                let script = script;
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buf = [0_u8; 2048];
                    loop {
                        let n = socket.read(&mut buf).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        request.extend_from_slice(&buf[..n]);
                        if request.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let text = String::from_utf8_lossy(&request);
                    let line = text.lines().next().unwrap_or("");
                    if !text.to_ascii_lowercase().contains("authorization: bearer test-token")
                    {
                        let _ = write_json(&mut socket, 401, r#"{"error":{"code":"unauthorized","message":"unauthorized"}}"#).await;
                        return;
                    }
                    if line.starts_with("GET /api/providers") {
                        let _ = write_json(&mut socket, 200, r#"{"data":[]}"#).await;
                    } else if line.starts_with("GET /v1/models") {
                        let _ = write_json(
                            &mut socket,
                            200,
                            r#"{"object":"list","data":[{"id":"stub-model","object":"model"}]}"#,
                        )
                        .await;
                    } else if line.starts_with("GET /api/conversations/") {
                        let body = format!(
                            r#"{{"id":"{CHAT}","title":"hello there","model":"stub-model","updated_at":"2026-11-08T00:00:00Z","messages":[{{"id":"{MESSAGE}","role":"assistant","content":"hello","status":"done"}}]}}"#
                        );
                        let _ = write_json(&mut socket, 200, &body).await;
                    } else if line.starts_with("GET /api/conversations") {
                        let _ = write_json(&mut socket, 200, r#"{"data":[],"next_cursor":null}"#).await;
                    } else if line.starts_with("POST /api/conversations/") {
                        match script {
                            Script::Reject => {
                                let _ = write_json(
                                    &mut socket,
                                    409,
                                    r#"{"error":{"code":"conflict","message":"busy"}}"#,
                                )
                                .await;
                            }
                            Script::Stream => write_stream(&mut socket).await,
                        }
                    } else if line.starts_with("POST /api/conversations") {
                        let body = format!(
                            r#"{{"id":"{CHAT}","title":"","model":"stub-model","updated_at":"2026-11-08T00:00:00Z"}}"#
                        );
                        let _ = write_json(&mut socket, 201, &body).await;
                    } else {
                        let _ = write_json(
                            &mut socket,
                            404,
                            r#"{"error":{"code":"not_found","message":"no such route"}}"#,
                        )
                        .await;
                    }
                });
                }
            });
        });
        let addr = address.recv().unwrap();
        format!("http://{addr}")
    }

    async fn write_json(
        socket: &mut tokio::net::TcpStream,
        status: u16,
        body: &str,
    ) -> std::io::Result<()> {
        let response = format!(
            "HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await
    }

    async fn write_stream(socket: &mut tokio::net::TcpStream) {
        let header = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\nx-conversation-id: {CHAT}\r\nx-user-message-id: {USER_MESSAGE}\r\nx-message-id: {MESSAGE}\r\nconnection: close\r\n\r\n"
        );
        if socket.write_all(header.as_bytes()).await.is_err() {
            return;
        }
        let _ = write_chunk(
            socket,
            "data: {\"choices\":[{\"delta\":{\"content\":\"hel\"}}]}\n\n",
        )
        .await;
        tokio::time::sleep(Duration::from_millis(40)).await;
        let _ = write_chunk(
            socket,
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
        )
        .await;
        tokio::time::sleep(Duration::from_millis(40)).await;
        let _ = write_chunk(socket, "data: [DONE]\n\n").await;
        let _ = socket.write_all(b"0\r\n\r\n").await;
    }

    async fn write_chunk(socket: &mut tokio::net::TcpStream, data: &str) -> std::io::Result<()> {
        socket
            .write_all(format!("{:x}\r\n", data.len()).as_bytes())
            .await?;
        socket.write_all(data.as_bytes()).await?;
        socket.write_all(b"\r\n").await?;
        socket.flush().await
    }
}
