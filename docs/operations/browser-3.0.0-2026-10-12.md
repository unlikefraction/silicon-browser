# Browser 3.0.0 — telemetry removal

Release date: 2026-10-12 (IST).

Browser 3.0 removes automatic command reporting, durable report queues,
command-history access and archive exports, search/fetch audit records, and
HTTP request tracing. Browser has no active application telemetry exporter to
Space Station or another analytics service. Authentication, direct browser
connections, visual recordings, resource billing and service limits remain
available.

The API, Rust crates, npm package, GitHub release and Silicon Apps production
packages are live. The website release uses the same telemetry-free frontend
and the release documentation in this source tree.

## Contract changes

This is a major version because the CLI and Rust API remove public commands,
methods, fields and types:

- `browser session logs` and `browser session sync` are removed.
- Search and fetch no longer accept `--purpose` or a JSON `purpose` field.
- The SDK removes command reporting, command-history retrieval, report/receipt
  types, and command archive links on recordings. Local browser output still
  streams to the caller.
- Commands no longer create local activity history or report files. When the
  new CLI uses its runtime cache, it deletes only legacy report files and locks
  in that backend's credential cache directories; it never uploads them.
- The retired command-report and log routes return `410 Gone`. Their handlers
  do not extract or parse request bodies and do not authenticate callers or
  request browser-provider work.
- The website removes command-history requests and rendering, including
  command archive links. Search/fetch no longer persist audit intent or counts.

No database rows were deleted. Historical command, report, migration and
resource records remain in the database and backups; the current API and
workers do not serve or export command history. The new CLI cannot control an
older executable that a user independently continues running, so clients must
be upgraded. The deployed API rejects old reporting calls.

## Source and checks

