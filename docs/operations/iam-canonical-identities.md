# IAM canonical identity compatibility

The backend uses IAM SDK 3.0.1 and derives ownership from the verified canonical identity handle. Its introspection reader also accepts the deployed UUID response when the private identity matches its authorization snapshot; the UUID never becomes an ownership key. Public Browser clients and routes are unchanged.

Before workers start, verified `identity_projection` rows migrate existing ownership, command-report replay authority and encrypted delivery credentials to the canonical handle. The same encryption key is required. Grant IDs, enrollment digests, operation/mutation keys, leases, pending work and completed receipts are preserved. A missing verified mapping or ciphertext authentication failure stops conversion. Test databases undergo the same conversion within their own storage boundary.

Migration 0008 retains a historical command actor spelling only when an existing command artifact already has a frozen content digest. Recording delivery can then regenerate identical JSONL bytes after an uncertain upload. Active ownership, API logs and command ciphertext contexts use the canonical identity.

Run the candidate's offline mode on every restored database before restarting the service. It starts neither the API nor provider/credential workers:

```sh
python3 /opt/silicon-browser/releases/RELEASE/canonical-cutover.py rehearse \
  --binary /opt/silicon-browser/releases/RELEASE/silicon-browser-backend \
  --snapshot-directory /var/lib/silicon-browser/canonical-rehearsal-TIMESTAMP
```

The operator helper snapshots production and every registered testing database, checks SQLite integrity, converts separate copies using the unchanged runtime key, and verifies every retained non-identity column. The manifest records database checksums, row counts, candidate hash, previous release and runtime configuration hash without credentials.

For the actual cutover, stop the service and backup timer, invoke the same helper with `apply` and a new snapshot directory, then switch the release symlink and start the service. The helper refuses `apply` while the service is active. It retains complete pre-conversion snapshots and restores all databases if offline conversion fails. Preserve these snapshots and the same key; upload the database snapshots to the private backup bucket as well.

The ordinary installer only rolls back executables/configuration, so do not use its automatic rollback for this identity conversion. Before the new daemon starts, rollback must pair all pre-conversion databases with the previous binary. After new workers have run, preserve their database state and reconcile any external token rotation or delivery before restoring an older snapshot. Never silently replace an existing IAM credential family or drop uncertain pending work.
