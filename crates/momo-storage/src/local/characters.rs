use super::*;

impl LocalStore {
    /// Create a default identity only when the Space has no mapping. Both rows
    /// commit together; concurrent callers receive the same durable identity.
    pub async fn ensure_default_assistant(
        &self,
        card: &CharacterCard,
    ) -> Result<CharacterCard, StorageError> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let mapped: Option<String> = sqlx::query_scalar(
            "SELECT character_id FROM default_assistants WHERE personal_space_id=?",
        )
        .bind(card.scope_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(id) = mapped {
            let row = sqlx::query("SELECT * FROM character_cards WHERE id=?")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
            let existing = character_from_row(&row)?;
            tx.commit().await?;
            return Ok(existing);
        }
        sqlx::query("INSERT INTO character_cards (id,scope_id,name,version,description,language,tags,author_uid,author_display_name,author_name,author_url,character_markdown,user_markdown,opening_markdown,created_at,updated_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
            .bind(card.id.to_string()).bind(card.scope_id.to_string()).bind(&card.name).bind(&card.version)
            .bind("").bind("").bind("[]").bind("").bind(&card.author_name).bind(&card.author_name)
            .bind(&card.author_url).bind(&card.character_markdown).bind(&card.user_markdown).bind(&card.opening_markdown)
            .bind(card.created_at.to_rfc3339()).bind(card.updated_at.to_rfc3339()).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO default_assistants(personal_space_id,character_id) VALUES(?,?)")
            .bind(card.scope_id.to_string())
            .bind(card.id.to_string())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(card.clone())
    }

    pub async fn save_character(&self, card: &CharacterCard) -> Result<(), StorageError> {
        if self.is_tombstoned("character", card.id).await? {
            return Ok(());
        }
        sqlx::query(
            r#"INSERT INTO character_cards
            (id, scope_id, name, version, description, language, tags, author_uid,
             author_display_name, author_name, author_url, character_markdown, user_markdown,
             opening_markdown, created_at, updated_at)
            VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
            ON CONFLICT(id) DO UPDATE SET
              scope_id=excluded.scope_id, name=excluded.name, version=excluded.version,
              author_display_name=excluded.author_display_name,
              author_name=excluded.author_name, author_url=excluded.author_url,
              character_markdown=excluded.character_markdown, user_markdown=excluded.user_markdown,
              opening_markdown=excluded.opening_markdown,
              updated_at=excluded.updated_at"#,
        )
        .bind(card.id.to_string())
        .bind(card.scope_id.to_string())
        .bind(&card.name)
        .bind(&card.version)
        .bind("")
        .bind("")
        .bind("[]")
        .bind("")
        .bind(&card.author_name)
        .bind(&card.author_name)
        .bind(&card.author_url)
        .bind(&card.character_markdown)
        .bind(&card.user_markdown)
        .bind(&card.opening_markdown)
        .bind(card.created_at.to_rfc3339())
        .bind(card.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_characters(&self) -> Result<Vec<CharacterCard>, StorageError> {
        let rows = sqlx::query("SELECT * FROM character_cards ORDER BY updated_at DESC, id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(character_from_row).collect()
    }

    pub async fn list_characters_for_scope(
        &self,
        scope_id: Uuid,
    ) -> Result<Vec<CharacterCard>, StorageError> {
        let rows = sqlx::query(
            "SELECT * FROM character_cards WHERE scope_id=? ORDER BY updated_at DESC, id",
        )
        .bind(scope_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(character_from_row).collect()
    }

    pub async fn character_by_id(
        &self,
        character_id: Uuid,
    ) -> Result<Option<CharacterCard>, StorageError> {
        let row = sqlx::query("SELECT * FROM character_cards WHERE id=?")
            .bind(character_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(character_from_row).transpose()
    }

    pub async fn character_for_scope(
        &self,
        scope_id: Uuid,
        character_id: Uuid,
    ) -> Result<Option<CharacterCard>, StorageError> {
        let row = sqlx::query("SELECT * FROM character_cards WHERE id=? AND scope_id=?")
            .bind(character_id.to_string())
            .bind(scope_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(character_from_row).transpose()
    }

    pub async fn stage_character(&self, card: &CharacterCard) -> Result<(), StorageError> {
        self.save_character(card).await
    }

    pub async fn stage_character_update(&self, card: &CharacterCard) -> Result<(), StorageError> {
        self.save_character(card).await
    }

    pub async fn stage_character_delete(&self, id: Uuid) -> Result<(), StorageError> {
        self.stage_delete("character", "delete_character", id).await
    }
}
