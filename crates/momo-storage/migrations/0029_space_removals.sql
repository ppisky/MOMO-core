-- Exact after-images are retained until cross-store acknowledgement. Replaying
-- the same operation cannot migrate twice or change the user's disposition.
CREATE TABLE space_removals (
    operation_id TEXT PRIMARY KEY NOT NULL,
    source_id TEXT NOT NULL UNIQUE,
    request_json TEXT NOT NULL,
    plan_json TEXT,
    receipt_json TEXT NOT NULL,
    completed INTEGER NOT NULL DEFAULT 0 CHECK (completed IN (0, 1))
);
CREATE TRIGGER retired_character_insert BEFORE INSERT ON character_cards
WHEN EXISTS (SELECT 1 FROM space_removals WHERE source_id=NEW.scope_id)
 OR EXISTS (SELECT 1 FROM space_removals WHERE completed=0 AND json_extract(request_json,'$.disposition.target_id')=NEW.scope_id)
BEGIN SELECT RAISE(ABORT, 'space retired or removal pending'); END;
CREATE TRIGGER retired_character_update BEFORE UPDATE ON character_cards
WHEN EXISTS (SELECT 1 FROM space_removals WHERE source_id=NEW.scope_id)
 OR EXISTS (SELECT 1 FROM space_removals WHERE completed=0 AND json_extract(request_json,'$.disposition.target_id')=NEW.scope_id)
BEGIN SELECT RAISE(ABORT, 'space retired or removal pending'); END;
CREATE TRIGGER retired_conversation_insert BEFORE INSERT ON conversations
WHEN EXISTS (SELECT 1 FROM space_removals WHERE source_id=NEW.scope_id)
 OR EXISTS (SELECT 1 FROM space_removals WHERE completed=0 AND json_extract(request_json,'$.disposition.target_id')=NEW.scope_id)
BEGIN SELECT RAISE(ABORT, 'space retired or removal pending'); END;
CREATE TRIGGER retired_conversation_update BEFORE UPDATE ON conversations
WHEN EXISTS (SELECT 1 FROM space_removals WHERE source_id=NEW.scope_id)
 OR EXISTS (SELECT 1 FROM space_removals WHERE completed=0 AND json_extract(request_json,'$.disposition.target_id')=NEW.scope_id)
BEGIN SELECT RAISE(ABORT, 'space retired or removal pending'); END;
CREATE TRIGGER retired_turn_insert BEFORE INSERT ON maintenance_turns
WHEN EXISTS (SELECT 1 FROM space_removals WHERE source_id=NEW.scope_id)
 OR EXISTS (SELECT 1 FROM space_removals WHERE completed=0 AND json_extract(request_json,'$.disposition.target_id')=NEW.scope_id)
BEGIN SELECT RAISE(ABORT, 'space retired or removal pending'); END;
CREATE TRIGGER retired_batch_insert BEFORE INSERT ON maintenance_batches
WHEN EXISTS (SELECT 1 FROM space_removals WHERE source_id=NEW.scope_id)
 OR EXISTS (SELECT 1 FROM space_removals WHERE completed=0 AND json_extract(request_json,'$.disposition.target_id')=NEW.scope_id)
BEGIN SELECT RAISE(ABORT, 'space retired or removal pending'); END;
CREATE TRIGGER retired_review_insert BEFORE INSERT ON memory_patch_reviews
WHEN EXISTS (SELECT 1 FROM space_removals WHERE source_id=NEW.scope_id)
 OR EXISTS (SELECT 1 FROM space_removals WHERE completed=0 AND json_extract(request_json,'$.disposition.target_id')=NEW.scope_id)
BEGIN SELECT RAISE(ABORT, 'space retired or removal pending'); END;
CREATE TRIGGER retired_lifecycle_insert BEFORE INSERT ON memory_lifecycle_events
WHEN EXISTS (SELECT 1 FROM space_removals WHERE source_id=NEW.space_id)
BEGIN SELECT RAISE(ABORT, 'space retired or removal pending'); END;
CREATE TRIGGER retired_assistant_insert BEFORE INSERT ON default_assistants
WHEN EXISTS (SELECT 1 FROM space_removals WHERE source_id=NEW.personal_space_id)
 OR EXISTS (SELECT 1 FROM space_removals WHERE completed=0 AND json_extract(request_json,'$.disposition.target_id')=NEW.personal_space_id)
BEGIN SELECT RAISE(ABORT, 'space retired or removal pending'); END;
CREATE TRIGGER retired_assistant_update BEFORE UPDATE ON default_assistants
WHEN EXISTS (SELECT 1 FROM space_removals WHERE source_id=NEW.personal_space_id)
 OR EXISTS (SELECT 1 FROM space_removals WHERE completed=0 AND json_extract(request_json,'$.disposition.target_id')=NEW.personal_space_id)
BEGIN SELECT RAISE(ABORT, 'space retired or removal pending'); END;
