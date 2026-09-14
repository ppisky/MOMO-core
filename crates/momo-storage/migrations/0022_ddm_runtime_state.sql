ALTER TABLE mo_state_operations
    ADD COLUMN observed_scene_json TEXT NOT NULL DEFAULT '{}';

CREATE TABLE ddm_projection_states (
    managed_space_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    character_id TEXT NOT NULL,
    profile_revision INTEGER NOT NULL CHECK (profile_revision > 0),
    source_fingerprint TEXT NOT NULL,
    bands_json TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (managed_space_id, conversation_id, character_id),
    FOREIGN KEY(managed_space_id) REFERENCES mo_state_spaces(space_id) ON DELETE CASCADE
);
