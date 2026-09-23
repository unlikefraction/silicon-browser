# Public identifier migration

Stop Browser API, TTL/recording workers and test-world writers. Back up every SQLite database (use SQLite backup or checkpoint with all writers stopped), its sidecars if present, encryption key and deployment configuration together. Rehearse against copies first. Never start the old binary after any database has been converted.

Update `IAM_APP_ID` and the configured Briefcase audience to bare app IDs. Keep organisation selection explicit. For each production and testing SQLite store, supply an authenticated IAM export scoped to that world and local membership organisation:

```json
[
  {"org_id":"bricks","kind":"silicon","old_id":"chef:bricks","new_id":"si:chef"},
  {"org_id":"bricks","kind":"carbon","old_id":"saket","new_id":"c:saket"}
]
```

Use the existing backend configuration and encryption key, with `SB_DATABASE_URL` pointing at the selected store, then run:

```sh
silicon-browser-backend --migrate-public-identifiers mapping.json
```

This command starts no HTTP listener or workers. Earlier private-UUID identity projections are canonicalized first using already verified directory records. The public namespace conversion then runs in one transaction, refuses ambiguous/colliding/unmapped identities, and can be repeated with the same mapping. A missing local projection must be recovered from an operator-verified IAM inventory; never guess ownership. Normal production and test-world startup refuses unmigrated identity references.

Profile/session/recording IDs, ownership, ACLs and participants remain attached to the same resources. Encrypted command and delivery credentials are authenticated and re-encrypted with the new IDs; refresh family tokens and operation keys remain unchanged. Frozen command artifact bytes, hashes and permanent Briefcase receipts remain unchanged. Do not replace old IDs inside provider URLs, file bytes or historical delivery paths. Migration tests cover encrypted credentials, command replay, frozen artifacts, collisions and transactional rollback.

Drain or reconcile unfinished IAM requests before the coordinated IAM cutover. Clear in-memory authorization caches by restarting. Interactive CLI/browser sessions can use their preserved refresh family after the IAM migration; clients must refresh or log in before showing a stale cached identity. Do not edit opaque access/refresh tokens locally. Testing stores use the same process with their own IAM-world mapping, never a production selector.

Roll back only by stopping all upgraded services and restoring the coordinated pre-cutover databases, keys, config and binaries. After new writes, fix forward or reconcile those writes before restoring a snapshot.

## September 23 operator

After the authorized test reset, `deploy/public-id-cutover.py rehearse --binary PATH --expected-binary-sha256 SHA256 --mapping mapping.json --snapshot-directory NEW_DIRECTORY` makes SQLite snapshots, retains the runtime configuration and exact mapping, and runs the new binary offline against copies. It verifies all non-identity ledger values are unchanged. It refuses unexpected test stores or registrations. The `apply` action additionally requires the service to be stopped, takes a fresh backup and rehearsal, then converts the production store. Failure restores the fresh snapshot while keeping the service stopped. The operator does not activate a release or persist application configuration changes; coordinate `IAM_APP_ID=browser` and `BRIEFCASE_APP_ID=briefcase` with IAM before starting the candidate.
