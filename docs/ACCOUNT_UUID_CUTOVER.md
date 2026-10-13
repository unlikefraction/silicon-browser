# Existing Accounts UUID cutover

The production cutover below was completed on October 10, 2026. Browser 3.0.0
retains historical database rows for recovery, but removes command reporting,
command-log APIs and exports. New CLI versions discard queued legacy reports.

This procedure upgrades Browser's **existing Accounts database** from legacy
base62 account IDs to the authoritative Accounts UUIDv4 export. It does not
import or relabel IAM identities. Keep the old IAM database and key untouched.
Coordinate this window with Accounts and Silicon Apps; do not independently
apply a local test map or infer identity from a public handle.

## Candidate and backup rehearsal

Build the native backend from the reviewed commit with `backend-release.yml`.
It produces an optimized ARM64 GNU/Linux binary with a glibc 2.34 baseline,
`SHA256SUMS`, and `SOURCE_REVISION`. Build the frontend (`frontend/out`) from the same commit;
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
401, fresh Accounts sign-in succeeds, and existing sessions and recordings
remain accessible to their mapped owners. UUIDs are never aliases
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
  separate connection caches. Browser 3.0.0 discards legacy command reports
  without sending them. Browser localStorage contexts are keyed by immutable UUID.

Rollback is coordinated across all three services, before reopening writers.
Stop Browser, restore the pre-cutover SQLite backup and matching encryption key
and previous executable/frontend, and restore Accounts/Apps from their matching
backups. Do not run the old backend on a migrated store or restore only one
participant. Preserve failed candidate state for diagnosis; never overwrite the
old IAM store or delete the original backup to make a retry succeed.

## Production verification — 2026-10-10

The coordinated three-account immutable map (SHA256 `1fcac37a3e8a3dda27c80c7ef08ea7fd57bcc68f089b9cdb8ef885f0a18bcb1a`) was dry-run/applied/replayed after stopping the Browser writer and uploading a final private backup. All five retained ciphertexts verified; replay changed zero rows. The one session, two participants, two commands, one recording, two reports and two outbox rows were retained. Only declared account references and command ciphertext whose AAD contains the old actor changed. The original IAM `browser.db` was outside the consumer and was left in place.

The optimized ARM64 backend from `3188095a4465c7ad0918b2e72f6408e561a4e223` is selected at `/opt/silicon-browser/releases/20261010-uuid-3188095`. The API and backup timer reopened after Accounts and Apps completed their matching cutovers. Health, SQLite integrity and foreign keys pass; anonymous resource access is refused. The final pre-cutover backup is `s3://silicon-browser-production-234951665042-us-east-1/uuid-cutover-backups/20261010T135407Z/`.

The same-domain website now serves the tested Vite output from `/opt/silicon-browser/web-releases/3188095` through an added nginx frontend vhost on the existing host. The previous Vercel team was inaccessible to the available token, so no duplicate Vercel project was created. Before DNS switched, trusted TLS and exact index/callback/JS/CSS/icon bytes were verified using `--resolve`; after the switch, browser automation verified the public page without JavaScript errors. The original API nginx block is byte-preserved. Certificate renewal is configured and tested with HTTP webroot `/var/www/certbot`.

The six native CLI1.1.1 targets passed workflow38056729978. Updated CLI publication remains required because the old client rejects canonical UUID connection principals. A genuine post-cutover owner sign-in is still needed for the live retained-resource journey; no account is impersonated or enrolled as a substitute. Operator evidence is under `.migration/live/browser-uuid`, with the public screenshot `.migration/live/browser-production-home.png`.

## Silicon UI and genuine account verification (2026-10-10)

The public website now uses actual Silicon UI Button, motion and foundation registry source, with source hashes in `frontend/src/components/silicon-ui/registry-lock.json`. A small Solid-to-React bridge retains the existing session lifecycle, routes, reactive disabled/expanded state and native form submissions. Locked builds and45frontend tests pass. Desktop/mobile checks found no runtime errors or overflow. Production static files still use the existing AWS host; API nginx configuration and certificate renewal are preserved.

Six native runners executed the exact Browser1.1.1 combined archive from3188095 (proof workflow38060531719, verifier9d27f6f). Existing discovery returns app_id only; native reports verify the default backend from the CLI-created signed-out state, and --version verifies1.1.1. The offline Apps importer checks the existing Browser repository, backend origin and original TOS admin; it does not transfer ownership.

A genuine c:saket Carbon SLT exchanged against the production API, returned a canonical UUID and read an empty profile list. c:saket is a different account from the prior TOS owner. This proves new-account authentication/isolation; retained prior-owner data is supported by the stopped-store backup/backfill comparison above, not a claimed interactive old-owner login.
