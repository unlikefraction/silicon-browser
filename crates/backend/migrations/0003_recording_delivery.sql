-- Each artifact has its own durable receipt. Completing the video must not
-- upload it again merely because command-log delivery subsequently failed.
ALTER TABLE sessions ADD COLUMN delivery_principal_id TEXT;
ALTER TABLE sessions ADD COLUMN delivery_membership_id TEXT;
CREATE TABLE recording_artifacts (
    session_id TEXT NOT NULL REFERENCES sessions(id),
    kind TEXT NOT NULL CHECK (kind IN ('video', 'commands')),
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'working', 'uploading', 'complete', 'failed')),
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TEXT NOT NULL,
    lease_id TEXT,
    lease_until TEXT,
    body_sha256 TEXT,
    size_bytes INTEGER CHECK (size_bytes IS NULL OR size_bytes >= 0),
    entry_id TEXT,
    artifact_path TEXT,
    receipt_url_enc TEXT,
    last_error TEXT,
    completed_at TEXT,
    PRIMARY KEY (session_id, kind)
);
CREATE INDEX recording_artifacts_pending ON recording_artifacts (next_attempt_at, lease_until)
    WHERE state IN ('pending', 'working', 'uploading');
