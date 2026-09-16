# Honeycomb distribution

Honeycomb distributes the `sb` CLI as `tos>browser`. It does not host the API or
website: the existing AWS backend, Vercel frontend, SQLite storage, IAM login,
and Briefcase recording delivery retain their current architecture.

## Changes from the previous release

- One `honeycomb.yaml` maps `sb` to six native builds: Linux, macOS and Windows,
  each on x86-64 and ARM64.
- Each payload includes the pinned browser controller beside `sb`. Setup and
  browser commands reuse that controller; no package installation hook is needed.
  The explicit `SB_CONTROLLER_BIN` override still takes precedence at runtime.
- The Windows CLI opens directory handles correctly and flushes file writes
  without attempting Unix directory synchronization. Credential directories are
  private on both platforms.
- The Windows ARM64 controller is built from pinned upstream source because
  upstream 0.36.0 provides only a Windows x86-64 binary. No architecture is
  represented by a shell wrapper or another architecture's executable.

The ordinary curl installer remains pinned to its existing four-platform 0.2.2
release. Honeycomb release 0.2.3 adds the portable package without changing that
installation channel or publishing new crates/npm packages.

## Current rollout status — 2026-09-16

`tos>browser` is registered in TOS and active, with its IAM webhook approved.
Its production backend is using the new app credential; live authentication
checks passed. The application is still **private**, with no uploaded release.

All six native builds and package checks passed in the
[Honeycomb package workflow](https://github.com/unlikefraction/silicon-browser/actions/runs/35065262468).
The [existing CI suite](https://github.com/unlikefraction/silicon-browser/actions/runs/35065262331)
also passed. Honeycomb validated the assembled 0.2.3 archive successfully.

Upload is blocked by IAM's proof-lifetime database constraint. Public release
also needs Honeycomb's unfinished production review/activation integration.
See [the bug notes](HONEYCOMB-BUGS.md) for evidence, repair requirements and exact
retry details. Publication and installation have not yet been verified.

## Build a release

Keep `Cargo.toml`, workspace versions in `Cargo.lock`, and `honeycomb.yaml` in
sync. The **Honeycomb package** GitHub Actions workflow builds and checks every
target on its native operating system and architecture. It runs when CLI source,
package inputs or the workflow change, and can also be dispatched manually. Download its final
`honeycomb-browser-0.2.3.tar.gz` artifact.

For local assembly of the six native workflow artifacts:

```sh
python3 scripts/test_honeycomb_package.py
python3 scripts/package_honeycomb.py --targets target/honeycomb/targets
honeycomb validate target/honeycomb-browser-0.2.3.tar.gz
```

The packer checks native executable headers and hashes against the versions
checked on each host. It includes only the manifest, CLI/controller executables,
and their licenses. Credentials, source trees and build receipts are excluded.
Non-Windows executable modes are restored after downloading Actions artifacts.

## Register and publish

`deploy/honeycomb-application.json` contains public configuration only. Creation
requires adding `webhook_secret` to a protected copy, using the same value as the
backend's `IAM_WEBHOOK_SECRET`. Save the returned one-time application secret in
AWS Secrets Manager as `IAM_APP_SECRET`; retain `SB_ENCRYPTION_KEY` and provider
credentials. Match `IAM_WEBHOOK_KEY_VERSION` to IAM's accepted signing version.

Use the existing application for subsequent releases. Do not recreate it or
rotate its credentials as part of an ordinary package update.

```sh
honeycomb apps get 'tos>browser' --json
honeycomb --idempotency-key silicon-browser-honeycomb-0.2.3-upload-20260916 \
  releases upload 'tos>browser' target/honeycomb-browser-0.2.3.tar.gz --revision REVISION
honeycomb releases list 'tos>browser'
honeycomb install 'tos>browser' --version 0.2.3
sb --version
sb --help
sb setup
```

Use the current application revision, not the release version. Reuse the exact
key, revision and archive after an uncertain upload response. If `sb` already
exists on PATH, use `--alias sb=sb-honeycomb` when installing and run that alias.

After verifying private installation, request publication with the latest revision
and inspect its status. Uploading a release does not itself make the app public.
Honeycomb reviewers control public approval. Webhook activation separately needs
IAM's fresh verification for `application.webhook.approve` and the exact pending
endpoint; a stored login is insufficient.

Browser requests identity, membership and tags for authorization, plus
`obo:tos>briefcase:briefcase.files.create` for user-authorized recording delivery.
Honeycomb itself needs its own Briefcase upload/download/publication grants; a
missing Honeycomb grant must be repaired in that platform, not added to Browser.

References: [package format](https://docs.honeycomb.teamofsilicons.com/package-format/),
[upload guide](https://docs.honeycomb.teamofsilicons.com/upload-an-app/),
[publication](https://docs.honeycomb.teamofsilicons.com/publication/).
