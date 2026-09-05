# Production-readiness audit — 2026-09-05

> Archived IAM 1.1.1 audit and superseded execution architecture. Current browser commands run locally, with a 15-second production IAM cache and signed-webhook invalidation. See [current readiness](PRODUCTION_READINESS.md).

This records the earlier IAM 1.1.1 audit. The subsequent IAM 1.2 integration uses current
authorization snapshots and removes the 60-second cache delay and missing-tag limitation.
See [the latest retest](IAM_1_2_RETEST.md) and [external findings](IAM_1_2_0_EXTERNAL_BUGS.md).

`UNDERSTANDING.md`, released dependency source, and actual responses were used to audit the
implementation. Existing workspace work was preserved. No deployment or package publication
was performed.

## Changes

- IAM client updated from 1.1.0 to 1.1.1. The installed IAM CLI's own updater confirmed 1.1.1
  was current; reinstalling the same release was unnecessary.
- Added explicit, validated IAM testing-plane configuration, propagated to every auth request.
- Bounded the authentication cache to 4,096 entries and removed the full-map scan from cache
  hits. Cached authorization remains valid for at most 60 seconds and never past token expiry.
- Corrected IAM `invalid_grant` handling to return an authentication failure and accepted
  compatible negotiated v1 when IAM also advertises newer API versions.
- Fixed persistence of newly exchanged credentials together with their issuer and organization.
- Kept invocation-only environment OAT setup from overwriting saved identity/org metadata.
- Required organization scope before command streaming and rejected dot-segment resource IDs.
- Preserved comma-bearing URLs and IPv6 addresses in fetch arguments.
- Fixed normalization of a maximum-size ACL that already includes its owner.
- Preserved Unicode characters split across command-output pipe reads.
- Bounded stalled stream-event delivery, preventing indefinite session locks and occupied
  command capacity; output-delivery latency no longer inflates recorded command duration.
- Rejected reserved localhost names and additional special-purpose provider IP addresses.
- Preserved the safe runtime environment during runner version checks so PATH-installed
  executables and their interpreters can be found without inheriting application secrets.
- Resolved explicitly relative runner paths before changing to session isolation directories.
- Gave each managed session a short, private Unix socket directory; the previous nested
  paths exceeded platform limits and prevented real browser commands from starting.

## Verification

**224 workspace tests passed:** 18 client, 137 backend, 23 CLI unit, 10 CLI binary-contract,
and 36 shared tests. Groups cover validation/ACLs, client contracts, CLI state, IAM HTTP
boundaries/cache/error recovery, provider adapters, command isolation, streaming, persistence,
and session lifecycle concurrency.

- `cargo fmt --all -- --check`
- `cargo test --workspace --all-targets`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `RUSTDOCFLAGS=-Dwarnings cargo doc --workspace --no-deps`
- Shared-crate package construction and successful compilation of the packaged crate, plus
  client package file-list checks. The client cannot be
  fully packaged against crates.io until its shared dependency is published.
- Real IAM Carbon and Silicon login, Application discovery, exchange, introspection, refresh,
  idempotent recovery, wrong-org rejection, OBO catalog/exchange/verification/replay rejection.
- `scripts/test_live_auth.py`: **27/27 checks for Carbon and 27/27 for Silicon**, through the
  actual `sb` binary, local Browser backend, and remote IAM testing plane. No mocks on this auth
  path. Includes bearer non-persistence, refreshed identity, and negative authentication cases.
- Live revocation: IAM reported a newly revoked access token inactive immediately; Browser
  initially used its cached authorization and rejected it with HTTP 401 after the 60-second
  TTL. A test-only Application credential was also rejected with HTTP 401 outside its plane.
- Bounded real provider runs are documented in [LIVE_PROVIDER_RESULTS.md](LIVE_PROVIDER_RESULTS.md).

The auth test harness intentionally uses HTTP 401 for a wrong organization: IAM introspection
reports the token inactive in that scope. An initial test expectation of 403 was corrected to
match that authoritative behavior. Repeated OTP logins also hit IAM's documented rate limiting;
subsequent SLTs were minted from the already authenticated test session.

## Remaining release gates

This audit does not establish complete production readiness for every product requirement.

1. **Recording storage:** The concrete Briefcase OBO storage adapter and storage-outbox worker
   remain deferred. Provider recordings can be discovered and durably queued, but are not yet
   delivered to the initiating identity's private Briefcase.
2. **IAM upstream defects:** See [IAM_UPSTREAM_FINDINGS.md](IAM_UPSTREAM_FINDINGS.md). These
   include lost credential updates under parallel IAM commands, symlink-following credential
   writes, IPv6 URL validation, and the public edge rejecting documented loopback app URLs.
   Reports are local; the dependency update does not repair those upstream defects.
3. **Authorization projections:** Tag ACLs still fail closed because ordinary OAT introspection
   does not provide authoritative tags. Public-handle ACLs depend on learned identity projections.
   Revocation can take up to the 60-second cache TTL to become visible.
4. **Deployment boundary:** SQLite and in-process session gates target a single backend instance.
   Configure network egress restrictions: literal-host checks do not prevent arbitrary DNS
   names from resolving to private addresses or changing their resolution.
5. **Unverified release operations:** No production deployment, container smoke test, prolonged
   load/soak test, outage drill, or full publish/install from crates.io was completed here.
6. **Existing product boundaries:** The live-viewer UI is not implemented, some TinyFish flags
   are explicitly unsupported, and dangerous host-file/connection/lifecycle runner commands are
   intentionally outside the managed-session command policy. See README for those contracts.

Tests created fixtures only in the supplied IAM environment and stopped real provider sessions.
The temporary backend was shut down. The supplied environment was not erased or retired.
