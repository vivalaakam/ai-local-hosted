impl Session {
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
        if let Some(tools) = self.call(false, async |api| api.list_tools().await).await {
            self.emit(UiEvent::Tools(tools));
        }
        if let Some(prompt) = self
            .call(false, async |api| api.get_system_prompt().await)
            .await
        {
            self.emit(UiEvent::SystemPrompt(prompt));
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
