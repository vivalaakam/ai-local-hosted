impl Api {
    pub fn new(base: impl Into<String>, token: impl Into<String>) -> Result<Self, ApiError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .build()
            .map_err(transport)?;
        Ok(Self {
            http,
            base: base.into().trim_end_matches('/').to_string(),
            token: token.into(),
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn set_base(&mut self, base: impl Into<String>) {
        self.base = base.into().trim_end_matches('/').to_string();
    }

    pub fn set_token(&mut self, token: impl Into<String>) {
        self.token = token.into();
    }

    pub async fn register(&self, username: &str, password: &str) -> Result<AuthTokens, ApiError> {
        let response = self
            .http
            .post(self.url("/oauth/register"))
            .json(&serde_json::json!({
                "username": username,
                "password": password,
            }))
            .send()
            .await
            .map_err(transport)?;
        auth_from(response).await
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<AuthTokens, ApiError> {
        self.token_form(&[
            ("grant_type", "password"),
            ("username", username),
            ("password", password),
        ])
        .await
    }

    pub async fn refresh(&self, refresh_token: &str) -> Result<AuthTokens, ApiError> {
        self.token_form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])
        .await
    }

    async fn token_form(&self, fields: &[(&str, &str)]) -> Result<AuthTokens, ApiError> {
        let response = self
            .http
            .post(self.url("/oauth/token"))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form_body(fields))
            .send()
            .await
            .map_err(transport)?;
        auth_from(response).await
    }

    pub async fn list_conversations(&self) -> Result<Vec<ConversationItem>, ApiError> {
        let mut items = Vec::new();
        let mut cursor = None;
        for _ in 0..500 {
            let mut path = "/api/conversations?limit=100".to_string();
            if let Some(cursor) = cursor.as_deref() {
                path.push_str("&cursor=");
                path.push_str(&query_escape(cursor));
            }
            let body: ListBody<ConversationBody> = self.get_json(&path).await?;
            let next = body.next_cursor.filter(|value| !value.is_empty());
            items.extend(body.data.into_iter().map(ConversationBody::item));
            match next {
                Some(next) if Some(next.as_str()) != cursor.as_deref() => cursor = Some(next),
                _ => break,
            }
        }
        Ok(items)
    }

    pub async fn import_claude(&self, bytes: Vec<u8>) -> Result<ImportResult, ApiError> {
        let response = self
            .http
            .post(self.url("/api/imports/claude"))
            .bearer_auth(&self.token)
            .header("content-type", "application/octet-stream")
            .body(bytes)
            .send()
            .await
            .map_err(transport)?;
        if !response.status().is_success() {
            return Err(error_from(response).await);
        }
        let body: ImportBody = response.json().await.map_err(transport)?;
        Ok(ImportResult {
            conversations: body.conversations,
            skipped: body.skipped,
            messages: body.messages,
        })
    }

    pub async fn list_models(&self) -> Result<Vec<String>, ApiError> {
        let body: ListBody<ModelBody> = self.get_json("/v1/models").await?;
        Ok(body.data.into_iter().map(|model| model.id).collect())
    }

    pub async fn list_providers(&self) -> Result<Vec<ProviderEndpoint>, ApiError> {
        let body: ListBody<ProviderBody> = self.get_json("/api/providers").await?;
        Ok(body.data.into_iter().map(ProviderBody::endpoint).collect())
    }

    pub async fn save_provider(&self, draft: &ProviderDraft) -> Result<ProviderEndpoint, ApiError> {
        let (path, method) = if draft.update {
            let Some(id) = draft.id else {
                return Err(ApiError {
                    status: 0,
                    code: "invalid_request".into(),
                    message: "missing provider id".into(),
                });
            };
            (format!("/api/providers/{id}"), reqwest::Method::PATCH)
        } else {
            ("/api/providers".to_string(), reqwest::Method::POST)
        };
        let body: ProviderBody = self.send_json(method, &path, provider_body(draft)).await?;
        Ok(body.endpoint())
    }

    pub async fn delete_provider(&self, id: Uuid) -> Result<(), ApiError> {
        let response = self
            .http
            .delete(self.url(&format!("/api/providers/{id}")))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(transport)?;
        if response.status().is_success() {
            return Ok(());
        }
        Err(error_from(response).await)
    }

    pub async fn list_tools(&self) -> Result<Vec<ToolEndpoint>, ApiError> {
        let body: ListBody<ToolBody> = self.get_json("/api/tools?include_source=1").await?;
        Ok(body.data.into_iter().map(ToolBody::endpoint).collect())
    }

    pub async fn save_tool(&self, draft: &ToolDraft) -> Result<ToolEndpoint, ApiError> {
        let (path, method) = if draft.update {
            let Some(id) = draft.id else {
                return Err(ApiError {
                    status: 0,
                    code: "invalid_request".into(),
                    message: "missing tool id".into(),
                });
            };
            (format!("/api/tools/{id}"), reqwest::Method::PATCH)
        } else {
            ("/api/tools".to_string(), reqwest::Method::POST)
        };
        let body: ToolBody = self.send_json(method, &path, tool_body(draft)).await?;
        Ok(body.endpoint())
    }

    pub async fn delete_tool(&self, id: Uuid) -> Result<(), ApiError> {
        let response = self
            .http
            .delete(self.url(&format!("/api/tools/{id}")))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(transport)?;
        if response.status().is_success() {
            return Ok(());
        }
        Err(error_from(response).await)
    }

    pub async fn get_system_prompt(&self) -> Result<Option<String>, ApiError> {
        let body: SystemPromptBody = self.get_json("/api/system-prompt").await?;
        Ok(body.content.filter(|text| !text.trim().is_empty()))
    }

    pub async fn save_system_prompt(&self, content: Option<&str>) -> Result<(), ApiError> {
        let response = self
            .http
            .put(self.url("/api/system-prompt"))
            .bearer_auth(&self.token)
            .json(&serde_json::json!({ "content": content }))
            .send()
            .await
            .map_err(transport)?;
        if response.status().is_success() {
            return Ok(());
        }
        Err(error_from(response).await)
    }

    pub async fn get_conversation(&self, id: Uuid) -> Result<ChatDetail, ApiError> {
        let body: ConversationBody = self.get_json(&format!("/api/conversations/{id}")).await?;
        Ok(body.detail())
    }

    pub async fn create_conversation(&self, model: &str) -> Result<ConversationItem, ApiError> {
        let body: ConversationBody = self
            .send_json(
                reqwest::Method::POST,
                "/api/conversations",
                serde_json::json!({ "model": model }),
            )
            .await?;
        Ok(body.item())
    }

    pub async fn set_model(&self, id: Uuid, model: &str) -> Result<ConversationItem, ApiError> {
        let body: ConversationBody = self
            .send_json(
                reqwest::Method::PATCH,
                &format!("/api/conversations/{id}"),
                serde_json::json!({ "model": model }),
            )
            .await?;
        Ok(body.item())
    }

    pub async fn delete_conversation(&self, id: Uuid) -> Result<(), ApiError> {
        let response = self
            .http
            .delete(self.url(&format!("/api/conversations/{id}")))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(transport)?;
        if response.status().is_success() {
            return Ok(());
        }
        Err(error_from(response).await)
    }

    /// Starts a chat turn. The body is SSE in the chat-completion shape.
    pub async fn send_message(
        &self,
        id: Uuid,
        content: &str,
        model: &str,
    ) -> Result<LiveTurn, ApiError> {
        let response = self
            .http
            .post(self.url(&format!("/api/conversations/{id}/messages")))
            .bearer_auth(&self.token)
            .json(&serde_json::json!({ "content": content, "model": model }))
            .send()
            .await
            .map_err(transport)?;
        if !response.status().is_success() {
            return Err(error_from(response).await);
        }
        let conversation_id = header_uuid(&response, "x-conversation-id")?;
        let user_message_id = header_uuid(&response, "x-user-message-id")?;
        let message_id = header_uuid(&response, "x-message-id")?;
        Ok(LiveTurn {
            conversation_id,
            user_message_id,
            message_id,
            response,
        })
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, ApiError> {
        self.send_json(reqwest::Method::GET, path, serde_json::Value::Null)
            .await
    }

    async fn send_json<T: for<'de> Deserialize<'de>>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: serde_json::Value,
    ) -> Result<T, ApiError> {
        let mut request = self
            .http
            .request(method, self.url(path))
            .bearer_auth(&self.token);
        if !body.is_null() {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(transport)?;
        if !response.status().is_success() {
            return Err(error_from(response).await);
        }
        response.json().await.map_err(transport)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
}

pub struct LiveTurn {
    pub conversation_id: Uuid,
    pub user_message_id: Uuid,
    pub message_id: Uuid,
    response: reqwest::Response,
}

