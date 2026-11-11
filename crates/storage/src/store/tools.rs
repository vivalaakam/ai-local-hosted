use super::*;

impl Store {
    pub async fn list_tools_for_user(&self, user_id: Uuid) -> Result<Vec<ToolRow>, StorageError> {
        let rows = query::all(
            &self.db,
            "SELECT id, owner_user_id, slug, name, description, parameters, kind, source,
                    enabled, created_at, updated_at
             FROM tools
             WHERE owner_user_id IS NULL OR owner_user_id = $1
             ORDER BY owner_user_id NULLS FIRST, slug",
            vec![Value::from(user_id)],
        )
        .await?;
        rows.iter().map(tool_from).collect()
    }

    pub async fn list_enabled_tools_for_user(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<ToolRow>, StorageError> {
        let rows = query::all(
            &self.db,
            "SELECT id, owner_user_id, slug, name, description, parameters, kind, source,
                    enabled, created_at, updated_at
             FROM tools
             WHERE enabled = TRUE
               AND (owner_user_id IS NULL OR owner_user_id = $1)
             ORDER BY owner_user_id NULLS FIRST, slug",
            vec![Value::from(user_id)],
        )
        .await?;
        rows.iter().map(tool_from).collect()
    }

    pub async fn get_tool(&self, user_id: Uuid, id: Uuid) -> Result<Option<ToolRow>, StorageError> {
        let row = query::one(
            &self.db,
            "SELECT id, owner_user_id, slug, name, description, parameters, kind, source,
                    enabled, created_at, updated_at
             FROM tools
             WHERE id = $1
               AND (owner_user_id IS NULL OR owner_user_id = $2)",
            vec![Value::from(id), Value::from(user_id)],
        )
        .await?;
        row.as_ref().map(tool_from).transpose()
    }

    pub async fn insert_user_tool(&self, tool: &NewTool) -> Result<(), StorageError> {
        query::exec(
            &self.db,
            "INSERT INTO tools (
                id, owner_user_id, slug, name, description, parameters, kind, source, enabled
             ) VALUES ($1, $2, $3, $4, $5, $6::jsonb, 'rhai', $7, $8)",
            vec![
                Value::from(tool.id),
                Value::from(tool.owner_user_id),
                Value::from(tool.slug.as_str()),
                Value::from(tool.name.as_str()),
                Value::from(tool.description.as_str()),
                Value::from(serde_json::to_string(&tool.parameters).map_err(json_err)?),
                Value::from(tool.source.as_str()),
                Value::from(tool.enabled),
            ],
        )
        .await
        .map_err(classify)?;
        Ok(())
    }

    pub async fn update_user_tool(
        &self,
        user_id: Uuid,
        id: Uuid,
        patch: ToolPatch,
    ) -> Result<Option<ToolRow>, StorageError> {
        let Some(existing) = self.get_tool(user_id, id).await? else {
            return Ok(None);
        };
        if existing.owner_user_id != Some(user_id) {
            return Err(StorageError::NotFound);
        }
        let name = patch.name.unwrap_or(existing.name);
        let description = patch.description.unwrap_or(existing.description);
        let parameters = patch.parameters.unwrap_or(existing.parameters);
        let source = patch
            .source
            .unwrap_or_else(|| existing.source.clone().unwrap_or_default());
        let enabled = patch.enabled.unwrap_or(existing.enabled);
        if source.trim().is_empty() {
            return Err(StorageError::Db(DbErr::Custom(
                "rhai source is required".into(),
            )));
        }
        query::exec(
            &self.db,
            "UPDATE tools
             SET name = $3,
                 description = $4,
                 parameters = $5::jsonb,
                 source = $6,
                 enabled = $7,
                 updated_at = now()
             WHERE id = $1 AND owner_user_id = $2",
            vec![
                Value::from(id),
                Value::from(user_id),
                Value::from(name.as_str()),
                Value::from(description.as_str()),
                Value::from(serde_json::to_string(&parameters).map_err(json_err)?),
                Value::from(source.as_str()),
                Value::from(enabled),
            ],
        )
        .await?;
        self.get_tool(user_id, id).await
    }

