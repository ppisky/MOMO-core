CREATE TABLE memory_patch_reviews (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    patch_yaml TEXT NOT NULL,
    targets TEXT NOT NULL,
    operation_count INTEGER NOT NULL,
    review_mode TEXT NOT NULL CHECK (
        review_mode IN ('auto_approve', 'require_confirmation', 'reject')
    ),
    status TEXT NOT NULL CHECK (
        status IN ('pending', 'approved', 'rejected', 'failed')
    ),
    created_at TEXT NOT NULL,
    resolved_at TEXT,
    result TEXT,
    error TEXT
);

CREATE INDEX idx_memory_patch_reviews_owner_status_created
    ON memory_patch_reviews(owner_id, status, created_at DESC);
