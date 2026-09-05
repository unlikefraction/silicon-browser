-- Client reports are cooperative telemetry. They never execute browser commands.
CREATE TABLE command_reports (
    session_id TEXT NOT NULL REFERENCES sessions(id),
    command_id TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    principal_id TEXT NOT NULL,
    payload_sha256 TEXT NOT NULL,
    sequence INTEGER,
    received_at TEXT NOT NULL,
    PRIMARY KEY(session_id, command_id)
);
