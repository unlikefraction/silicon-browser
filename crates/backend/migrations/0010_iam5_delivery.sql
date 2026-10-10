-- IAM 4 ordinary application families cannot become IAM 5 OBO grants.
UPDATE delivery_credentials SET enabled=0,state='needs_auth',operation=NULL,mutation_key=NULL,lease_owner=NULL,lease_until=0;

CREATE TABLE delivery_obo (
 id TEXT PRIMARY KEY,
 authorization_id TEXT UNIQUE,
 org_id TEXT NOT NULL,
 actor_id TEXT NOT NULL,
 world TEXT NOT NULL,
 enabled INTEGER NOT NULL DEFAULT 0,
 state TEXT NOT NULL DEFAULT 'pending',
 operation TEXT,
 mutation_key TEXT,
 encrypted_payload TEXT NOT NULL,
 destination_org TEXT,
 destination_actor TEXT,
 lease_owner TEXT,
 lease_until INTEGER NOT NULL DEFAULT 0,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX delivery_obo_enabled ON delivery_obo(org_id,actor_id,world) WHERE enabled=1;
ALTER TABLE sessions ADD COLUMN delivery_destination_org TEXT;
ALTER TABLE sessions ADD COLUMN delivery_destination_actor TEXT;
-- Old Browser jobs were explicitly bound to the initiating account and org.
UPDATE sessions SET delivery_destination_org=org_id,delivery_destination_actor=started_by WHERE delivery_principal_id IS NOT NULL;
ALTER TABLE recording_artifacts ADD COLUMN upload_operation_id TEXT;

-- Retired single-use uploads have no replayable operation ID. Preserve their
-- uncertain outcomes for reconciliation instead of uploading another version.
UPDATE recording_artifacts SET state='failed',last_error='legacy_upload_reconciliation_required',lease_id=NULL,lease_until=NULL
 WHERE body_sha256 IS NOT NULL AND state IN ('pending','working','uploading','failed');
UPDATE recordings SET status='failed' WHERE status <> 'trashed' AND session_id IN
 (SELECT session_id FROM recording_artifacts WHERE last_error='legacy_upload_reconciliation_required');
