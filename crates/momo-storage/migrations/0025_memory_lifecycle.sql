-- Durable completed-interaction events. No FK cascade from conversations:
-- deleting chat preserves memory and pending maintenance by contract.
CREATE TABLE memory_lifecycle_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id TEXT NOT NULL UNIQUE,
    space_id TEXT NOT NULL,
    activity_json TEXT NOT NULL,
    prepared_commit_json TEXT,
    report_json TEXT,
    completed INTEGER NOT NULL DEFAULT 0 CHECK (completed IN (0, 1))
);
CREATE INDEX memory_lifecycle_pending ON memory_lifecycle_events(space_id, completed, sequence);
