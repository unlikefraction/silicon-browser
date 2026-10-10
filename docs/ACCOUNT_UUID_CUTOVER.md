# Existing Accounts UUID cutover

This procedure upgrades Browser's **existing Accounts database** from legacy
base62 account IDs to the authoritative Accounts UUIDv4 export. It does not
import or relabel IAM identities. Keep the old IAM database and key untouched.
Coordinate this window with Accounts and Silicon Apps; do not independently
apply a local test map or infer identity from a public handle.

## Candidate and backup rehearsal

Build the native backend from the reviewed commit with `backend-release.yml`.
It produces an optimized ARM64 GNU/Linux binary with a glibc 2.34 baseline,
`SHA256SUMS`, and `SOURCE_REVISION`. Build the Vite frontend from the same commit;
both Rust and frontend validators must accept canonical hyphenated UUIDs.
The additive 0015 schema can be installed before cutover: the retirement fence
only activates once the offline migration commits.

Use a SQLite online backup of the current Accounts database for the rehearsal.
Never copy only the main file while WAL writers are active. Load the existing
`SB_ENCRYPTION_KEY` privately, set `SB_DATABASE_URL` to the backup clone, and run:

```sh
CANDIDATE --migrate-account-uuids /private/account-uuid-map.csv
CANDIDATE --migrate-account-uuids /private/account-uuid-map.csv --apply --writers-stopped
CANDIDATE --migrate-account-uuids /private/account-uuid-map.csv --apply --writers-stopped
```

The final report must show `already_applied: true` and an empty `changed` map.
Inspect retained resource IDs/counts, command plaintext, provider and recording
capabilities, and `PRAGMA integrity_check`. A dry run opens SQLite read-only and
performs no SQL migrations. It authenticates encrypted commands and retained
capabilities before any mutation. Reports contain counts and the CSV SHA256,
not account IDs, tokens, provider URLs, command text or keys.

## Coordinated production command

Current production uses `/var/lib/silicon-browser/silicon-browser-accounts.db`
on the Browser host and `/etc/silicon-browser/runtime.env`. The candidate,
`backup.py` and `migrate_account_uuids.py` must be staged together in a private
reviewed release directory. Substitute the exact reviewed candidate directory,
final export and a **new** private backup directory below.

After the coordinator freezes Accounts writers, re-exports the final immutable
CSV and authorizes the window, stop Browser and its backup timer, retain the
previous release path and runtime environment, then execute as root:

```sh
systemctl stop silicon-browser silicon-browser-backup.timer
umask 077
set -a
. /etc/silicon-browser/runtime.env
set +a
export SB_DATABASE_URL=sqlite:///var/lib/silicon-browser/silicon-browser-accounts.db
python3 /opt/silicon-browser/uuid-candidate/migrate_account_uuids.py \
  --binary /opt/silicon-browser/uuid-candidate/silicon-browser-backend \
  --map /root/account-uuid-map.csv \
  --apply --writers-stopped \
  --backup-dir /var/lib/silicon-browser/uuid-backup-FINAL-TIMESTAMP
```

The wrapper refuses a running service, validates first, creates a consistent
SQLite backup, saves the unchanged encryption key with owner-only permissions,
records candidate/map/backup hashes, applies in one write-locked transaction,
and verifies replay. It does not restart a service. Keep writers stopped until
Accounts, Apps and Browser have all consumed the same final export. Switch the
backend executable and deploy the matching frontend under the coordinated
release, then restart Browser and its backup timer. Verify old JWTs receive
401, fresh Accounts sign-in succeeds, and existing sessions, recordings and
command logs remain accessible to their mapped owners. UUIDs are never aliases
for authenticating old subjects. Existing CDP/live capabilities expire under
the browser provider's session lifecycle; the migration does not end browsers.

## Preserved state and fences

- Rekey profile owners and explicit `@uuid` ACL entries, session initiators and
  participants, command actors, report principals/actors, recording owners and
  discovery actors. Reject an incomplete map, kind conflict, duplicate or
  pre-existing target identity. Replay requires the exact original CSV hash.
- Reseal command ciphertext with the same key and new actor AAD. Preserve exact
  plaintext, report hashes, resource/provider IDs, free text, paths and receipts.
  `commands.delivery_actor_id` retains the historical spelling for byte-identical
  frozen command exports. Historical artifact storage bindings remain unchanged.
- Preserve provider/recording ciphertext byte for byte; authenticate it during
  preflight. No provider secrets, private keys or object keys are rotated.
- Expire unfinished browser authorization attempts. Preserve claimed token
  digests and webhook receipts. Outbox jobs contain local resource IDs, not
  signed account-subject deliveries, so their bytes and retry state remain intact.
- The immutable ledger activates rejection of all legacy/noncanonical subjects
  even if introspection was cached. Old-ID sharing/filter selectors also fail.
  Late webhooks only invalidate authentication cache and cannot restore owners.
- CLI and frontend credentials must sign in again after Accounts revokes their
  old token families. Never rewrite JWT subjects. New CLI login generations
  separate connection caches and pending command reports; existing local files
  remain retained. Browser localStorage contexts are keyed by immutable UUID.

Rollback is coordinated across all three services, before reopening writers.
Stop Browser, restore the pre-cutover SQLite backup and matching encryption key
and previous executable/frontend, and restore Accounts/Apps from their matching
backups. Do not run the old backend on a migrated store or restore only one
participant. Preserve failed candidate state for diagnosis; never overwrite the
old IAM store or delete the original backup to make a retry succeed.
