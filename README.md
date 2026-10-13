# Silicon Browser

Silicon Browser provides managed remote browser profiles, expiring sessions,
shared access, live handoff, recordings and usage accounting.
The `browser` CLI and Rust client execute browser actions locally over the
remote browser's direct connection. A Rust API manages access and metadata;
a Next.js website built with Silicon UI provides Carbon sign-in and live viewing.

Version 3.0.0 uses **Silicon Apps** for installation and updates, and **Silicon
Accounts** for sign-in. The hosted website is available at
[browser.teamofsilicons.com](https://browser.teamofsilicons.com).

## Install and sign in

Install [Silicon Apps](https://developers.teamofsilicons.com/docs/apps/start/install.md),
then install Browser on supported Linux targets:

```sh
silicon-apps install browser
browser --help
browser accounts --json
browser login status --json
```

For macOS and Linux, the npm distribution is also available with
`npm install -g silicon-browser@3.0.0`. Apps currently has Linux validation workers;
see [distribution availability](docs/APPS.md#hosted-target-availability).

A Silicon requests a short-lived token for Browser and exchanges it once:

```sh
silicon-accounts login --app browser
browser login --slt TOKEN
browser setup
browser profile ls
```

A Carbon signs in on the Browser website using the hosted Silicon Accounts flow.
Browser never receives an Accounts password or a Silicon's STK. Accounts
returns JWT access tokens and rotating `sar_` refresh tokens. Login is remembered
across CLI invocations and website reloads; `browser logout` clears the local CLI
login. A refresh with an
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
```

Ownership uses the permanent, canonical Accounts UUIDv4, such as
`550e8400-e29b-41d4-a716-446655440000`. Public `c:` and
`si:` handles can change and are resolved when sharing. Accounts has no
organization or tag membership; Browser controls its own profile access inside
one application. There is no organization selection. A custodian receives no implicit access to a
Silicon's Browser data.

For read-heavy work, no profile or browser session is needed:

```sh
browser search "browser automation"
browser fetch https://example.com,https://example.org
```

The CLI executes commands and streams their output locally. It does not collect,
queue or send command telemetry. No telemetry is sent to Space Station or any
other analytics service. Files and recordings created by local controller
commands write to the caller's machine.

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
SB_ORIGIN=http://127.0.0.1:8092 PORT=8091 cargo run -p silicon-browser-backend
npx --yes pnpm@10.33.0 --dir frontend install --frozen-lockfile
SB_PUBLIC_BACKEND_URL=http://127.0.0.1:8091 npx --yes pnpm@10.33.0 --dir frontend build
npx --yes pnpm@10.33.0 --dir frontend preview
```

The website is a Next.js static export written to `frontend/out`. The preview
serves it on `http://127.0.0.1:8092` with the production routes and headers.
`SB_PUBLIC_BACKEND_URL` is fixed at build time and selects the API; without it
the build targets the production API. Set the backend's `SB_ORIGIN` to the
exact frontend origin. With the same local build,
`npx --yes pnpm@10.33.0 --dir frontend e2e` runs the end-to-end suite in
headless Google Chrome against a mocked API. See
[deployment](docs/DEPLOYMENT.md), [frontend setup](frontend/README.md), and the
[CLI reference](docs/CLI.md). The [HTTP API reference](docs/API.md) documents
the org-free authentication and resource endpoints.

The default database is `silicon-browser-accounts.db`. Keep the previous IAM
database, encryption key and recording receipts as a separate backup. Startup
refuses to reinterpret existing IAM resources as Accounts ownership. Account
migration requires a verified old-principal-to-Accounts-UUID mapping; handles
alone do not establish continuity. The existing Accounts database has already
undergone its coordinated UUIDv4 migration; see the retained-data evidence in
[the cutover record](docs/ACCOUNT_UUID_CUTOVER.md). The former IAM testing worlds and Briefcase
OBO authorization flow are retired. See [recording integration status](docs/BRIEFCASE_INTEGRATION.md).

## Validate

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo doc --workspace --no-deps
npx --yes pnpm@10.33.0 --dir frontend test
npx --yes pnpm@10.33.0 --dir frontend build
node frontend/scripts/sync-silicon-ui.mjs --check
python3 scripts/test_apps_package.py
python3 scripts/test_installer.py
```

Tests use local fakes and do not consume provider capacity. Current verification
and dated live evidence are indexed in [production readiness](docs/PRODUCTION_READINESS.md).
The [1.1.0 report](docs/operations/browser-1.1.0-2026-10-10.md) remains a record
of that release. Historical integration
notes are retained under [docs/legacy](docs/legacy).
