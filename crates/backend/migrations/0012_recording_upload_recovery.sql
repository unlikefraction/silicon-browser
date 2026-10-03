-- Preserve deployed IAM 5 operation identities and bind future jobs at session start.
ALTER TABLE sessions ADD COLUMN delivery_destination_org TEXT;
ALTER TABLE sessions ADD COLUMN delivery_destination_actor TEXT;
-- A previous release already pinned destinations per artifact. Adopt only an
-- unambiguous saved destination, never the currently selected consent account.
UPDATE sessions SET
 delivery_destination_org=(SELECT MIN(storage_org_id) FROM recording_artifacts WHERE session_id=sessions.id),
 delivery_destination_actor=(SELECT MIN(storage_actor_id) FROM recording_artifacts WHERE session_id=sessions.id)
 WHERE id IN (SELECT session_id FROM recording_artifacts
  WHERE storage_org_id IS NOT NULL AND storage_actor_id IS NOT NULL
  GROUP BY session_id HAVING COUNT(DISTINCT storage_org_id)=1 AND COUNT(DISTINCT storage_actor_id)=1);
-- IAM 4 writes have no replayable reservation or verified destination. Hold
-- uncertain outcomes, including failed retries, instead of making another version.
UPDATE recording_artifacts SET state='failed',last_error='legacy_upload_reconciliation_required',lease_id=NULL,lease_until=NULL
 WHERE body_sha256 IS NOT NULL AND storage_org_id IS NULL AND state IN ('pending','working','uploading','failed');
UPDATE recordings SET status='failed' WHERE status <> 'trashed' AND session_id IN
 (SELECT session_id FROM recording_artifacts WHERE last_error='legacy_upload_reconciliation_required');
