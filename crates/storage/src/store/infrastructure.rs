use super::*;
impl Store {
    pub async fn connect(database_url: &str, schema: Option<&str>) -> Result<Self, StorageError> {
        let mut options = ConnectOptions::new(database_url);
        options
            .max_connections(8)
            .sqlx_logging(false)
            .record_stmt_in_spans(false);
        if let Some(schema) = schema {
            if !safe_ident(schema) {
                return Err(StorageError::Db(DbErr::Custom(format!(
                    "unsafe schema name: {schema}"
                ))));
            }
            let schema = schema.to_string();
            options.map_sqlx_postgres_pool_opts(move |pool| {
                let schema = schema.clone();
                pool.after_connect(move |conn, _meta| {
                    let schema = schema.clone();
                    Box::pin(async move {
                        let sql = format!(
                            "CREATE SCHEMA IF NOT EXISTS \"{schema}\"; SET search_path TO \"{schema}\""
                        );
                        Executor::execute(conn, sea_orm::sqlx::AssertSqlSafe(sql)).await?;
                        Ok(())
                    })
                })
            });
        }
        let db = Database::connect(options).await?;
        Ok(Self { db })
    }

    pub async fn migrate(&self) -> Result<(), StorageError> {
        self.db
            .execute_unprepared(
                "CREATE TABLE IF NOT EXISTS alh_migrations (version TEXT PRIMARY KEY)",
            )
            .await?;
        if self.table_exists("providers").await? && !self.migration_applied("0001_init").await? {
            self.record_migration("0001_init").await?;
        }
        let tx = self.db.begin().await?;
        query::exec(&tx, "LOCK TABLE alh_migrations IN EXCLUSIVE MODE", vec![]).await?;
        for (version, sql) in MIGRATIONS {
            if Self::migration_flag(&tx, version).await? {
                continue;
            }
            for statement in statements(sql) {
                tx.execute_unprepared(&statement).await?;
            }
            query::exec(
                &tx,
                "INSERT INTO alh_migrations (version) VALUES ($1)",
                vec![Value::from(*version)],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub fn connection(&self) -> &DatabaseConnection {
        &self.db
    }

    pub async fn begin(&self) -> Result<DatabaseTransaction, StorageError> {
        Ok(self.db.begin().await?)
    }

    pub async fn exec_on(
        conn: &impl ConnectionTrait,
        sql: &str,
        values: Vec<Value>,
    ) -> Result<u64, StorageError> {
        Ok(query::exec(conn, sql, values).await?.rows_affected())
    }

    pub async fn count_on(
        conn: &impl ConnectionTrait,
        sql: &str,
        values: Vec<Value>,
    ) -> Result<i64, StorageError> {
        let Some(row) = query::one(conn, sql, values).await? else {
            return Err(StorageError::NotFound);
        };
        Ok(row.try_get_by_index(0)?)
    }

    pub async fn strings_on(
        conn: &impl ConnectionTrait,
        sql: &str,
        values: Vec<Value>,
    ) -> Result<Vec<String>, StorageError> {
        let rows = query::all(conn, sql, values).await?;
        rows.iter()
            .map(|row| Ok(row.try_get_by_index(0)?))
            .collect()
    }

    pub async fn ping(&self) -> Result<(), StorageError> {
        query::exec(&self.db, "SELECT 1", vec![]).await?;
        Ok(())
    }

    pub async fn current_schema(&self) -> Result<String, StorageError> {
        let Some(row) = query::one(&self.db, "SELECT current_schema() AS schema", vec![]).await?
        else {
            return Err(StorageError::NotFound);
        };
        Ok(row.try_get("", "schema")?)
    }

    pub async fn request_log_text(
        &self,
        user_id: Uuid,
        provider_id: Uuid,
    ) -> Result<Vec<String>, StorageError> {
        Self::strings_on(
            &self.db,
            "SELECT to_jsonb(request_log)::text FROM request_log
             WHERE user_id = $1 AND provider_id = $2",
            vec![Value::from(user_id), Value::from(provider_id)],
        )
        .await
    }

    pub async fn delete_provider_data(
        &self,
        user_id: Uuid,
        provider_id: Uuid,
    ) -> Result<(), StorageError> {
        let user = Value::from(user_id);
        let id = Value::from(provider_id);
        query::exec(
            &self.db,
            "DELETE FROM request_log WHERE user_id = $1 AND provider_id = $2",
            vec![user.clone(), id.clone()],
        )
        .await?;
        query::exec(
            &self.db,
            "DELETE FROM response_states WHERE user_id = $1 AND provider_id = $2",
            vec![user.clone(), id.clone()],
        )
        .await?;
        query::exec(
            &self.db,
            "DELETE FROM conversations WHERE user_id = $1 AND provider_id = $2",
            vec![user, id],
        )
        .await?;
        Ok(())
    }

    pub async fn seed_ollama_if_empty(&self, base_url: &str) -> Result<bool, StorageError> {
        let count = Self::count_on(&self.db, "SELECT COUNT(*) FROM providers", vec![]).await?;
        if count > 0 {
            return Ok(false);
        }
        match query::exec(
            &self.db,
            "INSERT INTO providers (
                id, protocol, display_name, url, token, default_model, is_default, enabled, user_id
             ) VALUES (
                gen_random_uuid(), 'openai', 'Ollama', $1, NULL, NULL, TRUE, TRUE,
                '00000000-0000-0000-0000-000000000001'
             )",
            vec![Value::from(with_v1(base_url))],
        )
        .await
        {
            Ok(_) => Ok(true),
            Err(err) if is_unique(&err) => Ok(false),
            Err(err) => Err(err.into()),
        }
    }

    pub async fn mark_interrupted(&self) -> Result<u64, StorageError> {
        let result = query::exec(
            &self.db,
            "UPDATE messages
             SET status = 'error', error_code = 'interrupted', finished_at = now()
             WHERE status = 'streaming'",
            vec![],
        )
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn purge_response_states(&self, days: i64) -> Result<u64, StorageError> {
        let result = query::exec(
            &self.db,
            "DELETE FROM response_states WHERE created_at < now() - ($1::bigint * interval '1 day')",
            vec![Value::from(days)],
        )
        .await?;
        Ok(result.rows_affected())
    }
}
