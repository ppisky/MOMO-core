CREATE TABLE control_operations (
    operation_key TEXT PRIMARY KEY NOT NULL,
    request_fingerprint TEXT NOT NULL,
    response_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
