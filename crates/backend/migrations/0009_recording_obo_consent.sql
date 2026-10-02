-- Old ordinary OAuth credentials cannot become OBO consent. Recover only their
-- revocation, never a pending exchange/refresh. Existing job claims are retained.
UPDATE delivery_credentials SET enabled=0,state='revoking',operation='revoke',
 mutation_key=COALESCE(mutation_key,id),lease_owner=NULL,lease_until=0;
CREATE TABLE recording_obo_authorizations (
 id TEXT PRIMARY KEY, org_id TEXT NOT NULL, principal_id TEXT NOT NULL,
 membership_id TEXT NOT NULL, actor_id TEXT NOT NULL, retry_key TEXT NOT NULL,
 request_cipher TEXT NOT NULL, state_cipher TEXT NOT NULL, state_digest TEXT NOT NULL,
 iam_id TEXT, consent_url TEXT, status TEXT NOT NULL DEFAULT 'pending',
 code_digest TEXT, expires_at INTEGER NOT NULL,
 UNIQUE(org_id,principal_id,membership_id,retry_key)
);
CREATE TABLE recording_obo_grants (
 org_id TEXT NOT NULL, principal_id TEXT NOT NULL, membership_id TEXT NOT NULL,
 actor_id TEXT NOT NULL, tokens_cipher TEXT NOT NULL, credential_version TEXT NOT NULL DEFAULT '',
 enabled INTEGER NOT NULL DEFAULT 1, needs_auth INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(org_id,principal_id,membership_id)
);

ALTER TABLE recording_artifacts ADD COLUMN storage_org_id TEXT;
ALTER TABLE recording_artifacts ADD COLUMN storage_actor_id TEXT;
