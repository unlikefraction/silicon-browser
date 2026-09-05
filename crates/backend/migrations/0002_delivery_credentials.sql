CREATE TABLE delivery_credentials (
 id TEXT PRIMARY KEY,
 org_id TEXT NOT NULL,
 actor_id TEXT NOT NULL,
 principal_id TEXT NOT NULL,
 membership_id TEXT NOT NULL,
 actor_kind TEXT NOT NULL,
 enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
 state TEXT NOT NULL,
 operation TEXT,
 mutation_key TEXT,
 enrollment_digest TEXT NOT NULL,
 encrypted_payload TEXT NOT NULL,
 access_expires_at INTEGER NOT NULL DEFAULT 0,
 lease_owner TEXT,
 lease_until INTEGER NOT NULL DEFAULT 0,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX delivery_one_enabled_actor ON delivery_credentials(org_id, actor_id) WHERE enabled=1;
CREATE UNIQUE INDEX delivery_enrollment_replay ON delivery_credentials(org_id, actor_id, enrollment_digest);
CREATE INDEX delivery_recovery ON delivery_credentials(operation, lease_until);
