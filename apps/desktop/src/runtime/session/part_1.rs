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


}
