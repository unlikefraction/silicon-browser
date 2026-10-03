# Honeycomb distribution

Honeycomb distributes the `browser` CLI as `browser`. It does not host the API or
website: the existing AWS backend, Vercel frontend, SQLite storage, IAM login,
and Briefcase recording delivery retain their current architecture.

## Changes from the previous release

- One `honeycomb.yaml` maps `browser` to six native builds: Linux, macOS and Windows,
  each on x86-64 and ARM64.
- Each payload includes the pinned browser controller beside `browser`. Setup and
  browser commands reuse that controller; no package installation hook is needed.
  The explicit `SB_CONTROLLER_BIN` override still takes precedence at runtime.
- The Windows CLI opens directory handles correctly and flushes file writes
  without attempting Unix directory synchronization. Credential directories are
  private on both platforms.
- The Windows ARM64 controller is built from pinned upstream source because
  upstream 0.36.0 provides only a Windows x86-64 binary. No architecture is
  represented by a shell wrapper or another architecture's executable.

Release 0.2.4 renames the public command from `sb` to `browser`. Existing
`SB_*` variables, authentication, state paths and the private controller remain
compatible. The curl installer and Honeycomb use the same native CLI builds.

## Current release — 0.4.2

Browser 0.4.2 is public in Honeycomb's production and development channels at
application revision 4. It uses the accepted four-root Briefcase recording
contract: reserve, commit, status and entry listing. Recording access remains a
separate IAM approval bound to the selected account and organization.

