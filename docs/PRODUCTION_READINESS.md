# Architecture and readiness evidence — 2026-09-06

The current architecture is a local usability wrapper: `sb` runs its native controller on the caller's machine and connects directly to the remote browser. AWS supplies IAM-backed access, profile/session management, sensitive connection capabilities, usage and cooperative command-log storage. It never executes browser commands or relays their output. Vercel hosts a minimal SolidJS/TypeScript frontend whose live iframe connects directly to the remote viewer.

The native AWS daemon also performs completed-recording delivery and shared-key search/fetch scheduling. These separate integrations remain server-side. Neither Docker nor an AWS browser-controller installation is required. See [deployment layout](DEPLOYMENT.md) for configuration and traffic paths.

## Current source contracts

| Area | Implemented behavior |
| --- | --- |
| IAM access | Explicit organization scope, immutable principal/membership bindings and disclosed-tag ACL union. Production snapshots are bounded to 15 seconds/token expiry and invalidated by verified IAM webhooks. No secret means no webhook route. |
| Local browser control | Rust `LocalController` and CLI use the direct CDP capability. Connection renewal requires the current profile ACL, raw active state and unexpired TTL. The CLI partitions auth by backend URL and locally isolates controllers by backend/org/principal/session. |
| Profiles and sessions | Fixed profile location and stable public fingerprint, owner/default ACL, retirement, one active session per profile, provider correlation/reconciliation, expiry and confirmed stop before profile reuse. |
| Native local setup | Pinned native controller installed with integrity checks or reused locally. No local Chromium, Node/npm installation or AWS controller dependency. File paths belong to the caller. |
| Command reports | Stable UUID, command/flags, client timestamps and result metadata only; stdout/stderr fields are rejected. Identical retries acknowledge the original receipt. Server never executes reports. Silicon command text is encrypted; Carbon command history is omitted. |
| Recording archive | Native provider MP4 plus reported Silicon command JSONL, delivered through independent durable receipts. Command archive creation fences additional reports before reading pages; exact retries remain acknowledgeable afterward. |
| Delivery authorization | A second fresh Browser SLT enrolls the backend's separate IAM family. Encrypted credential storage, serialized rotation, durable revocation and original principal/membership bindings; CLI refresh credentials are never borrowed. |
| Failure recovery | Fenced leases, delayed native sources, bounded retries, stable proof-bound bytes and eligible explicit retry. Completed receipts survive partial failures. Legacy unbound jobs require verified ownership. |
| Recording paths and hiding | Verified Briefcase receipts determine paths. Pending paths are empty. Browser hiding cancels new delivery and hides links; OBO cannot delete files and Browser promises no remote purge schedule. |
| Frontend | SolidJS/TypeScript/Vite, IAM's visual language, Vercel assets, direct AWS API requests, exact-origin CORS, in-memory auth, IAM popup/callback nonce checks, direct live iframe. |
| Storage and capacity | SQLite WAL, eight connections, immediate write transactions where reads precede writes, batched session/usage listings, bounded auth cache and shared lookup work. No per-user browser process on AWS. |
| Search/fetch | Backend-routed provider key pool, fair scheduling, key-specific quotas, batching and explicit rejection of unsupported flags. |

## Verification records

The backend architecture correction passed a **199-test backend suite** and all-target Clippy with warnings denied. The subsequent removal of stdout/stderr from reports passed its seven control tests, the complete lifecycle wire test, and Clippy. These are component checks, not an aggregate count for every later workspace change.

The control suite includes **500 simultaneous clients issuing 1,000 authenticated connection/report requests** against a real temporary file-backed SQLite WAL database. Its recorded local run measured p50 **431 ms**, p95 **509 ms**, maximum **565 ms**, with no provider operations. IAM and browser adapters were fakes. This tests control-plane contention and idempotent sequence allocation; it does not establish cloud network latency, IAM production capacity, sustained throughput or a quota for 500 paid browsers.

