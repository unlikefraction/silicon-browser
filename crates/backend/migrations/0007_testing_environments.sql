-- Only encrypted configuration lives here. All test application data lives in
-- a separate SQLite database for its IAM environment and clean generation.
CREATE TABLE testing_environments (
    namespace TEXT PRIMARY KEY NOT NULL,
    environment_id TEXT NOT NULL,
    credentials TEXT NOT NULL
);
