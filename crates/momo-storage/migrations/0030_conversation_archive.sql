ALTER TABLE conversations ADD COLUMN archived_at TEXT;
CREATE INDEX conversations_archive_scope
    ON conversations(scope_id, archived_at, updated_at DESC, id);
