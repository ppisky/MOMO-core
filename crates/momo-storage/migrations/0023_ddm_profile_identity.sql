ALTER TABLE ddm_projection_states
    ADD COLUMN profile_fingerprint TEXT NOT NULL DEFAULT '';

CREATE INDEX idx_ddm_projection_states_character
    ON ddm_projection_states(character_id);
