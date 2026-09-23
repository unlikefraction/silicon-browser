-- The offline cutover records its exact world and authenticated IAM mapping.
CREATE TABLE public_identifier_schema (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    scope_key TEXT NOT NULL,
    mapping_json TEXT
);
