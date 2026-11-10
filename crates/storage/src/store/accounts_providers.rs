use super::*;
impl Store {
    pub async fn register_user(
        &self,
        id: Uuid,
        username: &str,
        password_hash: &str,
    ) -> Result<(), StorageError> {
        let tx = self.db.begin().await?;
        let _locked = query::one(
            &tx,
            "SELECT id FROM users WHERE id = $1 FOR UPDATE",
            vec![Value::from(LEGACY_USER_ID)],
        )
        .await?;
        if let Err(err) = query::exec(
            &tx,
            "INSERT INTO users (id, username, password_hash) VALUES ($1, $2, $3)",
            vec![
                Value::from(id),
                Value::from(username),
                Value::from(password_hash),
            ],
        )
        .await
        {
            return Err(classify(err));
        }
        let real_users = Self::count_on(
            &tx,
            "SELECT COUNT(*) FROM users WHERE id <> $1",
            vec![Value::from(LEGACY_USER_ID)],
        )
        .await?;
        if real_users == 1 {
            let users = vec![Value::from(id), Value::from(LEGACY_USER_ID)];
            query::exec(
                &tx,
                "INSERT INTO providers (
                    user_id, id, protocol, display_name, url, token, default_model, is_default, enabled
                 )
                 SELECT $1, id, protocol, display_name, url, token, default_model, is_default, enabled
                 FROM providers WHERE user_id = $2",
                users.clone(),
            )
            .await?;
            query::exec(
                &tx,
                "UPDATE models SET user_id = $1 WHERE user_id = $2",
                users.clone(),
            )
            .await?;
            query::exec(
                &tx,
                "UPDATE conversations SET user_id = $1 WHERE user_id = $2",
                users.clone(),
            )
            .await?;
            query::exec(
                &tx,
                "UPDATE request_log SET user_id = $1 WHERE user_id = $2",
                users.clone(),
            )
            .await?;
            query::exec(
                &tx,
                "UPDATE response_states SET user_id = $1 WHERE user_id = $2",
                users.clone(),
            )
            .await?;
            query::exec(
                &tx,
                "DELETE FROM providers WHERE user_id = $1",
                vec![Value::from(LEGACY_USER_ID)],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn user_by_name(&self, username: &str) -> Result<Option<UserAccount>, StorageError> {
        let row = query::one(
            &self.db,
            "SELECT id, username, password_hash FROM users WHERE username = $1",
            vec![Value::from(username)],
        )
        .await?;
        row.as_ref().map(user_from).transpose()
    }

    pub async fn system_prompt_layers(
        &self,
        user_id: Uuid,
    ) -> Result<SystemPromptLayers, StorageError> {
        let rows = query::all(
            &self.db,
            "SELECT user_id, scope, content FROM system_prompts
             WHERE (user_id = $1 AND scope IN ('assigned', 'self'))
                OR (user_id = $2 AND scope = 'global')",
            vec![Value::from(user_id), Value::from(LEGACY_USER_ID)],
        )
        .await?;
        let mut layers = SystemPromptLayers::default();
        for row in rows {
            let scope: String = row.try_get("", "scope")?;
            let content: String = row.try_get("", "content")?;
            match scope.as_str() {
                "global" => layers.global = Some(content),
                "assigned" => layers.assigned = Some(content),
                "self" => layers.own = Some(content),
                _ => {}
            }
        }
        Ok(layers)
    }

    pub async fn set_own_system_prompt(
        &self,
        user_id: Uuid,
        content: Option<&str>,
    ) -> Result<(), StorageError> {
        let content = content.map(str::trim).filter(|value| !value.is_empty());
        match content {
            Some(content) => {
                query::exec(
                    &self.db,
                    "INSERT INTO system_prompts (user_id, scope, content)
                     VALUES ($1, 'self', $2)
                     ON CONFLICT (user_id, scope) DO UPDATE SET
                        content = EXCLUDED.content, updated_at = now()",
                    vec![Value::from(user_id), Value::from(content)],
                )
                .await?;
            }
            None => {
                query::exec(
                    &self.db,
                    "DELETE FROM system_prompts WHERE user_id = $1 AND scope = 'self'",
                    vec![Value::from(user_id)],
                )
                .await?;
            }
        }
        Ok(())
    }

    pub async fn put_token(
        &self,
        token_hash: &str,
        user_id: Uuid,
        kind: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), StorageError> {
        query::exec(
            &self.db,
            "INSERT INTO oauth_tokens (token_hash, user_id, kind, expires_at)
             VALUES ($1, $2, $3, $4::timestamptz)
             ON CONFLICT (token_hash) DO UPDATE SET
                user_id = EXCLUDED.user_id,
                kind = EXCLUDED.kind,
                expires_at = EXCLUDED.expires_at",
            vec![
                Value::from(token_hash),
                Value::from(user_id),
                Value::from(kind),
                Value::from(expires_at.to_rfc3339()),
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn token_user(
        &self,
        token_hash: &str,
        kind: &str,
    ) -> Result<Option<Uuid>, StorageError> {
        let row = query::one(
            &self.db,
            "SELECT user_id FROM oauth_tokens
             WHERE token_hash = $1 AND kind = $2 AND expires_at > now()",
            vec![Value::from(token_hash), Value::from(kind)],
        )
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(row.try_get("", "user_id")?))
    }

    pub async fn delete_token(&self, token_hash: &str) -> Result<(), StorageError> {
        query::exec(
            &self.db,
            "DELETE FROM oauth_tokens WHERE token_hash = $1",
            vec![Value::from(token_hash)],
        )
        .await?;
        Ok(())
    }

    pub async fn list_providers(&self, user_id: Uuid) -> Result<Vec<ProviderRow>, StorageError> {
        let rows = query::all(
            &self.db,
            "SELECT user_id, id, protocol, display_name, url, token, default_model, is_default, enabled
             FROM providers
             WHERE user_id = $1
             ORDER BY id",
            vec![Value::from(user_id)],
        )
        .await?;
        rows.iter().map(provider_from).collect()
    }

    pub async fn get_provider(
        &self,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<Option<ProviderRow>, StorageError> {
        let row = query::one(
            &self.db,
            "SELECT user_id, id, protocol, display_name, url, token, default_model, is_default, enabled
             FROM providers WHERE user_id = $1 AND id = $2",
            vec![Value::from(user_id), Value::from(id)],
        )
        .await?;
        row.as_ref().map(provider_from).transpose()
    }

    pub async fn upsert_provider(&self, row: &ProviderRow) -> Result<(), StorageError> {
        query::exec(
            &self.db,
            "INSERT INTO providers (
                id, protocol, display_name, url, token, default_model, is_default, enabled, user_id
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (user_id, id) DO UPDATE SET
                protocol = EXCLUDED.protocol,
                display_name = EXCLUDED.display_name,
                url = EXCLUDED.url,
                token = EXCLUDED.token,
                default_model = EXCLUDED.default_model,
                enabled = EXCLUDED.enabled",
            provider_values(row),
        )
        .await?;
        Ok(())
    }
}
