fn active_profile_id_optional(conn: &Connection) -> Result<Option<String>, StoreError> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = 'active_profile_id'",
        [],
        |row| row.get(0),
    )
    .optional()
    .map_err(StoreError::from)
}

fn set_active_profile(conn: &Connection, id: Uuid) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES ('active_profile_id', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [id.to_string()],
    )?;
    Ok(())
}

fn profile_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ServerProfile> {
    Ok(ServerProfile {
        id: parse_uuid(&row.get::<_, String>(0)?),
        name: row.get(1)?,
        url: row.get(2)?,
    })
}

fn parse_uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).unwrap_or_else(|_| Uuid::nil())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_draft_and_cache_roundtrip() {
        let db = LocalDb::open_memory().unwrap();
        assert_eq!(db.server_url().unwrap(), DEFAULT_SERVER_URL);
        db.set_server_url("http://127.0.0.1:9").unwrap();
        assert_eq!(db.server_url().unwrap(), "http://127.0.0.1:9");

        let id = Uuid::parse_str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").unwrap();
        assert!(db.draft(&draft_key(Some(id))).unwrap().is_empty());
        db.set_draft(&draft_key(Some(id)), "unsent").unwrap();
        assert_eq!(db.draft(&draft_key(Some(id))).unwrap(), "unsent");
        db.clear_draft(&draft_key(Some(id))).unwrap();
        assert!(db.draft(&draft_key(Some(id))).unwrap().is_empty());

        db.replace_cache(&[ConversationItem {
            id,
            title: "hello".into(),
            model: "qwen".into(),
            updated_at: "2026-11-08T00:00:00Z".into(),
        }])
        .unwrap();
        let cached = db.cached_conversations().unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].title, "hello");
        db.replace_cache(&[]).unwrap();
        assert!(db.cached_conversations().unwrap().is_empty());

        assert!(db.session().unwrap().is_none());
        db.set_session(&SessionCreds {
            username: "ada".into(),
            access_token: "sekret-token".into(),
            refresh_token: "sekret-refresh".into(),
            access_expires_at: 10,
        })
        .unwrap();
        let saved = db.session().unwrap().unwrap();
        assert_eq!(saved.username, "ada");
        assert_eq!(saved.access_token, "sekret-token");
        assert!(!format!("{saved:?}").contains("sekret"));
        db.clear_session().unwrap();
        assert!(db.session().unwrap().is_none());
    }

    #[test]
    fn profiles_isolate_their_url_session_draft_and_cache() {
        let db = LocalDb::open_memory().unwrap();
        let first = db.active_profile().unwrap();
        db.set_server_url("http://127.0.0.1:8081").unwrap();
        db.set_draft(NEW_DRAFT, "first draft").unwrap();
        db.set_session(&SessionCreds {
            username: "first".into(),
            access_token: "first-token".into(),
            refresh_token: "first-refresh".into(),
            access_expires_at: 10,
        })
        .unwrap();

        let second = db.create_profile("Remote", "https://example.test").unwrap();
        db.select_profile(second.id).unwrap();
        assert_eq!(db.server_url().unwrap(), "https://example.test");
        assert!(db.session().unwrap().is_none());
        assert!(db.draft(NEW_DRAFT).unwrap().is_empty());
        assert!(db.cached_conversations().unwrap().is_empty());

        db.set_draft(NEW_DRAFT, "second draft").unwrap();
        db.select_profile(first.id).unwrap();
        assert_eq!(db.server_url().unwrap(), "http://127.0.0.1:8081");
        assert_eq!(db.draft(NEW_DRAFT).unwrap(), "first draft");
        assert_eq!(db.session().unwrap().unwrap().username, "first");

        db.select_profile(second.id).unwrap();
        assert_eq!(db.draft(NEW_DRAFT).unwrap(), "second draft");
        assert_eq!(db.profiles().unwrap().len(), 2);
    }
}
