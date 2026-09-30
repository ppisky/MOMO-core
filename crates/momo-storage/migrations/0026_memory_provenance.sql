ALTER TABLE response_operations ADD COLUMN user_message_id TEXT;
ALTER TABLE maintenance_batches ADD COLUMN provenance_json TEXT;
ALTER TABLE ddm_projection_states ADD COLUMN eligibility_key TEXT NOT NULL DEFAULT '';
CREATE TABLE response_evidence (
    request_id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    character_id TEXT NOT NULL,
    evidence_json TEXT NOT NULL,
    identity_json TEXT NOT NULL,
    revoked INTEGER NOT NULL DEFAULT 0,
    maintenance_stopped INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX response_evidence_conversation ON response_evidence(conversation_id);
CREATE TABLE memory_identity_bindings (
    conversation_id TEXT PRIMARY KEY,
    continuity_id TEXT NOT NULL,
    function_id TEXT
);
CREATE TABLE default_assistants (
    personal_space_id TEXT PRIMARY KEY,
    character_id TEXT NOT NULL
);
CREATE TABLE memory_evidence_controls (
    conversation_id TEXT PRIMARY KEY,
    stopped INTEGER NOT NULL,
    revoked INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE memory_evidence_control_audit (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    stopped INTEGER NOT NULL,
    revoked INTEGER NOT NULL,
    actor TEXT NOT NULL,
    created_at TEXT NOT NULL
);
