DROP INDEX idx_memory_patch_reviews_owner_status_created;
DROP INDEX nsg_vectors_owner_space_idx;

ALTER TABLE character_cards RENAME COLUMN owner_id TO scope_id;
ALTER TABLE conversations RENAME COLUMN owner_id TO scope_id;
ALTER TABLE memory_patch_reviews RENAME COLUMN owner_id TO scope_id;
ALTER TABLE nsg_vectors RENAME COLUMN owner_id TO scope_id;

CREATE INDEX idx_memory_patch_reviews_scope_status_created
    ON memory_patch_reviews(scope_id, status, created_at DESC);

CREATE INDEX nsg_vectors_scope_space_idx
    ON nsg_vectors(scope_id, vector_space_id);
