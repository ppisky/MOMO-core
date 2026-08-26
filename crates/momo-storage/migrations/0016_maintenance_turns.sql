CREATE TABLE maintenance_turns (
    request_id TEXT PRIMARY KEY NOT NULL,
    scope_id TEXT NOT NULL,
    user_content TEXT NOT NULL,
    assistant_content TEXT NOT NULL,
    memory_done INTEGER NOT NULL DEFAULT 0 CHECK (memory_done IN (0, 1)),
    nsg_done INTEGER NOT NULL DEFAULT 0 CHECK (nsg_done IN (0, 1)),
    created_at TEXT NOT NULL
);

CREATE INDEX maintenance_turns_scope_created
    ON maintenance_turns(scope_id, created_at, request_id);