The 45,786,164-byte archive has SHA-256
`2551aefed906fc9f46d7e83be226af15157fd7fa50d20762196073f3386d4a5e`.
[Native CI](https://github.com/unlikefraction/silicon-browser/actions/runs/37145497699)
built and tested all six targets from
`74e72a307a72eb95d3bffcd3757f54697d652c16`. The identical archive bytes were
accepted first in development and then in production. Both channels report
public visibility, configuration revision 4, published approval, and no pending
permission approval.

Anonymous installations in fresh homes on macOS ARM64 passed in both channels.
The CLI reports `browser 0.4.2`, its bundled controller reports
`agent-browser 0.36.0`, CLI help works, and both installed executable hashes match
the archive. These checks do not require IAM login or create a paid session.

```sh
honeycomb install 'browser@0.4.2'
# Optional development-channel install with a distinct command:
honeycomb install 'browser>test@0.4.2' --alias browser=browser-dev
```

The current curl installer pins the 0.4.2 native assets and hashes. Older release
tags and published archive bytes remain immutable.

## Previous release — 0.4.1 (2026-10-03 UTC)

Browser 0.4.1 was published at application revision 3 with the earlier three-root
recording scope. Its archive SHA-256 remains
`c7b92980fb09ada4538ded03d6222a0346455c2d4cb7c4831ab9afe4363df86a`.
[Its native CI](https://github.com/unlikefraction/silicon-browser/actions/runs/37142254120)
built all six targets from `6da8a79f2307483d28091869221f342b817c342d`.
The Rust client, shared contracts and CLI crates were also published as 0.4.1.
Keep those artifacts unchanged when releasing a newer version.

## Historical rollout status — 2026-09-17

`browser` is public and active, with its IAM webhook approved. The original
0.2.3 publication request `17dd5de1-49f7-4a3e-b45e-9a041ec5f3fa` is published at
configuration revision 1. Its installation and IAM login were verified.

The command rename is packaged as the new immutable 0.2.4 release. All six
[native builds and archive checks](https://github.com/unlikefraction/silicon-browser/actions/runs/35155065017)
and the [full CI suite](https://github.com/unlikefraction/silicon-browser/actions/runs/35155065018)
passed. The Rust packages are published as 0.2.4. See
[the bug notes](HONEYCOMB-BUGS.md) for historical platform findings and resolution.

Release 0.2.4 was accepted as public and installed successfully without any
Honeycomb login. Its archive SHA-256 is
`fe207a514e74cdd6e0e9bf6232b48fa099b9093dc321248b3a9e0c4c27eb0f69`.
The installed command reports `browser 0.2.4`; the standalone installer and
crates.io installation provide the same command. Native GitHub downloads are in
[managed-v0.2.4](https://github.com/unlikefraction/silicon-browser/releases/tag/managed-v0.2.4).

Only the catalog description's command name changed in desired configuration
revision 2. Honeycomb created request `c6b12204-fcf8-418e-af1e-f2fa81d85ff6`, which
awaits validator approval. The accepted revision and 0.2.4 archive remain public
while that wording change is reviewed.


## Build a release

Browser 0.3.0 introduced canonical identities; its published artifacts remain
immutable. The current release is 0.4.2, with IAM 5 account-bound sessions and recoverable recording delivery. Follow the [migration procedure](PUBLIC-IDENTIFIER-MIGRATION.md)
for any retained store that has not completed the identifier cutover.

Keep `Cargo.toml`, workspace versions in `Cargo.lock`, and `honeycomb.yaml` in
sync. The **Honeycomb package** GitHub Actions workflow builds and checks every
target on its native operating system and architecture. It runs when CLI source,
package inputs or the workflow change, and can also be dispatched manually. Download its final
`honeycomb-browser-0.4.2.tar.gz` artifact.

For local assembly of the six native workflow artifacts:

```sh
python3 scripts/test_honeycomb_package.py
python3 scripts/package_honeycomb.py --targets target/honeycomb/targets
honeycomb validate target/honeycomb-browser-0.4.2.tar.gz
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
honeycomb apps get 'browser' --json
honeycomb --idempotency-key silicon-browser-honeycomb-0.4.2-upload-YYYYMMDD \
  releases upload 'browser' target/honeycomb-browser-0.4.2.tar.gz --channel prod --revision REVISION
honeycomb releases list 'browser'
honeycomb install 'browser' --version 0.4.2
browser --version
browser --help
browser setup
```

Use the current application revision, not the release version. Reuse the exact
key, revision and archive after an uncertain upload response. If `browser` already
exists on PATH, use `--alias browser=browser-honeycomb` when installing and run that alias.
When upgrading an installation that had an `sb` alias, pass
`--alias browser=browser` to replace the old command mapping.

For an application that is still private, verify installation, request publication
with the latest revision, and inspect its status. Uploading a release does not
itself make a private app public. New releases of an already public application
receive public archive access during upload.
Honeycomb reviewers control public approval. Webhook activation separately needs
IAM's fresh verification for `application.webhook.approve` and the exact pending
endpoint; a stored login is insufficient.

Browser requests identity, membership and tags for authorization, plus
`briefcase.entries.list`, `briefcase.uploads.reserve`, `briefcase.uploads.commit`,
and `briefcase.uploads.status` for separately approved recording delivery.
Status is a separate accepted provider endpoint; reserve and commit do not grant
it implicitly. Update Browser's complete application configuration at its current
revision and obtain new feature consent when expanding from the old three roots.
The 2026-10-03 UTC update and normal review accepted Browser revision 4 with all
four roots, effective revision 4 and IAM revision 46. Publication request
`7f9c9101-6d03-4d6c-810d-590ce505d2b2` is published and activation
`4601391c-cf94-4923-86c3-f63c9593eeaf` is accepted. Release 0.4.2 was subsequently
accepted under development operation `51f91c31-208d-49b3-8668-edb8131c8dbb` and
production operation `7c8f0f0b-f10f-46fc-99f2-5f99eada8624`. Re-read current
revisions before later uploads or edits.
Honeycomb itself needs its own Briefcase upload/download/publication grants; a
missing Honeycomb grant must be repaired in that platform, not added to Browser.

References: [package format](https://docs.honeycomb.teamofsilicons.com/package-format/),
[upload guide](https://docs.honeycomb.teamofsilicons.com/upload-an-app/),
[publication](https://docs.honeycomb.teamofsilicons.com/publication/).
