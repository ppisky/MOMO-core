ALTER TABLE mo_state_operations ADD COLUMN response_request_id TEXT
    REFERENCES response_operations(request_id) ON DELETE SET NULL;

CREATE INDEX mo_state_operations_recent
    ON mo_state_operations(space_id, updated_at DESC, operation_id);
