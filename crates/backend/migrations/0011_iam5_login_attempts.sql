CREATE TABLE iam_login_attempts (
    id TEXT PRIMARY KEY,
    state_hash TEXT NOT NULL,
    identity_kind TEXT NOT NULL CHECK (identity_kind IN ('carbon', 'silicon')),
    expires_at INTEGER NOT NULL,
    token_digest TEXT,
    result TEXT
);
CREATE INDEX iam_login_attempt_expiry ON iam_login_attempts (expires_at);