    pub async fn delete_user_tool(&self, user_id: Uuid, id: Uuid) -> Result<bool, StorageError> {
        let result = query::exec(
            &self.db,
            "DELETE FROM tools WHERE id = $1 AND owner_user_id = $2",
            vec![Value::from(id), Value::from(user_id)],
        )
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Finalize the current streaming assistant with tool_calls, append tool results,
    /// and insert a new streaming assistant for the next model round.
    pub async fn append_tool_round(
        &self,
        conversation_id: Uuid,
        assistant_id: Uuid,
        assistant: &MessageFinal,
        tool_results: &[ToolResultMessage],
    ) -> Result<Uuid, StorageError> {
        let tx = self.db.begin().await?;
        let tool_calls = if assistant.tool_calls.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&assistant.tool_calls).map_err(json_err)?)
        };
        let usage = match &assistant.usage {
            Some(usage) => Some(serde_json::to_string(usage).map_err(json_err)?),
            None => None,
        };
        let result = query::exec(
            &tx,
            "UPDATE messages
             SET content = $2,
                 thinking = $3,
                 tool_calls = $4::jsonb,
                 status = 'done',
                 error_code = NULL,
                 usage = $5::jsonb,
                 finished_at = now()
             WHERE id = $1 AND conversation_id = $6 AND status = 'streaming'",
            vec![
                Value::from(assistant_id),
                Value::from(assistant.content.as_str()),
                Value::from(assistant.thinking.clone()),
                Value::from(tool_calls),
                Value::from(usage),
                Value::from(conversation_id),
            ],
        )
        .await?;
        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }
        let Some(next_row) = query::one(
            &tx,
            "SELECT COALESCE(MAX(position), -1) + 1 AS next
             FROM messages WHERE conversation_id = $1",
            vec![Value::from(conversation_id)],
        )
        .await?
        else {
            return Err(StorageError::NotFound);
        };
        let mut next: i32 = next_row.try_get("", "next")?;
        for result in tool_results {
            query::exec(
                &tx,
                "INSERT INTO messages (
                    id, conversation_id, position, role, content, tool_call_id, status, finished_at
                 ) VALUES ($1, $2, $3, 'tool', $4, $5, 'done', now())",
                vec![
                    Value::from(Uuid::new_v4()),
                    Value::from(conversation_id),
                    Value::from(next),
                    Value::from(result.content.as_str()),
                    Value::from(result.tool_call_id.as_str()),
                ],
            )
            .await?;
            next += 1;
        }
        let new_assistant_id = Uuid::new_v4();
        query::exec(
            &tx,
            "INSERT INTO messages (id, conversation_id, position, role, content, status)
             VALUES ($1, $2, $3, 'assistant', '', 'streaming')",
            vec![
                Value::from(new_assistant_id),
                Value::from(conversation_id),
                Value::from(next),
            ],
        )
        .await?;
        query::exec(
            &tx,
            "UPDATE conversations SET updated_at = now() WHERE id = $1",
            vec![Value::from(conversation_id)],
        )
        .await?;
        tx.commit().await?;
        Ok(new_assistant_id)
    }
}

fn tool_from(row: &QueryResult) -> Result<ToolRow, StorageError> {
    Ok(ToolRow {
        id: row.try_get("", "id")?,
        owner_user_id: row.try_get("", "owner_user_id")?,
        slug: row.try_get("", "slug")?,
        name: row.try_get("", "name")?,
        description: row.try_get("", "description")?,
        parameters: json_get(row, "parameters")?,
        kind: row.try_get("", "kind")?,
        source: row.try_get("", "source")?,
        enabled: row.try_get("", "enabled")?,
        created_at: row.try_get("", "created_at")?,
        updated_at: row.try_get("", "updated_at")?,
    })
}
