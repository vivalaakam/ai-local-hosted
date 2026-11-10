impl LocalDb {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::from_conn(Connection::open(path)?)
    }

    pub fn open_memory() -> Result<Self, StoreError> {
        Self::from_conn(Connection::open_in_memory()?)
    }

    fn from_conn(conn: Connection) -> Result<Self, StoreError> {
        conn.execute_batch(SCHEMA)?;
        migrate_profiles(&conn)?;
        Ok(Self { conn })
    }

    pub fn active_profile(&self) -> Result<ServerProfile, StoreError> {
        let id = active_profile_id(&self.conn)?;
        self.conn
            .query_row(
                "SELECT id, name, url FROM server_profiles WHERE id = ?1",
                [id],
                profile_from_row,
            )
            .map_err(StoreError::from)
    }

    pub fn profiles(&self) -> Result<Vec<ServerProfile>, StoreError> {
        let active = active_profile_id(&self.conn)?;
        let mut statement = self.conn.prepare(
            "SELECT id, name, url FROM server_profiles
             ORDER BY CASE id WHEN ?1 THEN 0 ELSE 1 END, name COLLATE NOCASE",
        )?;
        statement
            .query_map([active], profile_from_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn create_profile(&self, name: &str, url: &str) -> Result<ServerProfile, StoreError> {
        let profile = ServerProfile {
            id: Uuid::new_v4(),
            name: name.trim().to_string(),
            url: url.trim().to_string(),
        };
        self.conn.execute(
            "INSERT INTO server_profiles (id, name, url) VALUES (?1, ?2, ?3)",
            rusqlite::params![profile.id.to_string(), profile.name, profile.url],
        )?;
        Ok(profile)
    }

    pub fn select_profile(&self, id: Uuid) -> Result<(), StoreError> {
        let exists: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM server_profiles WHERE id = ?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(StoreError {
                message: "server profile not found".into(),
            });
        }
        set_active_profile(&self.conn, id)
    }

    pub fn server_url(&self) -> Result<String, StoreError> {
        Ok(self.active_profile()?.url)
    }

    pub fn set_server_url(&self, url: &str) -> Result<(), StoreError> {
        let profile = active_profile_id(&self.conn)?;
        self.conn.execute(
            "UPDATE server_profiles SET url = ?1 WHERE id = ?2",
            rusqlite::params![url.trim(), profile],
        )?;
        Ok(())
    }

    pub fn session(&self) -> Result<Option<SessionCreds>, StoreError> {
        self.conn
            .query_row(
                "SELECT username, access_token, refresh_token, access_expires_at
                 FROM profile_sessions WHERE profile_id = ?1",
                [active_profile_id(&self.conn)?],
                |row| {
                    Ok(SessionCreds {
                        username: row.get(0)?,
                        access_token: row.get(1)?,
                        refresh_token: row.get(2)?,
                        access_expires_at: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn set_session(&self, session: &SessionCreds) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO profile_sessions (profile_id, username, access_token, refresh_token, access_expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(profile_id) DO UPDATE SET
                username = excluded.username,
                access_token = excluded.access_token,
                refresh_token = excluded.refresh_token,
                access_expires_at = excluded.access_expires_at",
            rusqlite::params![
                active_profile_id(&self.conn)?, session.username, session.access_token,
                session.refresh_token, session.access_expires_at
            ],
        )?;
        Ok(())
    }

    pub fn clear_session(&self) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM profile_sessions WHERE profile_id = ?1",
            [active_profile_id(&self.conn)?],
        )?;
        Ok(())
    }

    pub fn draft(&self, conversation_id: &str) -> Result<String, StoreError> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT body FROM profile_drafts WHERE profile_id = ?1 AND conversation_id = ?2",
                rusqlite::params![active_profile_id(&self.conn)?, conversation_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.unwrap_or_default())
    }

    pub fn set_draft(&self, conversation_id: &str, body: &str) -> Result<(), StoreError> {
        if body.is_empty() {
            return self.clear_draft(conversation_id);
        }
        self.conn.execute(
            "INSERT INTO profile_drafts (profile_id, conversation_id, body) VALUES (?1, ?2, ?3)
             ON CONFLICT(profile_id, conversation_id) DO UPDATE SET body = excluded.body",
            rusqlite::params![active_profile_id(&self.conn)?, conversation_id, body],
        )?;
        Ok(())
    }

    pub fn clear_draft(&self, conversation_id: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM profile_drafts WHERE profile_id = ?1 AND conversation_id = ?2",
            rusqlite::params![active_profile_id(&self.conn)?, conversation_id],
        )?;
        Ok(())
    }

    pub fn cached_conversations(&self) -> Result<Vec<ConversationItem>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT id, title, model, updated_at FROM profile_conversation_cache
             WHERE profile_id = ?1 ORDER BY position",
        )?;
        let rows = statement.query_map([active_profile_id(&self.conn)?], |row| {
            Ok(ConversationItem {
                id: parse_uuid(&row.get::<_, String>(0)?),
                title: row.get(1)?,
                model: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn replace_cache(&self, items: &[ConversationItem]) -> Result<(), StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        let profile = active_profile_id(&self.conn)?;
        tx.execute(
            "DELETE FROM profile_conversation_cache WHERE profile_id = ?1",
            [&profile],
        )?;
        for (position, item) in items.iter().enumerate() {
            tx.execute(
                "INSERT INTO profile_conversation_cache
                 (profile_id, id, title, model, updated_at, position)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    profile,
                    item.id.to_string(),
                    item.title,
                    item.model,
                    item.updated_at,
                    position as i64
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

fn migrate_profiles(conn: &Connection) -> Result<(), StoreError> {
    let count: i64 =
        conn.query_row("SELECT COUNT(*) FROM server_profiles", [], |row| row.get(0))?;
    if count == 0 {
        let url: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'server_url'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let profile = ServerProfile {
            id: Uuid::new_v4(),
            name: "Local server".into(),
            url: url.unwrap_or_else(|| DEFAULT_SERVER_URL.into()),
        };
        conn.execute(
            "INSERT INTO server_profiles (id, name, url) VALUES (?1, ?2, ?3)",
            rusqlite::params![profile.id.to_string(), profile.name, profile.url],
        )?;
        conn.execute(
            "INSERT INTO profile_sessions (profile_id, username, access_token, refresh_token, access_expires_at)
             SELECT ?1, username, access_token, refresh_token, access_expires_at FROM session WHERE id = 1",
            [profile.id.to_string()],
        )?;
        conn.execute(
            "INSERT INTO profile_drafts (profile_id, conversation_id, body)
             SELECT ?1, conversation_id, body FROM drafts",
            [profile.id.to_string()],
        )?;
        conn.execute(
            "INSERT INTO profile_conversation_cache (profile_id, id, title, model, updated_at, position)
             SELECT ?1, id, title, model, updated_at, position FROM conversation_cache",
            [profile.id.to_string()],
        )?;
        set_active_profile(conn, profile.id)?;
    } else if active_profile_id_optional(conn)?.is_none() {
        let id: String = conn.query_row(
            "SELECT id FROM server_profiles ORDER BY name COLLATE NOCASE LIMIT 1",
            [],
            |row| row.get(0),
        )?;
        set_active_profile(conn, parse_uuid(&id))?;
    }
    Ok(())
}

fn active_profile_id(conn: &Connection) -> Result<String, StoreError> {
    active_profile_id_optional(conn)?.ok_or_else(|| StoreError {
        message: "no active server profile".into(),
    })
}