The native runtime and published Rust package source is
[`87b3bf8c6b645f93b85ce37e5b62b92668e1d611`](https://github.com/unlikefraction/silicon-browser/commit/87b3bf8c6b645f93b85ce37e5b62b92668e1d611).
Later release documentation and local fixture corrections do not change these
native payloads.

- 271 Rust tests passed, with formatting, warning-denied Clippy and
  warning-denied Rust documentation checks.
- 47 frontend tests, TypeScript checking and the production build passed.
- 14 Python installer, packaging and backup checks passed.
- CLI regressions observed HTTP requests and confirmed that successful and
  failed local browser commands only fetched connection metadata. Valid queued
  reports from an older release were discarded without delivery. Cleanup
  preserved connection files and did not follow directory symlinks.
- Backend regressions confirmed `410` responses, no new command/report rows,
  no search/fetch audit rows, and no historical command archive exposure.
- A separate headless Chrome fixture passed hidden-input and multiple-file
  uploads, authenticated HTTP and blob downloads, concurrent transfers, failure
  preservation and page-handler restoration. Its 1,048,832-byte payload matched
  SHA-256 checks, and the fixture observed no telemetry requests or queue files.
  It used Browser 3.0.0 and the pinned native controller 0.36.0 without a cloud
  browser or real credential.
- Source CI and the native ARM64 backend build completed successfully for the
  exact source revision. The [six-platform build](https://github.com/unlikefraction/silicon-browser/actions/runs/38257189319)
  and [execution of the combined archive on all six native runners](https://github.com/unlikefraction/silicon-browser/actions/runs/38258662771)
  also passed. Each native proof ran `--version`, `--help`, `accounts --json`
  and signed-out `login status --json` from an initially empty home.

## Production API and data preservation

The API release is `20261012-2cbf9988075bea15`. Its ARM64 binary SHA-256 is
`b77a4938831ab1fa50f6947f8eba936de261283a2eb7d4b6d07ab56c2b41a3a1`.
The backend workflow artifact ZIP SHA-256 is
`d5c2137190ed55d328aab90fd4aec60a347394b7f1752dc361cc783b09aabd0b`.

Before deployment, writers were stopped and the existing Accounts database was
snapshotted. The prior runtime, API, frontend and service configuration were
retained for recovery. The verified database snapshot was 294,912 bytes with
SHA-256 `9a5b2be1ec3995f6b140cce55d8ff90407ed81e21117f05bc4dc8738d0618663`.
A freshly downloaded encrypted cloud backup passed restore checks. The
post-deployment snapshot had the same size and hash, confirming unchanged
application rows before the live smoke test.

The post-deployment audit verified the exact native binary, unchanged runtime
and encryption key, owner-only `0600` runtime permissions, database integrity,
foreign keys and healthy API. All 15 existing schema migrations remained
applied. The account cutover ledger remained one cutover and three rekeys.

Public HTTPS checks returned `200` from `/healthz` and `410` from the retired
report/log endpoints. Malformed JSON and invalid log dates also received the
same retirement response, without authentication or payload validation.

The live Browser 3.0.0 smoke test created incognito session
`01a1305f-d1a2-70a1-ab9f-523d92c343ca`, opened `https://example.com`, read
`Example Domain`, and confirmed session end. The provider billed nine browser
seconds and 333 micro-USD. Afterwards, the database contained three ended
sessions and zero nonterminal sessions. Historical `commands` and
`command_reports` counts stayed at four each, `discovery_log` stayed at zero,
and the smoke session had zero command or report rows. No browser session was
left running.

## Hosting metrics

Optional EC2 detailed monitoring was disabled and the CPU alarm without any
notification or recovery action was removed. The CloudFormation update reached
`UPDATE_COMPLETE`; the existing host remained running and the API healthy.
AWS basic provider metrics and the system-health instance-recovery alarm remain.
These are hosting functions and are not Browser command or analytics exports.

The executed template pinned the already-running AMI, preserving the instance,
network, persistent storage, IAM access and runtime secrets.

## Verified artifacts and distribution

The six-platform archive `apps-browser-3.0.0.tar.gz` is 45,109,444 bytes with
SHA-256 `87f6f93ae1cc5f0dd976a8a5777d5e7820301313669458ae8c247a2945fd100c`.
All native proofs refer to this exact archive and source revision.

| Native CLI target | Binary SHA-256 |
| --- | --- |
| Linux ARM64 | `000b9941e6cf9835727281aab7af6e83c80b38f21ab081ce75ddc81f5851b7ad` |
| Linux x86-64 | `f00fed3127d9db4410cef25efc6639ba9a37424ea88f3107277091d7130e51af` |
| macOS ARM64 | `34ae5c513d2abea3eff19930d8469fe3c45b079b0d7a6e4c21db6689bace874f` |
| macOS x86-64 | `a249da353edecf8b0095525ab7e2643fcb18ac3edd6cac98b83587de60d3749e` |
| Windows ARM64 | `72d952860dbea91b2a13a4ff0ee24e9942c1b9d65cbccd37dc81f0ddae051a99` |
| Windows x86-64 | `45aea6c6074ec28285fc817e2e594856976f126479477ad0eb85573faaa8d796` |

All three Rust crates were published as 3.0.0. Fresh public downloads matched
both the crates.io checksums and the uploaded archives. Their clean VCS source
revision, package contents and internal dependency versions were verified.

| Published crate | Archive SHA-256 |
| --- | --- |
| [silicon-browser-shared 3.0.0](https://crates.io/crates/silicon-browser-shared/3.0.0) | `93b03b3b64f075e1d475d2a044f26cd11334c2613360385706e31e608437996d` |
| [silicon-browser 3.0.0](https://crates.io/crates/silicon-browser/3.0.0) | `3e49ddd4149afe3b5fe7cdeeb3ec1c6c81619b4b1582245ebfccb6f1ef0ed920` |
| [silicon-browser-cli 3.0.0](https://crates.io/crates/silicon-browser-cli/3.0.0) | `27e35e7a27cc2bb4e924a9e55b0b5ad37b7a20787d66cb0f3422eead0ea03ca8` |

The [npm package](https://www.npmjs.com/package/silicon-browser/v/3.0.0) is public
with `latest` set to 3.0.0. Its fresh registry download exactly matched the
prepared tarball, SHA-256
`4740669f0ba78edcf3162b59346e63e895928555bc328a4629c47297864feac2`.
All four native binaries matched the platform proof hashes. A fresh isolated
public installation passed version, help, account discovery, signed-out login
and session help checks, with the retired commands absent.

The [GitHub release](https://github.com/unlikefraction/silicon-browser/releases/tag/managed-v3.0.0)
is public and latest. Its tag points to the exact native source revision above.
All 18 uploaded asset sizes and GitHub SHA-256 digests match local verified
files. Unauthenticated downloads of the source receipt, checksum manifest,
combined proof record and macOS ARM64 standalone archive matched exactly.

The primary local `browser` installation was upgraded to the verified macOS
ARM64 payload. It reports 3.0.0 and retains its working saved login. The old
binary was backed up outside PATH; unrelated pipx and Cargo executables were
not changed.

## Silicon Apps publication

The official validator accepted the six-target archive. Both Linux targets
passed hosted help, account-discovery and signed-out-login validation, and
both development package signatures and archive checksums were verified.
Development release `ccbbda28-3248-416f-8d75-10c31dc8c47f` was promoted to
production release `792ebcda-30b7-4d2c-bf1b-b491a6f4d8cc` and published.

Default production resolution returns 3.0.0 for Linux x86-64 and ARM64, with
the verified archive hash and size above and signing key `apps-2026-10`.
Publication readiness reports no errors. Linux package IDs are
`2cbf9e24-111d-4836-a5af-e6e784c9c4f7` and
`13944a6f-6250-4ecd-99b4-e32b93492c3e`, respectively.

The live hosted-worker inventory has Linux runners. Default Apps resolution
for macOS and Windows returns `target_unavailable` and does not fall back to
an older release. macOS remains available through npm and GitHub; Windows is
included in the six-target GitHub archive. Their native CI execution proofs
passed. Historical exact-version releases remain available for recovery.

## Frontend deployment

The [website](https://browser.teamofsilicons.com) release includes the frontend
without command-history controls and the final repository documentation. Its
deployment records identify the exact source revision, archive and served
index/assets by SHA-256. Verification compares every public file with its build
manifest and checks the documentation routes, frontend behavior and API health.
The root switch preserves existing hashed assets for open browser tabs and
retains the prior release for recovery.

## Evidence location and limits

Machine-readable receipts are retained locally under `target/release-3.0.0/`:
backend source/deployment/post-verification, public retirement checks, database
and live-smoke receipts, monitoring verification, six native proof files, and
crate publication/checksum verification, npm public-install checks, GitHub
asset verification, Silicon Apps publication and frontend deployment checks.
They contain no published credential material. Exact frontend build and public
verification receipts are recorded after the final source commit is selected.

These checks establish the tested contract, preserved data, deployed API and a
real navigation/end cycle. They do not establish sustained production load
capacity or the telemetry policies of third-party browsers and infrastructure.
Local operational error messages, authentication state, service usage and
resource lifecycle data remain necessary to operate Browser.
