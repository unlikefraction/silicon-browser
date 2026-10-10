# Browser 2.0.0 — Accounts without organization selection

Release date: 2026-10-10 (UTC).

Browser 2.0 removes organization selection from the public SDK, CLI, website and
current authentication responses. Canonical Accounts UUIDv4s own resources.
Saved CLI and website logins survive subsequent commands and reloads through
serialized refresh rotation. The website uses the actual Silicon UI registry
components and includes their MIT notice. Historical identity migrations and
backups remain available for recovery.

## Source and checks

The native build source is `2f1af1488eea2a64ab2f5886a341a864dc655dd3`.
`b6bc760940424d44c2f3ddcf56e154b38d31028c` adds the single-Accounts-database
backup helper and final frontend verification notes; it does not change the
native Rust payload. `55ef5d4` commits the four matching npm binaries.

- 291 Rust tests passed: shared 39, backend 164 plus 2 integration checks,
  SDK 29, CLI library 30 and CLI binary 27.
- Formatting, warning-denied Clippy and Rust documentation checks passed.
- 48 frontend tests, TypeScript checking and the production build passed.
- Apps packaging (3), installer (5), backup (5), and UUID backup replay checks passed.
- [Source CI](https://github.com/unlikefraction/silicon-browser/actions/runs/38073141719)
  and the [backend build](https://github.com/unlikefraction/silicon-browser/actions/runs/38073145160) passed.
- [Six native builds](https://github.com/unlikefraction/silicon-browser/actions/runs/38073141712)
  and [verification of the exact combined archive on all six native runners](https://github.com/unlikefraction/silicon-browser/actions/runs/38073860311) passed.
  The proof workflow runs `--version`, `--help`, `accounts --json` and signed-out
  `login status --json` against the extracted release payloads.

## Production deployment

The API release is `20261010-621d00c1e520cd4d`. Its binary SHA-256 is
`0610363dff15e2fe678a31a20bea8f51360bdaae41498328c6c4eb36c07214eb`.
The first 2.0 frontend artifact was built from `b6bc760940424d44c2f3ddcf56e154b38d31028c`.
Its archive SHA-256 is `4ca63a2172661fdbc4a211f6aeea942686eeb0d1668cb2f98a5df42183eb5ae6`;
the served index SHA-256 is `944cd7031275780bf4d0a98382bedc588d628a76946fd718c61e0870df47310c`.

Before deployment, the current Accounts database was snapshotted while writers
were stopped. Both a local copy and a freshly downloaded encrypted cloud backup
passed integrity, foreign-key and schema checks. The cloud copy was 286,720
bytes with SHA-256 `c40dde8576510e9c4b8221a954ba9e7f00da8fcf96a5528c989d12c55f6860dc`.
The previous API, runtime, service units, frontend and nginx configuration were
archived together. No identity cutover was repeated.

The post-deployment audit verified exact runtime/Secrets Manager equality,
unchanged encryption key, owner-only runtime permissions, healthy API/nginx,
and an active successful backup timer. All saved resource rows were preserved.
Schema migrations 1–15 and database integrity/foreign-key checks passed. The
UUID cutover ledger remained one cutover and three ciphertext rekeys.

The first frontend switch automatically restored the previous vhost because
an immediate request after nginx reload still returned the old index. A bounded
readiness check then verified the new index, its two entry assets and all 11
retained old assets. Only the frontend root directive changed; API routing,
TLS and security configuration were preserved. Public home, callback and live
links, documentation and the Silicon UI license returned successfully. A real
browser showed the welcome screen and correctly expanded/collapsed the Silicon
SLT form; no user credentials were entered during that visual check.

## Live account and browser checks

A fresh SLT was exchanged as `si:tos`, using canonical account UUID
`0080c488-a9e9-4c22-aa91-1c7a639f1d7b`. The 2.0 CLI returned an authenticated,
organization-free identity in a separate process with no token environment
variable. Its saved state has mode `0600`, contains a refresh token, and has no
legacy organization field. The default local CLI and controller were updated;
the previous binaries were retained privately for recovery.

The existing migrated session history remained accessible. A new incognito
session (`01a126f6-c339-7fc3-9129-10d12910fa9b`) opened `https://example.com`,
returned the title `Example Domain`, and ended successfully after nine browser
seconds. The provider-reported charge was 333 micro-USD. The database then had
two ended sessions and zero nonterminal sessions. No session was left running.

## Distribution

The combined six-platform archive SHA-256 is
`a2e2ea79c67238f8868cd1527e453e1aae151c2507b911d98db115b20ff73cf9`.
The npm tarball SHA-256 is
`c53a7ef8782888b81a06a6504df1e98d54dcb3a795a39642813aa5bf9ea50b52`.

The npm `latest` version is 2.0.0; a downloaded registry tarball is byte-for-byte
identical to the prepared artifact. A fresh public npm install passed version,
help and isolated signed-out discovery checks. All three Rust crates (`silicon-browser-shared`,
`silicon-browser`, and `silicon-browser-cli`) have published 2.0.0 versions. Their
public checksums and every packaged source file were verified against the
release source.

Silicon Apps reports `published: true` with readiness `ready: true` and no
errors. The production release is `6239e746-b061-41b3-ac4b-feae13cd77ac`,
promoted from development release `b4da050d-4ccf-43c9-96db-437d9b7fbb49`,
and was published as `si:tos`. Both Linux packages passed hosted validation.

[GitHub managed-v2.0.0](https://github.com/unlikefraction/silicon-browser/releases/tag/managed-v2.0.0)
is published with 18 assets, including the complete six-platform archive,
standalone macOS/Linux archives, checksums, source revision and six native
proof receipts. The tag points to the exact native build source `2f1af14`.

The official Apps service currently configures hosted validation only for Linux.
This release therefore uses Apps for Linux, npm for macOS/Linux and the GitHub
release for all six native targets. macOS/Windows payloads have native CI proof,
but that proof is not a supported substitute for the Apps service's hosted
validation route. No old binary is relabeled as 2.0.0.

## Scope of evidence

These checks establish the released contract, artifact provenance, deployment,
saved login, data preservation and a real provider navigation/end cycle. They
do not establish sustained production load capacity. Accounts can revoke or
expire a refresh family; an uncertain one-use exchange requires a new sign-in.
Old clients receive the fixed Browser organization compatibility field only
when they omit `contract_version: 2`; current clients expose no organization
selection. The historical database partition is retained internally so encrypted
resources and old local command queues remain readable.
