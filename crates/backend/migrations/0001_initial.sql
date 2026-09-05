PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS profiles (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL,
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    provider_profile_id TEXT NOT NULL UNIQUE,
    fingerprint TEXT NOT NULL UNIQUE,
    location TEXT NOT NULL CHECK (length(location) = 2),
    access_json TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('provisioning', 'active', 'ended', 'failed')),
    created_at TEXT NOT NULL,
    ended_at TEXT,
    end_note TEXT
);

CREATE INDEX IF NOT EXISTS profiles_org_created ON profiles (org_id, created_at DESC);

-- IAM application-token introspection proves an internal principal UUID but,
-- after exchange, no longer returns its public id. Persist only that verified
-- exchange-time mapping so ACL ownership stays stable across requests/restarts.
CREATE TABLE IF NOT EXISTS identity_projection (
    org_id TEXT NOT NULL,
    principal_id TEXT NOT NULL,
    public_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('carbon', 'silicon')),
    updated_at TEXT NOT NULL,
    PRIMARY KEY (org_id, principal_id),
    UNIQUE (org_id, public_id)
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL,
    profile_id TEXT REFERENCES profiles(id),
    provider_session_id TEXT UNIQUE,
    started_by TEXT NOT NULL,
    started_by_kind TEXT NOT NULL CHECK (started_by_kind IN ('carbon', 'silicon')),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    description TEXT NOT NULL CHECK (length(description) BETWEEN 1 AND 2000),
    status TEXT NOT NULL DEFAULT 'starting' CHECK (status IN ('starting', 'active', 'ending', 'ended', 'expired', 'failed')),
    started_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    ended_at TEXT,
    end_note TEXT,
    provider_cdp_url_enc TEXT,
    provider_live_url_enc TEXT,
    provider_recording_url_enc TEXT,
    stop_lease_id TEXT,
    stop_lease_until TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS one_live_session_per_profile
    ON sessions(profile_id)
    WHERE profile_id IS NOT NULL AND status IN ('starting', 'active', 'ending');
CREATE INDEX IF NOT EXISTS sessions_org_started ON sessions (org_id, started_at DESC);
CREATE INDEX IF NOT EXISTS sessions_expiry ON sessions (status, expires_at);
CREATE INDEX IF NOT EXISTS sessions_stop_lease ON sessions (status, expires_at, stop_lease_until);

CREATE TABLE IF NOT EXISTS session_participants (
    session_id TEXT NOT NULL REFERENCES sessions(id),
    actor_id TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('initiator', 'runner', 'viewer')),
    first_seen_at TEXT NOT NULL,
    PRIMARY KEY (session_id, actor_id, role)
);

CREATE TABLE IF NOT EXISTS commands (
    session_id TEXT NOT NULL REFERENCES sessions(id),
    sequence INTEGER NOT NULL,
    actor_id TEXT NOT NULL,
    command_enc TEXT NOT NULL,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    exit_code INTEGER,
    PRIMARY KEY (session_id, sequence)
);

CREATE TABLE IF NOT EXISTS recordings (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id),
    owner_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'recording' CHECK (status IN ('recording', 'pending', 'available', 'trashed', 'failed')),
    artifact_path TEXT NOT NULL,
    briefcase_url_enc TEXT,
    duration_ms INTEGER,
    size_bytes INTEGER,
    trashed_at TEXT,
    purge_after TEXT
);

CREATE TABLE IF NOT EXISTS usage (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id),
    browser_millis INTEGER NOT NULL DEFAULT 0 CHECK (browser_millis >= 0),
    proxy_bytes_in INTEGER CHECK (proxy_bytes_in IS NULL OR proxy_bytes_in >= 0),
    proxy_bytes_out INTEGER CHECK (proxy_bytes_out IS NULL OR proxy_bytes_out >= 0),
    proxy_bytes_unclassified INTEGER CHECK (proxy_bytes_unclassified IS NULL OR proxy_bytes_unclassified >= 0),
    proxy_megabytes TEXT,
    browser_cost TEXT NOT NULL DEFAULT '0',
    proxy_cost TEXT NOT NULL DEFAULT '0',
    currency TEXT NOT NULL DEFAULT 'USD',
    sampled_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS outbox (
    id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    done_at TEXT
);

CREATE INDEX IF NOT EXISTS outbox_pending ON outbox (next_attempt_at) WHERE done_at IS NULL;

CREATE TABLE IF NOT EXISTS discovery_log (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('search', 'fetch')),
    purpose TEXT NOT NULL,
    item_count INTEGER NOT NULL,
    created_at TEXT NOT NULL
);
