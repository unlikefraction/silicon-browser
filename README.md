# Silicon Browser

Silicon Browser gives Carbons and Silicons managed browser profiles, shared access, expiring sessions, live handoff, recordings, command history and usage accounting. The primary interface is the `silicon-browser` Rust crate; `browser` adds local setup, remembered authentication and a native browser controller. A minimal SolidJS frontend handles sign-in, profiles, sessions and live viewing.

Browser commands execute on the caller's machine and connect directly to the remote browser. The backend manages authentication, access and session metadata; it never executes `browser run` or relays browser output. The product contract is [UNDERSTANDING.md](UNDERSTANDING.md).

## Try the CLI

For the six-platform Honeycomb package (`tos>browser`), including Windows and
bundled browser controllers, see [Honeycomb distribution](docs/HONEYCOMB.md).
The application is public; install it with `honeycomb install 'tos>browser'`.

Run this in a macOS or Linux terminal:

```sh
curl -fsSL https://raw.githubusercontent.com/unlikefraction/silicon-browser/managed-v0.2.5/scripts/install.sh | sh && export PATH="$HOME/.local/bin:$PATH"
```

Since 0.2.5, the CLI command is `browser` (previously `sb`). Existing `SB_*` settings and saved authentication are reused.

The installer detects Intel/x86-64 or ARM64, verifies the release SHA-256, installs `browser` into `~/.local/bin`, updates your shell's PATH, and runs interactive setup. Follow the prompts for a fresh IAM authorization token; choose an organization only when that token authorizes more than one. Setup checks recording delivery and installs the native browser controller. No Rust, Node/npm, local Chromium, or sudo is needed. Requires `curl`, `tar`, and `sha256sum` or `shasum`; Linux requires glibc 2.34 or later (Alpine/musl and 32-bit machines are not supported by this release). Running the command again reinstalls the pinned release and reruns setup.

Then explore the commands:

```sh
browser --help
browser --help remote-browser
browser --help search-and-fetch
```

