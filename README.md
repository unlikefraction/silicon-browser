# Silicon Browser

Silicon Browser provides managed remote browser profiles, expiring sessions,
shared access, live handoff, recordings, command history and usage accounting.
The `browser` CLI and Rust client execute browser actions locally over the
remote browser's direct connection. A Rust API manages access and metadata;
a SolidJS website provides Carbon sign-in and live viewing.

Version 1.1.0 uses **Silicon Apps** for installation and updates, and **Silicon
Accounts** for sign-in. The code migration is prepared here; this checkout has
not been published or deployed.

## Install and sign in

Install [Silicon Apps](https://developers.teamofsilicons.com/docs/apps/start/install.md),
then install Browser after its release is published:

```sh
silicon-apps install browser
browser --help
browser accounts --json
browser login status --json
```

A Silicon requests a short-lived token for Browser and exchanges it once:

```sh
silicon-accounts login --app browser
browser login --slt TOKEN
browser setup
browser profile ls
```

A Carbon signs in on the Browser website using the hosted Silicon Accounts flow.
Browser never receives an Accounts password or a Silicon's STK. Accounts
returns JWT access tokens and rotating `sar_` refresh tokens. A refresh with an
uncertain outcome requires a fresh sign-in; Browser does not resubmit the old
token. The CLI keeps private credentials under
`${SILICON_HOME:-$HOME}/.silicon-browser`, partitioned by backend URL.

Silicon Apps keeps installed packages up to date. Browser packages bundle the
pinned native controller; `browser setup` checks it. See [Apps packaging and
release](docs/APPS.md) for native targets, validation and publication.

## Use a browser

```sh
browser proxy ls
browser profile new --name Research --location US
browser session new PROFILE_ID --name Research --description "Vendor comparison" --ttl 30m
browser run SESSION_ID "open https://example.com"
browser run SESSION_ID "snapshot -i"
browser run SESSION_ID "screenshot ./research.png"
browser session end SESSION_ID --note done
```

Profiles keep cookies and a fixed proxy location. Incognito sessions have no
profile. Sessions expose live links for authenticated viewers:

```sh
browser session live SESSION_ID
browser recording ls
browser usage limits
browser session logs SESSION_ID
```

Ownership uses the permanent, case-sensitive Accounts UUID. Public `c:` and
`si:` handles can change and are resolved when sharing. Accounts has no
organization or tag membership; Browser controls its own profile access inside
one application workspace. A custodian receives no implicit access to a
Silicon's Browser data.

For read-heavy work, no profile or browser session is needed:

```sh
browser search "browser automation" --purpose "Find primary sources"
browser fetch https://example.com,https://example.org --purpose "Read relevant claims"
```

The CLI streams action output locally and submits only cooperative command
metadata. Failed report delivery never repeats a browser action; use
`browser session sync SESSION_ID` to retry queued reports. Files and recordings
created by local controller commands write to the caller's machine.

## Run from source

```sh
cargo build -p silicon-browser-cli
./target/debug/browser accounts --json
./target/debug/browser login --slt TOKEN
./target/debug/browser setup
```

Copy [.env.example](.env.example) into a private runtime environment and set the
Accounts app secret, provider key and stable encryption key. Register
`https://browser.teamofsilicons.com/auth/callback` as Browser's Accounts redirect
URI, and configure the website's origin in Accounts. For local development use
your exact local frontend origin and its `/auth/callback` URI.

```sh
cargo run -p silicon-browser-backend
pnpm --dir frontend install --frozen-lockfile
pnpm --dir frontend build
node frontend/dev.mjs
```

Set `SB_PUBLIC_BACKEND_URL` while building the frontend to select a local API;
set the backend's `SB_ORIGIN` to the exact frontend origin. See
[deployment](docs/DEPLOYMENT.md), [frontend setup](frontend/README.md), and the
[CLI reference](docs/CLI.md).

The default database is `silicon-browser-accounts.db`. Keep the previous IAM
database, encryption key and recording receipts as a separate backup. Startup
refuses to reinterpret existing IAM resources as Accounts ownership. Account
migration requires a verified old-principal-to-Accounts-UUID mapping; handles
alone do not establish continuity. The former IAM testing worlds and Briefcase
OBO authorization flow are retired. See [recording integration status](docs/BRIEFCASE_INTEGRATION.md).

## Validate

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo doc --workspace --no-deps
pnpm --dir frontend test
pnpm --dir frontend build
python3 scripts/test_apps_package.py
python3 scripts/test_installer.py
```

Tests use local fakes and do not consume provider capacity. Live Accounts
registration, provider credentials, production data migration and store
publication require their own deployment verification. Historical integration
notes are retained under [docs/legacy](docs/legacy).
