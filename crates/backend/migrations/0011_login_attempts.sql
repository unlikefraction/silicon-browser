-- Browser callbacks bind to the selected identity kind and one exact SLT.
-- State and token material remain out of the database; only digests are stored.
CREATE TABLE login_attempts (
 id TEXT PRIMARY KEY,
 identity_kind TEXT NOT NULL CHECK(identity_kind IN ('carbon','silicon')),
 state_digest TEXT NOT NULL,
 token_digest TEXT,
 rejected INTEGER NOT NULL DEFAULT 0,
 expires_at INTEGER NOT NULL
);
CREATE INDEX login_attempts_expiry ON login_attempts(expires_at);
