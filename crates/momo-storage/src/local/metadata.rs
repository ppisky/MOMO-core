use super::*;

impl LocalStore {
    pub async fn save_portable_metadata(
        &self,
        kind: &str,
        object_id: &str,
        document: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            r#"INSERT INTO portable_metadata (kind, object_id, document) VALUES (?,?,?)
            ON CONFLICT(kind, object_id) DO UPDATE SET document=excluded.document"#,
        )
        .bind(kind)
        .bind(object_id)
        .bind(document)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn portable_metadata(
        &self,
        kind: &str,
        object_id: &str,
    ) -> Result<Option<String>, StorageError> {
        Ok(sqlx::query_scalar(
            "SELECT document FROM portable_metadata WHERE kind=? AND object_id=?",
        )
        .bind(kind)
        .bind(object_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn delete_portable_metadata(
        &self,
        kind: &str,
        object_id: &str,
    ) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM portable_metadata WHERE kind=? AND object_id=?")
            .bind(kind)
            .bind(object_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_character_ddm_profile(
        &self,
        character_id: &str,
    ) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "DELETE FROM portable_metadata WHERE kind='character_ddm_profile' AND object_id=?",
        )
        .bind(character_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("DELETE FROM ddm_projection_states WHERE character_id=?")
            .bind(character_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }
}
