use super::*;
impl Store {
    pub(super) async fn record_migration(&self, version: &str) -> Result<(), StorageError> {
        query::exec(
            &self.db,
            "INSERT INTO alh_migrations (version) VALUES ($1) ON CONFLICT (version) DO NOTHING",
            vec![Value::from(version)],
        )
        .await?;
        Ok(())
    }
}
