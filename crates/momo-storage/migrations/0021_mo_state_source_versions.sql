CREATE TABLE mo_state_source_versions (
    managed_space_id TEXT NOT NULL,
    source_space_id TEXT NOT NULL,
    dmw_revision INTEGER NOT NULL DEFAULT 0 CHECK (dmw_revision >= 0),
    nsg_revision INTEGER NOT NULL DEFAULT 0 CHECK (nsg_revision >= 0),
    scene_revision INTEGER NOT NULL DEFAULT 0 CHECK (scene_revision >= 0),
    dmw_fingerprint TEXT NOT NULL DEFAULT '',
    nsg_fingerprint TEXT NOT NULL DEFAULT '',
    scene_fingerprint TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL,
    PRIMARY KEY (managed_space_id, source_space_id),
    FOREIGN KEY(managed_space_id) REFERENCES mo_state_spaces(space_id) ON DELETE CASCADE
);

ALTER TABLE mo_state_operations
    ADD COLUMN source_versions_json TEXT NOT NULL DEFAULT '[]';
