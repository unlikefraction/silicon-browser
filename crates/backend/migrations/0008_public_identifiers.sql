-- The offline cutover records its exact world and authenticated IAM mapping.
CREATE TABLE public_identifier_schema (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    scope_key TEXT NOT NULL,
    mapping_json TEXT
);

-- A command archive can be regenerated after an uncertain upload. Its frozen
-- actor spelling is historical content, independent of current authorization.
ALTER TABLE commands ADD COLUMN artifact_actor_id TEXT;
