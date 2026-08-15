CREATE TABLE nsg_vectors (
    owner_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    vector_space_id TEXT NOT NULL,
    dimension INTEGER NOT NULL CHECK (dimension > 0 AND dimension <= 8192),
    vector_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (owner_id, node_id, vector_space_id)
);

CREATE INDEX nsg_vectors_owner_space_idx
ON nsg_vectors (owner_id, vector_space_id);
