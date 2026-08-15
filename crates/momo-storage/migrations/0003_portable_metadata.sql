CREATE TABLE portable_metadata (
    kind TEXT NOT NULL,
    object_id TEXT NOT NULL,
    document TEXT NOT NULL,
    PRIMARY KEY (kind, object_id)
);
