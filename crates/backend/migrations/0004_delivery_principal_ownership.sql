-- Public IAM names can change or be reused; family ownership follows the immutable principal.
-- If an earlier database enrolled the same principal under two names, retain the
-- newest grant and send the older family through the existing revocation outbox.
UPDATE delivery_credentials AS older
SET enabled=0, state='revoking', operation=COALESCE(operation,'revoke'),
    mutation_key=COALESCE(mutation_key,lower(hex(randomblob(16))))
WHERE enabled=1 AND EXISTS (
    SELECT 1 FROM delivery_credentials AS newer
    WHERE newer.enabled=1 AND newer.org_id=older.org_id AND newer.principal_id=older.principal_id
      AND (newer.created_at>older.created_at OR (newer.created_at=older.created_at AND newer.rowid>older.rowid))
);
DROP INDEX delivery_one_enabled_actor;
CREATE UNIQUE INDEX delivery_one_enabled_principal ON delivery_credentials(org_id, principal_id) WHERE enabled=1;
