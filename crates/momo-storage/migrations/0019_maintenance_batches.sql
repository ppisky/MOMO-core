CREATE TABLE maintenance_batches (
    batch_key TEXT PRIMARY KEY NOT NULL,
    scope_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('memory', 'semantic_graph')),
    request_ids_json TEXT NOT NULL,
    patch_yaml TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX maintenance_batches_scope_kind
    ON maintenance_batches(scope_id, kind, created_at);
