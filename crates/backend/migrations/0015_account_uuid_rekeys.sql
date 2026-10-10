-- An audit ledger and retired-subject fence, never an authentication alias.
CREATE TABLE IF NOT EXISTS account_uuid_rekeys (
 old_uuid TEXT PRIMARY KEY,
 new_uuid TEXT NOT NULL UNIQUE,
 kind TEXT NOT NULL CHECK (kind IN ('carbon', 'silicon')),
 map_sha256 TEXT NOT NULL,
 migrated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS account_uuid_cutover (
 singleton INTEGER PRIMARY KEY CHECK (singleton=1),
 map_sha256 TEXT NOT NULL,
 migrated_at TEXT NOT NULL
);
