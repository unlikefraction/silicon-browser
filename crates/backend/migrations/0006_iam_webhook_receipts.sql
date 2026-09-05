CREATE TABLE iam_webhook_receipts (
    event_id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    aggregate_id TEXT NOT NULL,
    aggregate_version INTEGER NOT NULL,
    received_at TEXT NOT NULL
);
CREATE INDEX iam_webhook_received_at ON iam_webhook_receipts(received_at);
