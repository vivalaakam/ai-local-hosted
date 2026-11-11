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
    SaveTool(ToolDraft),
    DeleteTool(Uuid),
    SaveSystemPrompt(Option<String>),
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
    Tools(Vec<ToolEndpoint>),
    ToolSaved,
    SystemPrompt(Option<String>),
    SystemPromptSaved,
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

