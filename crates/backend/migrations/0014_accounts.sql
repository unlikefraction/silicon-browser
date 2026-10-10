-- Accounts auth state is separate from preserved IAM records and delivery receipts.
ALTER TABLE recording_artifacts ADD COLUMN upload_operation_id TEXT;

CREATE TABLE accounts_login_attempts (
 id TEXT PRIMARY KEY,
 state_hash TEXT NOT NULL,
 code_verifier_enc TEXT NOT NULL,
 redirect_uri TEXT NOT NULL,
 expires_at INTEGER NOT NULL,
 claimed_at INTEGER
);
CREATE INDEX accounts_login_expiry ON accounts_login_attempts(expires_at);
CREATE TABLE accounts_token_operations (
 token_digest TEXT PRIMARY KEY,
 operation TEXT NOT NULL,
 submitted_at INTEGER NOT NULL
);
CREATE TABLE accounts_webhook_receipts (
 event_id TEXT PRIMARY KEY,
 event_type TEXT NOT NULL,
 received_at TEXT NOT NULL
);