The [published documentation](https://browser.teamofsilicons.com/docs/) and
[CLI guide](docs/CLI.md) are the instructive references for authentication,
state layout, command-tree conventions, development, telemetry, and protocol
compatibility.
For IAM testing, start with [Use IAM testing environments](#use-iam-testing-environments).

Four Space Station tables have been provisioned, but Browser does not yet emit
runtime telemetry to them. See the [current capability status](docs/CLI.md#telemetry-updates-and-compatibility)
for telemetry, updates, and versioning limits.

For installation without interactive setup, use `curl -fsSL https://raw.githubusercontent.com/unlikefraction/silicon-browser/managed-v0.2.5/scripts/install.sh | sh -s -- --no-setup`, then run `~/.local/bin/browser setup` when ready. You can also [download prebuilt binaries](https://github.com/unlikefraction/silicon-browser/releases/tag/managed-v0.2.5) or use `cargo install silicon-browser-cli --version 0.2.5 --locked` with Rust 1.98 or later. For a source checkout, use `cargo install --path crates/cli`.

`browser login <SLT>` exchanges a short-lived IAM token; use `--org-id` when the IAM CLI token authorizes multiple workspaces. For example, `iam login --app-id 'tos>browser' --grant-org tos -o json | jq -r .slt` followed by `browser --org-id tos login '<SLT>'`. `browser login status --json` reports the current session, and `browser iam --json` prints the canonical application ID.

The dashboard is live at [browser.teamofsilicons.com](https://browser.teamofsilicons.com). The CLI uses its production API by default. The published [Rust client](https://crates.io/crates/silicon-browser) is available with `cargo add silicon-browser@0.2.5`.

Setup accepts a fresh IAM `oac_` token interactively or through `SB_AUTHTOKEN`. IAM supplies the token's organization authorization; `--org` selects one when several are available. Setup exchanges the one-shot token and never persists it. An `oat_` in that variable is an explicit invocation-only bearer override. Setup installs the pinned native controller with integrity checks; it does not install local Chromium, Node or npm. Running setup again is safe.

Private CLI state lives in `$SILICON_HOME/.silicon-browser` (or `~/.silicon-browser` when `SILICON_HOME` is unset), partitioned by normalized backend URL. `SB_HOME` remains an explicit override for tests and isolated runs. Local controller namespaces also distinguish organization, immutable principal and session. This lets different backends and logged-in identities use the CLI without sharing authentication or controller state.

```sh
browser profile ls
browser session new PROFILE_ID --name "Research" --description "Vendor comparison" --ttl 30m
browser run SESSION_ID "open https://example.com"
browser run SESSION_ID "snapshot -i"
browser run SESSION_ID "screenshot ./research.png"
browser session end SESSION_ID --note "done"
```

Screenshots, PDFs and local recordings write to your machine. Uploads transfer actual local bytes to file inputs, including hidden inputs and snapshot references. `download <link-selector> <local-path>` copies ordinary same-origin HTTP links with browser credentials, or blob/data links, to an atomically finalized local file. Button/script/POST downloads, cross-origin frames, `wait --download` and `--download-path` are unsupported and fail explicitly. File bytes move over the direct browser connection; they do not pass through the Browser API.

Connection replacement and session lifecycle remain managed by `browser`; use `browser session end` instead of the controller's `close`. Local controller daemons exit after five idle minutes. Browser commands otherwise use the pinned local controller's remote-CDP capabilities. `browser run --help` reveals the supported action reference using Silicon Browser command names.

`browser --json ...` provides structured output. Run callbacks stream local stdout/stderr while the action runs. After completion, the CLI saves command metadata and submits an idempotent report; browser output is never included. Failed report delivery cannot repeat the browser action. `browser session sync SESSION_ID` retries queued reports.

For read-heavy work, no browser profile or session is needed:

```sh
browser search "browser automation" --purpose "Find primary sources"
browser fetch https://example.com,https://example.org --purpose "Extract the relevant claims"
```

These requests use the backend's shared search-provider key pool, batching and fair scheduling. With an OAT override, a clean CLI resolves a sole available organization; zero or multiple organizations require setup or `--org-id` to select the workspace.

Recording discovery covers profiles, session actors, incognito runs and name/description metadata:

```sh
browser recording ls --filter "profile:PROFILE_ID -> for:@silicon-id -> name:^research"
browser recording ls --filter "is:incognito -> contains:checkout"
```

`is:shared` means visible but owned by someone else, including visibility inherited from a profile ACL. `browser usage show --org` returns the organization aggregate and accepts a `between:` window; individual usage remains scoped to visible sessions.

`browser usage limits` shows the provider account's current concurrent-browser entitlement. The dashboard's Usage page displays the same value; successful results are cached for at most one minute, so a plan change requires no Browser redeployment.

## Use IAM testing environments

Use CLI 0.2.2 or later, the production API, or the website
[Testing environment selector](https://browser.teamofsilicons.com). The
[published testing API guide](https://browser.teamofsilicons.com/docs/#api)
also shows how to supply a test app secret directly to the API.

Create a Browser test app and actors in IAM, then enroll its `ask_` app secret:

```sh
# Read a private file containing {"app_secret":"ask_..."}.
browser testing login --credentials-stdin < test-credentials.json

# Use the environment UUID returned by enrollment on every test command.
browser --test <environment-uuid> --org-id tos login worker:tos
browser --test <environment-uuid> login status --json
browser --test <environment-uuid> testing status --json
browser --test <environment-uuid> profile ls
```

`browser testing login` also reads `SB_TEST_APP_SECRET`. The app secret alone selects
and verifies the IAM environment; an IAM root key is optional. Test login accepts
an existing test actor ID such as `alice` or `worker:tos`, or an IAM test `oac_`
short-lived token. Production login still requires a short-lived token. See
[`browser testing` configuration](docs/CLI.md#iam-test-environments) for optional keys
and the full command flow.

**Creating browser sessions requires a Briefcase test application secret and recording
authorization.** Add `briefcase_test_environment_key` to the enrollment JSON, or
set `SB_BRIEFCASE_TEST_KEY` when enrolling through environment variables. This is
the `app_secret` returned when importing `tos>briefcase` into the same IAM test world:
`ask_` followed by 43 base64url characters, not a 32-character root key.
Run `browser --test <environment-uuid> setup` to install/check the local controller
and authorize a separate recording token family using the signed-in test actor.
`SB_RECORDING_SLT` can instead supply a fresh Browser test SLT. Missing
recording configuration fails explicitly; test recordings never use production
Briefcase storage.

On the website, select **Testing environment** from the welcome screen or the
workspace header. Enter the test app secret, an existing actor ID or test SLT,
and the organization. Add the Briefcase test application secret to create browser sessions,
then authorize recording access in Settings. The test badge identifies the
environment; **Exit test mode** restores the production workspace. Web test keys
and sign-ins stay in memory, and leaving or reloading clears them. Invalid
enrollment leaves the current workspace unchanged. See the
[website testing instructions](frontend/README.md#use-an-iam-testing-environment).

Testing uses the normal API handlers and real browser/search providers, so
provider usage is metered. It isolates Browser data and authentication; it does
not provision free browser capacity. Backend data is partitioned by the verified
IAM environment UUID and its clean generation. Cleaning the IAM environment
selects fresh Browser storage; rotating its app secret preserves that
generation's data. Missing or mismatched test credentials never fall back to
production.

## Run the backend and frontend

Copy `.env.example` to `.env` and fill its server-only values. Generate a stable local encryption key with `openssl rand -hex 32`.

```sh
cargo run -p silicon-browser-backend
curl --fail http://127.0.0.1:8080/healthz
```

The production layout is a native AWS daemon at `https://backend.browser.teamofsilicons.com` and a separate Vercel frontend at `https://browser.teamofsilicons.com`. Build the backend for the host's operating system and architecture:

```sh
cargo build --locked --release -p silicon-browser-backend
```

Run that binary under the service manager. It handles SIGTERM and serves only API/health/webhook routes. The host needs no browser controller, Chromium or container runtime. SQLite uses WAL on persistent private storage; retain its encryption key across upgrades. Background tasks handle session expiry/recovery and transfer completed native recordings into Briefcase. They do not execute user commands.

Test SQLite databases live beside the main database in `<database-file>.testing/`.
The main database retains an encrypted test-environment registry so cleanup and
authorized recording delivery can resume after restart. Keep that directory,
the main database, and the stable encryption key on persistent private storage.

The frontend uses SolidJS, TypeScript and Vite, with IAM's typography and visual style. Set Vercel's Root Directory to `frontend`, Framework Preset to Vite and Node version to 24.x. Set the backend's `SB_ORIGIN` to the exact frontend origin for CORS and generated live links. Frontend API requests go directly to AWS, and its live iframe connects directly to the authorized remote viewer. See [deployment configuration](docs/DEPLOYMENT.md) and [frontend setup](frontend/README.md).

## Architecture and boundaries

- The backend alone owns IAM application credentials and provider API keys. Authorized callers receive sensitive CDP/live capabilities; these URLs are credentials, not public links. API authorization controls issuance and renewal, but cannot retract a capability already issued by the remote provider. Closing or expiring the remote session ends it.
- Production IAM authorization snapshots are cached per token and organization for at most 15 seconds and never past token expiry. A verified `/webhooks/iam` delivery invalidates the cache. Configure the matching `IAM_WEBHOOK_SECRET` and key version; without the secret the webhook route is disabled. Undisclosed IAM tags grant no access.
- The CLI caches a connection for at most 60 seconds, bounded by session expiry. Local invocations in the same controller namespace are serialized. Separate machines can act concurrently; the backend does not order browser actions.
- Command reports contain command text/flags, client timestamps, exit status and a stable UUID. Silicon-session commands are encrypted at rest; Carbon sessions retain no command history. Logs are cooperative, ordered by receipt, and can omit direct actions or reports still offline when archival begins. Exact retries return the same receipt. A new report after archive closure returns `409 report_window_closed` and stays queued locally. See [command execution](docs/COMMAND_EXECUTION_GAPS.md).
- Native recording capture belongs to the remote browser provider. The backend copies completed MP4s and Silicon command JSONL into the initiator's Briefcase using an independently authorized IAM family. `browser setup` uses a second fresh Browser SLT (`SB_RECORDING_SLT` or masked prompt), never the CLI refresh token. The web UI obtains that separate authorization through IAM sign-in.
- Configure `BRIEFCASE_URL` and `BRIEFCASE_APP_ID` together; session creation requires recording authorization. Transfer staging defaults to 512 MiB per artifact. OBO upload must finish within its proof lifetime, at most 60 seconds. A lost successful response can produce an identical additional file version on retry. Briefcase owns paths and retention; `browser recording rm` hides locally. `browser recording send SESSION_ID` retries eligible exhausted failures while preserving completed receipts. See [delivery](docs/BRIEFCASE_INTEGRATION.md) and [remaining integration limits](docs/BRIEFCASE_INTEGRATION_GAPS.md).

Internally, Browser Use provides remote browsers and native recordings, the pinned `agent-browser` binary supplies local control, and TinyFish supplies search/fetch. Provider adapters isolate those integrations from the public product. Browser Use currently reports aggregate proxy traffic/cost, preserved as unclassified usage; explicit-null incognito proxy metering remains an [external finding](docs/BROWSER_PROVIDER_FINDINGS.md). A profile fingerprint is a stable Silicon profile identity, not a disclosed upstream anti-detect fingerprint. Unsupported TinyFish search/fetch flags fail explicitly; `purpose` is retained as audit intent because its API has no matching field.

A database constraint permits one active session per profile. Lifecycle recovery uses provider correlation metadata; an ambiguous create cannot release the slot merely because an immediate lookup finds nothing. Stop confirmation precedes slot release. These safeguards concern session management, not browser-command execution.

## Workspace and validation

- `crates/shared`: domain types, validation, ACLs and filters.
- `crates/client`: stateless Rust metadata API and local controller with callbacks.
- `crates/cli`: stateful `browser`, backend-specific auth, local runtime and report queue.
- `crates/backend`: metadata API, encrypted SQLite, IAM and provider adapters, recording delivery.
- `frontend`: SolidJS/TypeScript app built with Vite for Vercel.

```sh
cargo fmt --all -- --check
CARGO_INCREMENTAL=0 cargo test --workspace --all-targets
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets -- -D warnings
CARGO_INCREMENTAL=0 cargo doc --workspace --no-deps
npm ci --prefix frontend
npm test --prefix frontend
npm run build --prefix frontend
```

Automated tests use fakes and do not read `.env` or create paid browsers. A synthetic file-backed WAL test exercises 500 simultaneous clients fetching connections and reporting commands; it does not establish a deployed SLA or provider quota. [Readiness evidence](docs/PRODUCTION_READINESS.md) distinguishes current code checks from earlier real Carbon/Silicon recording and live-view tests.

For ordinary integration use, follow [IAM testing enrollment](#use-iam-testing-environments)
on the normal backend. No dedicated test backend or server-wide IAM root key is
required. Tests that deliberately start an entire backend in a single test
plane may still configure `IAM_TEST_ENVIRONMENT_KEY`, the secret root key rather
than the public UUID. Leave that server-wide variable unset on a production
backend. Briefcase requires its imported IAM application secret from that same world.

`scripts/test_live_auth.py` checks exchange/refresh, identity and organization scoping, rejection behavior and optional CLI use without creating provider sessions. Supply `SB_TEST_BACKEND`, `SB_TEST_ORG`, a fresh `SB_TEST_SLT`, and optionally an absolute `SB_TEST_CLI`. It consumes and rotates the resulting test authorization without printing credentials.

This low-level auth harness targets a separately configured test backend;
it does not enroll an environment or verify browser-provider sessions:

```sh
slt=$(iam --test <environment-uuid> login --app-id 'tos>browser' --grant-org tos -o json | jq -r .slt)
SB_TEST_BACKEND=https://browser-test.example SB_TEST_ORG=tos SB_TEST_SLT="$slt" \
  SB_TEST_CLI=/absolute/path/to/browser python3 scripts/test_live_auth.py
```

For the shared backend's `/testing/<environment-uuid>` routes, run the opt-in
IAM testing smoke after exporting `SB_TEST_APP_SECRET`, `SB_TEST_ACTOR` (an
existing test actor ID), and `SB_TEST_ORG`:

```sh
SB_TEST_BACKEND=http://127.0.0.1:8080 SB_TEST_CLI="$PWD/target/debug/browser" \
  python3 scripts/test_iam_testing.py
```

Optional `SB_IAM_TEST_KEY` and `SB_BRIEFCASE_TEST_KEY` are forwarded when set;
omit `SB_TEST_CLI` to check only the API. The script verifies enrollment,
distinct actor token families, refresh, listings, environment isolation, and
the CLI's temporary credential partition. It creates no browser/provider
sessions and prints no credentials or response bodies.

Production credentials and test credentials are rejected across planes; no
production browser session is created by this test.

External observations remain in local reports. IAM's sibling-OAT revocation behavior and independently owned refresh recovery are documented in [the IAM finding](docs/IAM_1_2_2_EXTERNAL_BUGS.md). Older provider and readiness reports are dated historical evidence, not claims that the removed server controller still exists.
