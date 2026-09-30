-- Immutable, deduplicated role context for historical memory judgments.
CREATE TABLE memory_character_profiles (
    character_id TEXT NOT NULL,
    revision TEXT NOT NULL,
    profile TEXT NOT NULL,
    PRIMARY KEY (character_id, revision)
);
