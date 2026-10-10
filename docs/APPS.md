# Silicon Apps distribution

Silicon Apps distributes the `browser` CLI and manages installed updates. Silicon
Accounts provides Carbon browser sign-in and Silicon short-lived tokens. The
store does not host Browser's API or website.

## Install and sign in

Install [Silicon Apps](https://developers.teamofsilicons.com/docs/apps/start/install),
then run:

```sh
silicon-apps install browser
browser --help
browser accounts --json
browser login status --json
browser setup
```

A public app can be installed without signing into the store. To use Browser's
managed profiles and sessions, a Carbon signs in on
[the Browser website](https://browser.teamofsilicons.com) through Silicon
Accounts. A Silicon gets a Browser token and exchanges it:

```sh
silicon-accounts login --app browser
browser login --slt TOKEN
```

`browser setup` prepares the pinned local controller. Store packages include that
controller beside the CLI, so setup can reuse it. The CLI does not run its own
updater. Silicon Apps follows the installed channel and normally checks every
minute; `silicon-apps update browser` checks immediately.

## Build and check packages

Keep `Cargo.toml`, workspace versions in `Cargo.lock` and `apps.yaml` in sync.
The **Silicon Apps package** GitHub Actions workflow builds and tests the CLI
natively on Linux, macOS and Windows, each on x86-64 and ARM64. It stages the
pinned browser controller beside each CLI and assembles
`apps-browser-1.1.0.tar.gz`. Windows ARM64 builds the controller from its pinned
upstream source because that upstream release has no Windows ARM64 asset.

The package's root `apps.yaml` uses this schema:

```yaml
schema_version: 1
app_id: browser
version: 1.1.0
command: browser
targets:
  linux-x86_64:
    binary: targets/linux-x86_64/browser
```

The repository manifest lists all six native targets. Each target includes the
CLI, controller and license files. The packer rejects wrong architectures,
changed native build receipts and mismatched versions, excludes state and
credentials, and preserves executable modes after Actions artifact downloads.
Archives are deterministic and enforce Apps' limits: 512 MiB compressed, 1 GiB
expanded and 20,000 entries. Every archived member is an explicitly selected
regular file; no installation hook is needed.

For local assembly from the six native workflow artifacts:

```sh
python3 scripts/test_apps_package.py
python3 scripts/package_apps.py --targets target/apps/targets
```

Before uploading, extract the archive into an empty directory and use the
official Apps validator:

```sh
mkdir -p target/apps/validate
tar -xzf target/apps-browser-1.1.0.tar.gz -C target/apps/validate
silicon-apps validate target/apps/validate
```

The native staging step checks `--version`, `--help`, `accounts --json` and
`login status --json`. Upload validation runs the last three commands signed out
in an isolated worker. `accounts --json` must contain `"app_id":"browser"`, and
signed-out login status must report the JSON boolean `"authenticated":false`.

## Hosted target availability

Apps recognizes nine platform targets, but a package can be uploaded only when
its target's validation worker is live. On October 10, 2026, the four Linux
workers were live; Windows and macOS workers were not configured. Of Browser's
six native builds, only `linux-x86_64` and `linux-aarch64` can currently be
uploaded. The remaining native builds are still produced for standalone
distribution and future Apps support. Check again before a release:

```sh
silicon-apps targets
silicon-apps capabilities
```

## Create or update the catalog entry

`deploy/apps-application.json` records the intended public catalog fields. It is
not a single API request: creation, details, access and links use their separate
Apps operations. It contains no application secret or account credentials.
The former deployment configuration and its pending protocol edits are
preserved under `deploy/legacy/`; they are not input to Silicon Apps.

Authors sign into Silicon Apps through Silicon Accounts, then inspect the
existing Browser app before creating anything. Preserve an existing app ID and
its author access:

```sh
silicon-accounts login --app silicon-apps
silicon-apps login --slt TOKEN
silicon-apps show browser
silicon-apps availability browser
```

Only when `browser` is available, create it:

```sh
silicon-apps create browser --name 'Silicon Browser'
```

Save any one-time secret securely; do not commit it. Copy the description and
links from `deploy/apps-application.json` to `description.txt` and `links.json`,
then save the corresponding fields:

```sh
silicon-apps setup browser details --description-file description.txt --tags browser,automation,productivity
silicon-apps setup browser access --visibility public
silicon-apps setup browser links links.json
silicon-apps setup browser show
```

Accounts sign-in configuration belongs to Silicon Accounts and the Browser
backend deployment, separate from the package manifest. Browser's CLI must use
the public client ID; application secrets stay on the backend.

## Upload, release and publish

Use a fresh immutable version for a new release. Version strings are strict
`x.y.z` with no prerelease suffix. Upload the archive separately for each live
target, then copy the accepted package IDs into a development release:

```sh
silicon-apps upload browser --target linux-x86_64 target/apps-browser-1.1.0.tar.gz
silicon-apps upload browser --target linux-aarch64 target/apps-browser-1.1.0.tar.gz
silicon-apps packages browser
silicon-apps release browser --version 1.1.0 --package LINUX_X64_PACKAGE_ID --package LINUX_ARM64_PACKAGE_ID
silicon-apps install 'browser>dev@1.1.0'
browser --version
browser --help
browser accounts --json
browser login status --json
```

Every release starts in development. Promote the tested development release to
production, then check readiness and publish the app:

```sh
silicon-apps promote browser DEVELOPMENT_RELEASE_ID --version 1.1.0
silicon-apps readiness browser
silicon-apps publish browser
```

Publishing requires a 200 to 600 character description and an accepted package
in a release. It takes effect immediately when ready; there is no manual review.
The default `silicon-apps install browser` selects production. An app may publish
with development releases only, but users must then choose `browser>dev`.
Release versions and package bytes cannot be replaced. Keep a mutation's
idempotency key and exact request when recovering an uncertain response.

This migration prepares source and packaging only. It does not claim a new
catalog publication, a production Accounts configuration or hosted validation
for an unavailable target. Previous rollout notes are preserved in
[the archived distribution notes](legacy/HONEYCOMB.md).

References: [manifest](https://developers.teamofsilicons.com/docs/apps/reference/manifest),
[publishing](https://developers.teamofsilicons.com/docs/apps/start/publish),
[updates](https://developers.teamofsilicons.com/docs/apps/learn/releases-and-updates).
