# IAM 1.2.0 source and CLI review

Checked 2026-09-05, without creating or changing remote resources.

## Published packages and deployed service match

- Installed `iam --version`: `iam 1.2.0`.
- Both downloaded crates, `silicon-iam-cli 1.2.0` and `silicon-iam-client 1.2.0`, identify commit `ec04ec92444e02c88a39c83a286dbf47b5ded458` in `.cargo_vcs_info.json`.
- All 53 packaged Rust source files (26 client, 27 CLI) were compared byte-for-byte with that commit. No differences were found.
- Live `iam -o json system version` reports the same commit, selected API `v1`, supported APIs `["v1"]`, and server package build `0.1.0`. The server package build is separate from the client/CLI package versions.
- Live `iam -o json system health` reports `live: true` and `ready: true`.

The release commit is publicly fetchable by SHA: [1.2.0 source commit](https://github.com/teamofsilicons/silicon-iam/commit/ec04ec92444e02c88a39c83a286dbf47b5ded458).

## GitHub default-branch documentation is behind the release

At the check time, remote `HEAD` and `refs/heads/main` both point to `d9fa8745b28a5aff3cd041005fd8855ce10f73ca`. That tree still declares client and CLI version `1.1.0`. No version tags were advertised by `git ls-remote`, and GitHub's releases page lists no releases.

For this installed version, consult its bundled `iam docs` or the immutable release commit rather than assuming the `main` documentation matches the executable. The discrepancy was checked against both git refs and the [GitHub releases page](https://github.com/teamofsilicons/silicon-iam/releases).

## Real CLI discovery and migration behavior

Commands exercised: root help, `app create --help`, `app obo exchange --help`, `step-up --help`, `docs --search localhost`, `docs --search 'public HTTPS origin'`, `system version`, and `system health`.

The 1.2.0 application help and bundled documentation now explicitly distinguish the local runtime's loopback HTTP allowance from hosted ingress restrictions. Hosted testing environments isolate database state; they do not bypass ingress policy. This is a documented limitation, not evidence that the hosted loopback registration issue has been resolved. The root audit separately owns its live registration reproduction.

The CLI now requires its Unix credential home to be owned by the current user and mode `0700`. On this machine the existing default home failed that requirement, so even `system version` and `system health` initially failed with a useful permissions message. Both succeeded using a temporary private `SILICON_IAM_HOME`; no existing credentials or directory permissions were modified. Help and offline docs remained usable without opening that home. The migration procedure is documented by `iam docs storage`.

## Reusable harness review

`scripts/test_live_auth.py` already tests browser-facing SLT exchange/recovery, identity binding, organization isolation, malformed credentials, refresh/recovery, discovery, and CLI bearer non-persistence. No harness expansion was needed for this read-only provenance task.

Its scope is intentionally narrower than complete IAM product testing: it does not perform Carbon account setup, Silicon lifecycle, step-up changes, OBO consumption/replay, raw application registration, or deployed-edge policy checks. Those require explicit realistic CLI workflows and separate fixtures; success in the browser-facing harness alone must not be presented as complete IAM coverage.