Regression coverage includes current ACL renewal after historical participation, ending/expired capability denial, secret/no-store handling, rejection of old execution requests and output-bearing telemetry, report identity conflicts, archive fencing and timestamp bounds. IAM tests cover concurrent cache misses, invalidation races and signed webhook verification/deduplication.

Earlier real Carbon/Silicon sessions proved automatic native recordings, recipient-side Briefcase downloads and exact hashes, full MP4 decoding, and interactive Carbon live handoff. Those runs used the previous controller/frontend implementation; they remain evidence for the native recording and delivery integration, not proof of the replacement local-controller or SolidJS flows. [Historical automatic live retest](AUTOMATIC_RECORDING_LIVE_RETEST.md).

The real [300 MiB upload](BRIEFCASE_300_MIB_CLI_RETEST.md) exercised upload, full download, range boundaries and private-owner access with about 18 MiB peak client memory inside the 2 GiB sandbox. The [slow-transfer test](BRIEFCASE_SLOW_UPLOAD_RETEST.md) independently exposed the proof deadline.

The earlier 306-Rust/19-frontend totals belong to the September 5 architecture and are preserved in its dated report. Final implementation checks passed 318 Rust tests and 18 frontend tests, along with formatting, warning-denied Clippy and documentation builds. Real production local-CDP commands, verified file-byte transfers, SolidJS sign-in/live use, native recording delivery, domain TLS and actual IAM webhook deliveries are recorded in [the deployment verification](PRODUCTION_DEPLOYMENT_2026_09_06.md).

## Security and operational boundaries

- An issued CDP/live URL is a direct provider capability. IAM/webhook invalidation controls later API access and renewal; it cannot revoke that already-issued URL. Closing/expiring the remote browser ends it. The CLI's connection cache is at most 60 seconds; it does not extend provider session expiry.
- Logs are cooperative, ordered by server receipt and timestamped by clients. Separate machines can act concurrently. Direct actions, client crashes before report persistence and offline reports after archive closure can be absent. `sb session sync` retries delivery only; `409 report_window_closed` preserves an unsent report locally.
- Connection replacement and managed lifecycle commands remain reserved to `sb`; local file operations are supported by the controller. The literal all-command superset is therefore not claimed. [Execution scope](COMMAND_EXECUTION_GAPS.md).
- Browser Use's explicit-null incognito requests still produced nonzero reported proxy traffic/cost. Actual proxy-free routing remains unverified. Aggregate telemetry is preserved as unclassified rather than invented ingress/egress values. [Provider finding](BROWSER_PROVIDER_FINDINGS.md).
- Briefcase upload bodies must arrive within the proof's at-most-60-second lifetime. The 512 MiB staging default and configurable cap do not extend it. Lost success responses can produce an identical extra file version; exactly-once publication is not claimed.
- IAM normal family revocation also invalidated sibling OATs for the same parent login/application in the recorded test. Independent ORTs remain refreshable; bounded recovery uses each owner's credential and only repeats API mutations after an explicit pre-handler rejection. [IAM finding](IAM_1_2_2_EXTERNAL_BUGS.md).
- The deployment target is one native AWS backend process with local persistent SQLite. Additional hosts need coordinated data access, search quotas and auth-cache invalidation. Production IAM/webhook reachability, provider capacity, TLS/DNS, process restart, disk headroom, backup/restore and sustained load need operational evidence; a process health response alone does not prove them.

External observations stay in repository files until deliberately reported upstream. The local Rust 1.98.1 incremental compiler ICE was avoided with `CARGO_INCREMENTAL=0`; it is a toolchain observation, not a deployed runtime failure. This document describes source and recorded checks, and does not substitute for a dated deployment/publication record.

Deployed domain, native service, real local-controller/dashboard and backup recovery evidence is recorded separately in [the production deployment verification](PRODUCTION_DEPLOYMENT_2026_09_06.md).
