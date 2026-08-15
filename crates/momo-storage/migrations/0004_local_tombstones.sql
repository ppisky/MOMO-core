CREATE TABLE local_tombstones (
    object_type TEXT NOT NULL,
    object_id TEXT NOT NULL,
    deleted_at TEXT NOT NULL,
    PRIMARY KEY (object_type, object_id)
);
