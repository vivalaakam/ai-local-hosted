use super::*;
impl Store {
    pub async fn insert_provider(&self, row: &ProviderRow) -> Result<(), StorageError> {
        let tx = self.db.begin().await?;
        let mut values = provider_values(row);
        values[6] = Value::from(false);
        if let Err(err) = query::exec(
            &tx,
            "INSERT INTO providers (
                id, protocol, display_name, url, token, default_model, is_default, enabled, user_id
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            values,
        )
        .await
        {
            return Err(classify(err));
        }
        if row.is_default {
            set_default_on(&tx, row.user_id, row.id).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn update_provider(
        &self,
        user_id: Uuid,
        id: Uuid,
        patch: ProviderUpdate,
    ) -> Result<Option<ProviderRow>, StorageError> {
        let Some(mut row) = self.get_provider(user_id, id).await? else {
            return Ok(None);
        };
        if let Some(protocol) = patch.protocol {
            row.protocol = protocol;
        }
        if let Some(display_name) = patch.display_name {
            row.display_name = display_name;
        }
        if let Some(url) = patch.url {
            row.url = url;
        }
        if let Some(token) = patch.token {
            row.token = blank_to_none(token);
        }
        if let Some(default_model) = patch.default_model {
            row.default_model = blank_to_none(default_model);
        }
        if let Some(enabled) = patch.enabled {
            row.enabled = enabled;
        }
        if patch.is_default == Some(false) {
            row.is_default = false;
        }
        let tx = self.db.begin().await?;
        query::exec(
            &tx,
            "UPDATE providers
             SET protocol = $2, display_name = $3, url = $4, token = $5,
                 default_model = $6, is_default = $7, enabled = $8
             WHERE id = $1 AND user_id = $9",
            provider_values(&row),
        )
        .await?;
        if patch.is_default == Some(true) {
            set_default_on(&tx, user_id, id).await?;
        }
        tx.commit().await?;
        self.get_provider(user_id, id).await
    }

    pub async fn delete_provider(&self, user_id: Uuid, id: Uuid) -> Result<bool, StorageError> {
        match query::exec(
            &self.db,
            "DELETE FROM providers WHERE user_id = $1 AND id = $2",
            vec![Value::from(user_id), Value::from(id)],
        )
        .await
        {
            Ok(result) => Ok(result.rows_affected() > 0),
            Err(err) => Err(classify(err)),
        }
    }

    pub async fn set_default_provider(&self, user_id: Uuid, id: Uuid) -> Result<(), StorageError> {
        let tx = self.db.begin().await?;
        set_default_on(&tx, user_id, id).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn models_fetched_at(
        &self,
        user_id: Uuid,
        provider_id: Uuid,
    ) -> Result<Option<DateTime<Utc>>, StorageError> {
        let Some(row) = query::one(
            &self.db,
            "SELECT MAX(fetched_at) AS fetched_at FROM models
             WHERE user_id = $1 AND provider_id = $2",
            vec![Value::from(user_id), Value::from(provider_id)],
        )
        .await?
        else {
            return Ok(None);
        };
        Ok(row.try_get("", "fetched_at")?)
    }

    pub async fn replace_models(
        &self,
        user_id: Uuid,
        provider_id: Uuid,
        models: &[ModelDescriptor],
    ) -> Result<(), StorageError> {
        let tx = self.db.begin().await?;
        query::exec(
            &tx,
            "DELETE FROM models WHERE user_id = $1 AND provider_id = $2",
            vec![Value::from(user_id), Value::from(provider_id)],
        )
        .await?;
        for model in models {
            query::exec(
                &tx,
                "INSERT INTO models (
                    user_id, provider_id, name, context_window, supports_tools, fetched_at
                 ) VALUES ($1, $2, $3, $4, $5, now())",
                vec![
                    Value::from(user_id),
                    Value::from(provider_id),
                    Value::from(model.name.as_str()),
                    Value::from(model.context_window),
                    Value::from(model.supports_tools),
                ],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn catalog(&self, user_id: Uuid) -> Result<Vec<CatalogModel>, StorageError> {
        let rows = query::all(
            &self.db,
            "SELECT m.provider_id, m.name
             FROM models m
             JOIN providers p ON p.user_id = m.user_id AND p.id = m.provider_id
             WHERE p.enabled AND m.user_id = $1
             ORDER BY m.provider_id, m.name",
            vec![Value::from(user_id)],
        )
        .await?;
        rows.iter()
            .map(|row| {
                let provider_id: Uuid = row.try_get("", "provider_id")?;
                Ok(CatalogModel {
                    provider_slug: provider_id.to_string(),
                    name: row.try_get("", "name")?,
                })
            })
            .collect()
    }

    pub async fn create_conversation(
        &self,
        input: NewConversation,
    ) -> Result<Conversation, StorageError> {
        if self
            .get_provider(input.user_id, input.provider_id)
            .await?
            .is_none()
        {
            return Err(StorageError::NotFound);
        }
        let Some(row) = query::one(
            &self.db,
            "INSERT INTO conversations (id, user_id, title, provider_id, model, system_prompt)
             VALUES ($1, $2, $3, $4, $5, $6)
             RETURNING id, title, provider_id, model, system_prompt, created_at, updated_at, archived_at",
            vec![
                Value::from(input.id),
                Value::from(input.user_id),
                Value::from(input.title),
                Value::from(input.provider_id),
                Value::from(input.model),
                Value::from(input.system_prompt),
            ],
        )
        .await?
        else {
            return Err(StorageError::NotFound);
        };
        conversation_from(&row)
    }
}
