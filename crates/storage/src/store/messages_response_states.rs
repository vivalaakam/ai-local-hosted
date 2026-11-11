use super::*;
impl Store {
    pub async fn begin_turn(
        &self,
        user_id: Uuid,
        conversation_id: Uuid,
        content: &str,
        model: Option<&str>,
        tools_prompt: Option<&str>,
    ) -> Result<BegunTurn, StorageError> {
        let tx = self.db.begin().await?;
        let row = query::one(
            &tx,
            "SELECT title, provider_id, model, system_prompt
             FROM conversations
             WHERE id = $1 AND user_id = $2
             FOR UPDATE",
            vec![Value::from(conversation_id), Value::from(user_id)],
        )
        .await?;
        let Some(row) = row else {
            return Err(StorageError::NotFound);
        };
        let title: String = row.try_get("", "title")?;
        let provider_id: Uuid = row.try_get("", "provider_id")?;
        let mut model_name: String = row.try_get("", "model")?;
        let system_prompt: Option<String> = row.try_get("", "system_prompt")?;
        let Some(streaming_row) = query::one(
            &tx,
            "SELECT EXISTS(
                SELECT 1 FROM messages WHERE conversation_id = $1 AND status = 'streaming'
             ) AS streaming",
            vec![Value::from(conversation_id)],
        )
        .await?
        else {
            return Err(StorageError::NotFound);
        };
        let streaming: bool = streaming_row.try_get("", "streaming")?;
        if streaming {
            return Err(StorageError::Conflict);
        }
        if let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) {
            model_name = model.to_string();
        }
        let Some(next_row) = query::one(
            &tx,
            "SELECT COALESCE(MAX(position), -1) + 1 AS next FROM messages WHERE conversation_id = $1",
            vec![Value::from(conversation_id)],
        )
        .await?
        else {
            return Err(StorageError::NotFound);
        };
        let mut next: i32 = next_row.try_get("", "next")?;
        let mut system_message_id = None;
        if next == 0 {
            let layers = layers_on(&tx, user_id).await?;
            if let Some(prompt) =
                display_system_prompt(&layers, system_prompt.as_deref(), tools_prompt)
            {
                let id = Uuid::new_v4();
                query::exec(
                    &tx,
                    "INSERT INTO messages (id, conversation_id, position, role, content, status, finished_at)
                     VALUES ($1, $2, $3, 'system', $4, 'done', now())",
                    vec![
                        Value::from(id),
                        Value::from(conversation_id),
                        Value::from(next),
                        Value::from(prompt),
                    ],
                )
                .await?;
                system_message_id = Some(id);
                next += 1;
            }
        }
        let user_message_id = Uuid::new_v4();
        let assistant_id = Uuid::new_v4();
        query::exec(
            &tx,
            "INSERT INTO messages (id, conversation_id, position, role, content, status)
             VALUES ($1, $2, $3, 'user', $4, 'done')",
            vec![
                Value::from(user_message_id),
                Value::from(conversation_id),
                Value::from(next),
                Value::from(content),
            ],
        )
        .await?;
        query::exec(
            &tx,
            "INSERT INTO messages (id, conversation_id, position, role, content, status)
             VALUES ($1, $2, $3, 'assistant', '', 'streaming')",
            vec![
                Value::from(assistant_id),
                Value::from(conversation_id),
                Value::from(next + 1),
            ],
        )
        .await?;
        let new_title = if title.trim().is_empty() {
            let drafted = title_from(content);
            if drafted.is_empty() {
                None
            } else {
                Some(drafted)
            }
        } else {
            None
        };
        query::exec(
            &tx,
            "UPDATE conversations
             SET model = $2, title = COALESCE($3, title), updated_at = now()
             WHERE id = $1",
            vec![
                Value::from(conversation_id),
                Value::from(&model_name),
                Value::from(new_title),
            ],
        )
        .await?;
        let history = load_done(&tx, conversation_id).await?;
        tx.commit().await?;
        Ok(BegunTurn {
            conversation_id,
            user_message_id,
            assistant_message_id: assistant_id,
            system_message_id,
            provider_id,
            model: model_name,
            system_prompt,
            history,
        })
    }

    pub async fn finalize_message(&self, update: MessageFinal) -> Result<(), StorageError> {
        let tool_calls = if update.tool_calls.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&update.tool_calls).map_err(json_err)?)
        };
        let usage = match &update.usage {
            Some(usage) => Some(serde_json::to_string(usage).map_err(json_err)?),
            None => None,
        };
        query::exec(
            &self.db,
            "UPDATE messages
             SET content = $2, thinking = $3, tool_calls = $4::jsonb, status = $5,
                 error_code = $6, usage = $7::jsonb, finished_at = now()
             WHERE id = $1",
            vec![
                Value::from(update.id),
                Value::from(update.content),
                Value::from(update.thinking),
                Value::from(tool_calls),
                Value::from(update.status),
                Value::from(update.error_code),
                Value::from(usage),
            ],
        )
        .await?;
        query::exec(
            &self.db,
            "UPDATE conversations
             SET updated_at = now()
             WHERE id = (SELECT conversation_id FROM messages WHERE id = $1)",
            vec![Value::from(update.id)],
        )
        .await?;
        Ok(())
    }

    pub async fn insert_request_log(&self, entry: &RequestLogEntry) -> Result<(), StorageError> {
        query::exec(
            &self.db,
            "INSERT INTO request_log (
                id, source, protocol, conversation_id, message_id, provider_id,
                client_model, model, status, error_code, latency_ms, prompt_tokens,
                completion_tokens, user_id
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
            vec![
                Value::from(Uuid::new_v4()),
                Value::from(entry.source.as_str()),
                Value::from(entry.protocol.as_str()),
                Value::from(entry.conversation_id),
                Value::from(entry.message_id),
                Value::from(entry.provider_id),
                Value::from(entry.client_model.as_str()),
                Value::from(entry.model.as_str()),
                Value::from(entry.status.as_str()),
                Value::from(entry.error_code.clone()),
                Value::from(entry.latency_ms),
                Value::from(entry.prompt_tokens),
                Value::from(entry.completion_tokens),
                Value::from(entry.user_id),
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn save_response_state(&self, state: &NewResponseState) -> Result<(), StorageError> {
        query::exec(
            &self.db,
            "INSERT INTO response_states (
                id, previous_id, client_model, provider_id, upstream_model, messages, output, user_id
             ) VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7::jsonb, $8)
             ON CONFLICT (id) DO UPDATE SET
                messages = EXCLUDED.messages,
                output = EXCLUDED.output
             WHERE response_states.user_id = EXCLUDED.user_id",
            vec![
                Value::from(state.id.as_str()),
                Value::from(state.previous_id.clone()),
                Value::from(state.client_model.as_str()),
                Value::from(state.provider_id),
                Value::from(state.upstream_model.as_str()),
                Value::from(serde_json::to_string(&state.messages).map_err(json_err)?),
                Value::from(serde_json::to_string(&state.output).map_err(json_err)?),
                Value::from(state.user_id),
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn get_response_state(
        &self,
        user_id: Uuid,
        id: &str,
    ) -> Result<Option<ResponseState>, StorageError> {
        let row = query::one(
            &self.db,
            "SELECT id, previous_id, client_model, provider_id, upstream_model, messages, output, created_at
             FROM response_states WHERE user_id = $1 AND id = $2",
            vec![Value::from(user_id), Value::from(id)],
        )
        .await?;
        row.as_ref().map(response_state_from).transpose()
    }

    pub(super) async fn table_exists(&self, table: &str) -> Result<bool, StorageError> {
        let Some(row) = query::one(
            &self.db,
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.tables
                WHERE table_schema = current_schema() AND table_name = $1
             ) AS present",
            vec![Value::from(table)],
        )
        .await?
        else {
            return Ok(false);
        };
        Ok(row.try_get("", "present")?)
    }

    pub(super) async fn migration_applied(&self, version: &str) -> Result<bool, StorageError> {
        Self::migration_flag(&self.db, version).await
    }

    pub(super) async fn migration_flag(
        conn: &impl ConnectionTrait,
        version: &str,
    ) -> Result<bool, StorageError> {
        let Some(row) = query::one(
            conn,
            "SELECT EXISTS(SELECT 1 FROM alh_migrations WHERE version = $1) AS applied",
            vec![Value::from(version)],
        )
        .await?
        else {
            return Ok(false);
        };
        Ok(row.try_get("", "applied")?)
    }
}
