CREATE TABLE response_operations (
    request_id TEXT PRIMARY KEY NOT NULL,
    request_fingerprint TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    user_written INTEGER NOT NULL DEFAULT 0 CHECK (user_written IN (0, 1)),
    response_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
