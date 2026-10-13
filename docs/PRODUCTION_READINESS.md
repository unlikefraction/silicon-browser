# Production readiness

Browser 3.0.0 uses Silicon Accounts and Silicon Apps. The public API, Rust
client, CLI and website expose no organization selection. Permanent canonical
Accounts UUIDv4s identify owners and grants. Credentials persist locally;
refresh rotation is serialized and an uncertain exchange requires fresh login.

The native API and nginx website run on the existing AWS host. CLI actions and
website live viewing connect directly to the browser provider. The API manages
access, lifecycle and metadata; it never executes browser commands or receives
their output. See [deployment](DEPLOYMENT.md), [HTTP API](API.md),
[CLI](CLI.md) and [distribution](APPS.md) for the current contracts.

## Recorded evidence

| Record | What it establishes |
| --- | --- |
| [Next.js and native Silicon UI frontend](operations/frontend-nextjs-2026-10-13.md) | Pre-deployment validation: 49 unit tests, 38 browser scenarios, 178 screenshots and 53 exact registry files; linked frontend release holds deployment evidence. |
| [Browser 3.0.0 telemetry removal](operations/browser-3.0.0-2026-10-12.md) | 271 Rust/47 frontend checks, six native proofs, live API retirement and zero new telemetry rows, preserved data, and npm/Cargo/GitHub/Silicon Apps 3.0 publication. |
| [Browser 2.0.0 release](operations/browser-2.0.0-2026-10-10.md) | Organization-free contract, preserved data, 291 Rust/48 frontend checks, six native artifacts, live account/provider smoke and publication evidence. |
| [Accounts UUID cutover](ACCOUNT_UUID_CUTOVER.md) | Coordinated migration of the existing Accounts store; stopped-writer backup, authenticated ciphertext, retained resource counts, idempotent replay, integrity/foreign-key checks and preserved IAM backup. |
| [Accounts UUID cutover: website and account verification](ACCOUNT_UUID_CUTOVER.md#silicon-ui-and-genuine-account-verification-2026-10-10) | AWS static hosting, actual Silicon UI registry source, desktop/mobile checks, six native 1.1.1 packages and genuine new-Carbon sign-in/isolation. |
| [Browser 1.1.0 release](operations/browser-1.1.0-2026-10-10.md) | Dated Accounts/Apps deployment, native release, provider and publication checks for 1.1.0. |
| [Historical IAM readiness](legacy/PRODUCTION_READINESS-IAM-2026-09-06.md) | Preserved September architecture, tests and integration observations for the retired IAM/OBO implementation. |

Prior-release evidence does not prove a later candidate. The UUID cutover's
genuine `c:saket` login establishes new-account isolation; retained prior-owner
data was verified by backup comparison, not by impersonating that owner.
Record the exact source revision, artifact hashes, commands and outcomes for
each new deployment and publication.

## Release verification

Run formatting, workspace tests, warning-denied Clippy, Rust documentation,
frontend tests/build and the installer/package checks listed in the
[README](../README.md#validate). Exercise signed-out discovery, Accounts SLT
exchange, refresh persistence, logout and rejection of retired subjects. Use
local fakes for repeatable failure cases, including duplicate and interrupted
single-use exchanges.

Before switching production, verify a restorable SQLite backup with the stable
encryption key. Verify the candidate API and exact frontend bytes, HTTPS,
callback registration and signed webhook configuration. With an explicitly
authorized real account, test an allowed provider session, direct local action,
absence of command-report requests and confirmed session end. Check cross-account denial without
granting access as a substitute for owner verification.

Native distribution requires matching version/architecture receipts for all
six builds, isolated discovery checks and official Apps validation. Query live
worker availability before uploading. Verify the production resolve and its
signature, then independently verify npm/Cargo/GitHub artifacts for channels
being published. A running process or health response alone is insufficient.

## Operational boundaries

Issued CDP/live URLs are provider capabilities. API revocation or a webhook
prevents later API authorization but cannot retract an existing provider URL;
ending the remote session does. Browser does not collect or transmit command
telemetry. Legacy command-report and log routes return `410 Gone`. Resource
usage remains available for billing and service limits.

Production currently uses one API process and local SQLite WAL storage.
Additional hosts require coordinated storage, authorization-cache invalidation
and provider quotas. Existing local concurrency results do not establish
sustained cloud throughput or capacity for hundreds of paid browser sessions.
Automatic Briefcase delivery is retired; see [recording status](BRIEFCASE_INTEGRATION.md).
