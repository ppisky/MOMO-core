CREATE TABLE message_context (
    message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    payload_json TEXT NOT NULL
);
