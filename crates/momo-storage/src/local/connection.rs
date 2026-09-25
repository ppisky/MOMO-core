use super::*;

impl LocalStore {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true);
        Self::connect(options).await
    }

    pub async fn in_memory() -> Result<Self, StorageError> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?
            .foreign_keys(true)
            .shared_cache(true);
        Self::connect(options).await
    }

    async fn connect(options: SqliteConnectOptions) -> Result<Self, StorageError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        // Control actions are idempotent state transitions, but they span
        // SQLite and file-backed memory. A process can therefore stop after
        // claiming an action and before recording its response. Only
        // completed responses are durable replay records; an unfinished claim
        // is released when the single owner of this database starts again.
        sqlx::query("DELETE FROM control_operations WHERE response_json IS NULL")
            .execute(&pool)
            .await?;
        Ok(Self { pool })
    }

    #[must_use]
    pub const fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}
