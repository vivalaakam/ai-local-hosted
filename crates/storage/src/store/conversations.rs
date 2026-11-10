use super::*;
impl Store {
    /// Inserts exported chats for this user. A conversation id that already exists is skipped.
    pub async fn import_chats(
        &self,
        user_id: Uuid,
        provider_id: Uuid,
        chats: &[ImportedChat],
    ) -> Result<ImportCounts, StorageError> {
        if self.get_provider(user_id, provider_id).await?.is_none() {
            return Err(StorageError::NotFound);
        }
        let tx = self.db.begin().await?;
        let mut counts = ImportCounts {
            conversations: 0,
            skipped: 0,
            messages: 0,
        };
        for chat in chats {
            let inserted = query::one(
                &tx,
                "INSERT INTO conversations
                    (id, user_id, title, provider_id, model, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6::timestamptz, $7::timestamptz)
                 ON CONFLICT (id) DO NOTHING
                 RETURNING id",
                vec![
                    Value::from(chat.id),
                    Value::from(user_id),
                    Value::from(chat.title.as_str()),
                    Value::from(provider_id),
                    Value::from(chat.model.as_str()),
                    Value::from(chat.created_at.to_rfc3339()),
                    Value::from(chat.updated_at.to_rfc3339()),
                ],
            )
            .await?;
            if inserted.is_none() {
                counts.skipped += 1;
                continue;
            }
            counts.conversations += 1;
            for (position, message) in chat.messages.iter().enumerate() {
                let position = i32::try_from(position).map_err(|_| {
                    StorageError::Db(DbErr::Custom("conversation has too many messages".into()))
                })?;
                let created = message.created_at.to_rfc3339();
                let row = query::one(
                    &tx,
                    "INSERT INTO messages
                        (id, conversation_id, position, role, content, thinking, status, created_at, finished_at)
                     VALUES ($1, $2, $3, $4, $5, $6, 'done', $7::timestamptz, $7::timestamptz)
                     ON CONFLICT (id) DO NOTHING
                     RETURNING id",
                    vec![
                        Value::from(message.id),
                        Value::from(chat.id),
                        Value::from(position),
                        Value::from(message.role.as_str()),
                        Value::from(message.content.as_str()),
                        Value::from(message.thinking.clone()),
                        Value::from(created.as_str()),
                    ],
                )
                .await?;
                if row.is_some() {
                    counts.messages += 1;
                }
            }
        }
        tx.commit().await?;
        Ok(counts)
    }

    pub async fn list_conversations(
        &self,
        user_id: Uuid,
        cursor: Option<(DateTime<Utc>, Uuid)>,
        limit: i64,
    ) -> Result<ConversationPage, StorageError> {
        let (updated_at, id) = match cursor {
            Some((updated_at, id)) => (Some(updated_at), Some(id)),
            None => (None, None),
        };
        let rows = query::all(
            &self.db,
            "SELECT id, title, provider_id, model, system_prompt, created_at, updated_at, archived_at
             FROM conversations
             WHERE user_id = $1
               AND archived_at IS NULL
               AND (
                    $2::timestamptz IS NULL
                    OR updated_at < $2
                    OR (updated_at = $2 AND id < $3)
               )
             ORDER BY updated_at DESC, id DESC
             LIMIT $4",
            vec![
                Value::from(user_id),
                Value::from(updated_at),
                Value::from(id),
                Value::from(limit + 1),
            ],
        )
        .await?;
        let mut items = rows
            .iter()
            .map(conversation_from)
            .collect::<Result<Vec<_>, _>>()?;
        let next_cursor = if items.len() as i64 > limit {
            items.pop();
            items.last().map(cursor_of)
        } else {
            None
        };
        Ok(ConversationPage { items, next_cursor })
    }

    pub async fn get_conversation(
        &self,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<Option<Conversation>, StorageError> {
        let row = query::one(
            &self.db,
            "SELECT id, title, provider_id, model, system_prompt, created_at, updated_at, archived_at
             FROM conversations WHERE user_id = $1 AND id = $2",
            vec![Value::from(user_id), Value::from(id)],
        )
        .await?;
        row.as_ref().map(conversation_from).transpose()
    }

    pub async fn list_messages(
        &self,
        user_id: Uuid,
        conversation_id: Uuid,
    ) -> Result<Vec<MessageRow>, StorageError> {
        if self
            .get_conversation(user_id, conversation_id)
            .await?
            .is_none()
        {
            return Err(StorageError::NotFound);
        }
        let rows = query::all(
            &self.db,
            "SELECT id, conversation_id, position, role, content, parts, tool_calls, tool_call_id,
                    thinking, status, error_code, usage, created_at, finished_at
             FROM messages
             WHERE conversation_id = $1
             ORDER BY position",
            vec![Value::from(conversation_id)],
        )
        .await?;
        rows.iter().map(message_from).collect()
    }

    pub async fn patch_conversation(
        &self,
        user_id: Uuid,
        id: Uuid,
        patch: ConversationPatch,
    ) -> Result<Option<Conversation>, StorageError> {
        if self.get_conversation(user_id, id).await?.is_none() {
            return Ok(None);
        }
        if let Some(title) = patch.title {
            query::exec(
                &self.db,
                "UPDATE conversations SET title = $2, updated_at = now() WHERE id = $1 AND user_id = $3",
                vec![Value::from(id), Value::from(title), Value::from(user_id)],
            )
            .await?;
        }
        if let Some(system_prompt) = patch.system_prompt {
            query::exec(
                &self.db,
                "UPDATE conversations SET system_prompt = $2, updated_at = now()
                 WHERE id = $1 AND user_id = $3",
                vec![
                    Value::from(id),
                    Value::from(system_prompt),
                    Value::from(user_id),
                ],
            )
            .await?;
        }
        if let Some(model) = patch.model {
            query::exec(
                &self.db,
                "UPDATE conversations SET model = $2, updated_at = now() WHERE id = $1 AND user_id = $3",
                vec![Value::from(id), Value::from(model), Value::from(user_id)],
            )
            .await?;
        }
        if let Some(archived) = patch.archived {
            if archived {
                query::exec(
                    &self.db,
                    "UPDATE conversations SET archived_at = now(), updated_at = now()
                     WHERE id = $1 AND user_id = $2",
                    vec![Value::from(id), Value::from(user_id)],
                )
                .await?;
            } else {
                query::exec(
                    &self.db,
                    "UPDATE conversations SET archived_at = NULL, updated_at = now()
                     WHERE id = $1 AND user_id = $2",
                    vec![Value::from(id), Value::from(user_id)],
                )
                .await?;
            }
        }
        self.get_conversation(user_id, id).await
    }

    pub async fn delete_conversation(&self, user_id: Uuid, id: Uuid) -> Result<bool, StorageError> {
        let result = query::exec(
            &self.db,
            "DELETE FROM conversations WHERE id = $1 AND user_id = $2",
            vec![Value::from(id), Value::from(user_id)],
        )
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn set_conversation_route(
        &self,
        user_id: Uuid,
        id: Uuid,
        provider_id: Uuid,
        model: &str,
    ) -> Result<(), StorageError> {
        if self.get_provider(user_id, provider_id).await?.is_none() {
            return Err(StorageError::NotFound);
        }
        query::exec(
            &self.db,
            "UPDATE conversations SET provider_id = $2, model = $3, updated_at = now()
             WHERE id = $1 AND user_id = $4",
            vec![
                Value::from(id),
                Value::from(provider_id),
                Value::from(model),
                Value::from(user_id),
            ],
        )
        .await?;
        Ok(())
    }
}
