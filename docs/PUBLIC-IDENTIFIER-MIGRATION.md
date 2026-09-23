# Public identifier migration

Browser uses `c:alice` for Carbons, `si:chef` for Silicons, and bare application
IDs (`browser`, `briefcase`). Organization selection remains explicit. Both
actor kinds contain a colon; kind must agree with IAM's verified actor type.
Membership IDs are `c:alice[org]` and `si:chef[org]`. Bundle IDs and release
selectors retain their separate grammars (`tos>bundle`, `browser>test@1.0.0`).

This change prepares source and an offline data migrator. Its release verification and actual production cutover are recorded separately
under `docs/operations/`. This procedure never rotates secrets.

## Verified upstream contracts

Checked 23 September 2026:

- [IAM SDK 4.0.0](https://crates.io/crates/silicon-iam-client/4.0.0) is published.
  Browser pins that exact registry release in its manifest and lockfile. Its
  archive checksum is `a1668b04888a69dc8923b4f179a6641416226a01c9d095d66dead59c68476c79`;
  bundled source provenance identifies commit
  `5743fb2452ced1be82a19846c7527b5ccc3afe1b`.
- [Honeycomb application configuration](https://docs.honeycomb.teamofsilicons.com/application-config/)
  uses `app_id` and a separate owning `org_id`.
- [Honeycomb compatibility](https://docs.honeycomb.teamofsilicons.com/compatibility/)
  preserves immutable legacy archives through catalog-authorized aliases. New
  manifests use bare app IDs. Upgrade Honeycomb before installing them.
- [Honeycomb OBO scopes](https://docs.honeycomb.teamofsilicons.com/scopes-and-obo/)
  use `obo:briefcase:briefcase.files.create` for Browser recording delivery.

Some live overview/upload examples still show older SDK versions or qualified
IDs. The current SDK and canonical identifier contract govern this migration.
Browser uses IAM's app-handle limits (1–80 characters, beginning with a lowercase
letter, then lowercase letters, digits, `_` or `-`); Carbon and Silicon handles
remain 3–30 and 3–50 characters respectively. Registered Carbon zeros remain valid.

## Prepare the cutover

1. Stop Browser API, TTL/recording workers and every test-world writer. Drain or
   reconcile pending IAM token mutations and external uploads before changing
   identity-bearing requests. Do not delete replay evidence or retry uncertain
   operations under new keys to bypass this step.
2. Back up every SQLite database using SQLite backup with writers stopped,
   including test-world stores. Preserve the encryption key, deployment
   configuration and previous binaries together. Keep database sidecars when
   making a filesystem snapshot. Rehearse on restored copies first.
3. Export IAM's restricted `iam_private.public_id_schema_map`, including
   `scope_key`, actor kind, old/new IDs and authoritative owning organization.
   Resolve global Silicon/app handle collisions in IAM first. Retain inactive
   and removed actors. Build a separate map for each Browser SQLite world,
   expanding IAM membership rows to each local membership organization where
   that identity is referenced. Never infer Silicon ownership from its new ID.
4. Change backend `IAM_APP_ID=browser` and `BRIEFCASE_APP_ID=briefcase` where
   recording delivery is configured. The actual backend variable is
   `IAM_APP_ID`, not the `SB_IAM_APP_ID` spelling in the handoff. Keep secrets,
   organization selections, endpoint IDs and provider configuration unchanged.

## Offline data conversion

The map is an authenticated operator input, not an authentication alias table:

```json
[
  {"scope_key":"","org_id":"bricks","kind":"silicon","old_id":"chef:bricks","new_id":"si:chef"},
  {"scope_key":"","org_id":"bricks","kind":"carbon","old_id":"saket","new_id":"c:saket"}
]
```

Empty `scope_key` means production. A test-world map must carry its exact IAM
environment UUID in every row. `org_id` is the local membership organization;
set `owning_org_id` separately when the Silicon belongs to another
organization (it defaults to `org_id`). Validate this owner against the IAM
export before preparing the file. For example, a Silicon `chef:bricks` that is
also a member of `tos` needs `org_id: "tos"`, `owning_org_id: "bricks"`. Include every retained identity, not only current
users. A missing old IAM UUID-to-public projection must be recovered from
operator-verified IAM data before conversion; guessing is unsafe.

Use the existing configuration and encryption key, pointing `SB_DATABASE_URL`
at the one selected database:

```sh
silicon-browser-backend --migrate-public-identifiers mapping.json
```

For an isolated testing database or a restored test-world copy:

```sh
silicon-browser-backend --migrate-public-identifiers testing-map.json --scope-key ENVIRONMENT_UUID
```

This mode starts no HTTP listener or network workers. Normal server/test-world
startup rejects unmigrated references. The migration uses one transaction,
rejects unmapped, conflicting and cross-world mappings, and can be repeated
with the same map. Keep the mapping and backup through the rollback window.

Browser-generated profile, session, recording, credential and report IDs stay
unchanged. Old `principal_id` and `membership_id` columns copied directly from
IAM are authority references: SDK 4 replaces those former IAM UUID references
with canonical actor/membership strings using verified projections. They are
not newly allocated Browser accounts.

Ownership, ACLs and participants keep their resources. Command and delivery
credential ciphertext is authenticated under its old context and re-encrypted
under the canonical context. Opaque tokens, operation keys, request hashes and
accepted receipts are retained. Frozen command artifact identities retain
the original serialized values, preserving exact artifact bytes and hashes.
Provider URLs, file contents, signed historical payloads and permanent
Briefcase receipt paths are never searched and replaced.

## Resume and verify

Complete the checks below on restored isolated stores before the production
restart. Restart upgraded IAM, Honeycomb and Browser together after their data
migrations, keeping incoming traffic closed during the final smoke checks.
Browser starts its TTL and delivery workers with the API, so reconcile all
uncertain operations before starting it. Restart clears the in-memory auth
caches. CLI sessions refresh canonical identity before using a stale
cache; this preserves the credential generation and exact queued command reports.
Old controller connection caches are discarded and resolved again. If refresh is
unavailable, drain or reconcile queued reports before a new login replaces the
credential generation. Browser tabs with old cached identities require normal
sign-in. Never rewrite opaque credentials locally. Re-enroll any stale testing selector
against its verified world without falling back to production.

Verify Carbon and Silicon login/refresh; selected organization and test-world
isolation; existing profile access and denied cross-org access; recording owner
and participant continuity; old credential decryption; exact command-report
replay; frozen artifact hashes; OBO delivery and unchanged completed receipts.
Compare row counts and resource IDs with the snapshot. Reopen incoming writes
only after these checks pass. Unit tests are not proof of a live cutover.

Previously published package archives remain immutable. This release uses the
new version 0.3.1. Preserve existing release archives and checksums. Honeycomb
uploads now require an explicit `--channel prod` or `--channel dev`; preserve
`browser>test@VERSION` for dev release selection.

## Local verification

```sh
cargo test --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
npm test --prefix frontend
npm run build --prefix frontend
python3 scripts/test_honeycomb_package.py
```

The migration regression uses populated disposable SQLite stores. It checks
canonical Carbon and Silicon access, credential decryption, exact command-report
replay, frozen artifact hashes, transaction rollback, mapping collisions and
production/testing world isolation. Paid provider workflows and production
cutover still require the coordinated maintenance-window smoke checks.

## Rollback

Before reopening writes, stop all upgraded services and restore the coordinated
IAM/Honeycomb/Browser database, key, configuration and binary snapshots. Do not
roll back just one service or run an old binary on converted data. After new
writes, stop traffic and reconcile those writes before snapshot restoration,
or fix forward.
