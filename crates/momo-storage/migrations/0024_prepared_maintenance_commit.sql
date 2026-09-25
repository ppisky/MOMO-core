-- NULL means model patch staged; non-NULL means exact file after-images prepared.
-- Acknowledgement removes the batch and marks its source turns in one transaction.
ALTER TABLE maintenance_batches ADD COLUMN prepared_commit_json TEXT;
ALTER TABLE memory_patch_reviews ADD COLUMN prepared_commit_json TEXT;
