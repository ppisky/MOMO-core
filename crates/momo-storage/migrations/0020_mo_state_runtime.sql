CREATE TABLE mo_state_spaces (
    space_id TEXT PRIMARY KEY NOT NULL,
    profile TEXT NOT NULL,
    dmw_revision INTEGER NOT NULL DEFAULT 0 CHECK (dmw_revision >= 0),
    nsg_revision INTEGER NOT NULL DEFAULT 0 CHECK (nsg_revision >= 0),
    scene_revision INTEGER NOT NULL DEFAULT 0 CHECK (scene_revision >= 0),
    snapshot_revision INTEGER NOT NULL DEFAULT 0 CHECK (snapshot_revision >= 0),
    dmw_fingerprint TEXT NOT NULL DEFAULT '',
    nsg_fingerprint TEXT NOT NULL DEFAULT '',
    scene_fingerprint TEXT NOT NULL DEFAULT '',
    scene_json TEXT NOT NULL DEFAULT '{}',
    current_snapshot_json TEXT,
    degraded INTEGER NOT NULL DEFAULT 0 CHECK (degraded IN (0, 1)),
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE mo_state_operations (
    operation_id TEXT PRIMARY KEY NOT NULL,
    space_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    event_fingerprint TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (
        phase IN ('staged', 'applying', 'projected', 'completed', 'failed')
    ),
    base_dmw_revision INTEGER NOT NULL CHECK (base_dmw_revision >= 0),
    base_nsg_revision INTEGER NOT NULL CHECK (base_nsg_revision >= 0),
    base_scene_revision INTEGER NOT NULL CHECK (base_scene_revision >= 0),
    snapshot_json TEXT,
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(space_id) REFERENCES mo_state_spaces(space_id) ON DELETE CASCADE
);

CREATE INDEX mo_state_operations_space_phase
    ON mo_state_operations(space_id, phase, created_at);
