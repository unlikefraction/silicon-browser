# Silicon Browser

Silicon Browser gives Carbons and Silicons managed browser profiles, shared access, expiring sessions, live handoff, recordings, command history and usage accounting. The primary interface is the `silicon-browser` Rust crate; `sb` adds local setup, remembered authentication and a native browser controller. A minimal SolidJS frontend handles sign-in, profiles, sessions and live viewing.

Browser commands execute on the caller's machine and connect directly to the remote browser. The backend manages authentication, access and session metadata; it never executes `sb run` or relays browser output. The product contract is [UNDERSTANDING.md](UNDERSTANDING.md).

## Try the CLI

Run this in a macOS or Linux terminal:

```sh
curl -fsSL https://raw.githubusercontent.com/unlikefraction/silicon-browser/v2/scripts/install.sh | sh && export PATH="$HOME/.local/bin:$PATH"
```

The installer detects Intel/x86-64 or ARM64, verifies the release SHA-256, installs `sb` into `~/.local/bin`, updates your shell's PATH, and runs interactive setup. Follow the prompts for your organization and fresh IAM authorization tokens; setup checks recording delivery and installs the native browser controller. No Rust, Node/npm, local Chromium, or sudo is needed. Requires `curl`, `tar`, and `sha256sum` or `shasum`; Linux requires glibc 2.34 or later (Alpine/musl and 32-bit machines are not supported by this release). Running the command again reinstalls the pinned release and reruns setup.

Then explore the commands:

```sh
sb --help
sb --help remote-browser
sb --help search-and-fetch
```

For installation without interactive setup, use `curl -fsSL https://raw.githubusercontent.com/unlikefraction/silicon-browser/v2/scripts/install.sh | sh -s -- --no-setup`, then run `~/.local/bin/sb setup` when ready. You can also [download prebuilt binaries](https://github.com/unlikefraction/silicon-browser/releases/tag/managed-v0.1.1) or use `cargo install silicon-browser-cli --version 0.1.1 --locked` with Rust 1.98 or later. For a source checkout, use `cargo install --path crates/cli`.

The dashboard is live at [browser.teamofsilicons.com](https://browser.teamofsilicons.com). The CLI uses its production API by default. The published [Rust client](https://crates.io/crates/silicon-browser) is available with `cargo add silicon-browser@0.1.1`.

Setup accepts a fresh org-bound IAM `oac_` token interactively or through `SB_AUTHTOKEN`. It exchanges the one-shot token and never persists it. An `oat_` in that variable is an explicit invocation-only bearer override. Setup installs the pinned native controller with integrity checks; it does not install local Chromium, Node or npm. Running setup again is safe.

Private CLI state lives in `~/.silicon-browser`, partitioned by normalized backend URL. `SB_HOME` isolates a separate root for tests. Local controller namespaces also distinguish organization, immutable principal and session. This lets different backends and logged-in identities use the CLI without sharing authentication or controller state.

```sh
sb profile ls
sb session new PROFILE_ID --name "Research" --description "Vendor comparison" --ttl 30m
sb run SESSION_ID "open https://example.com"
sb run SESSION_ID "snapshot -i"
sb run SESSION_ID "screenshot ./research.png"
sb session end SESSION_ID --note "done"
```

Screenshots, PDFs and local recordings write to your machine. Uploads transfer actual local bytes to file inputs, including hidden inputs and snapshot references. `download <link-selector> <local-path>` copies ordinary same-origin HTTP links with browser credentials, or blob/data links, to an atomically finalized local file. Button/script/POST downloads, cross-origin frames, `wait --download` and `--download-path` are unsupported and fail explicitly. File bytes move over the direct browser connection; they do not pass through the Browser API.

Connection replacement and session lifecycle remain managed by `sb`; use `sb session end` instead of the controller's `close`. Local controller daemons exit after five idle minutes. Browser commands otherwise use the pinned local controller's remote-CDP capabilities. `sb run --help` reveals the supported action reference using Silicon Browser command names.

`sb --json ...` provides structured output. Run callbacks stream local stdout/stderr while the action runs. After completion, the CLI saves command metadata and submits an idempotent report; browser output is never included. Failed report delivery cannot repeat the browser action. `sb session sync SESSION_ID` retries queued reports.

For read-heavy work, no browser profile or session is needed:

```sh
sb search "browser automation" --purpose "Find primary sources"
sb fetch https://example.com,https://example.org --purpose "Extract the relevant claims"
```

These requests use the backend's shared search-provider key pool, batching and fair scheduling. With an org-bound OAT override, a clean CLI resolves a sole available organization; zero or multiple organizations require explicit setup or `--org-id`.

Recording discovery covers profiles, session actors, incognito runs and name/description metadata:

```sh
sb recording ls --filter "profile:PROFILE_ID -> for:@silicon-id -> name:^research"
sb recording ls --filter "is:incognito -> contains:checkout"
```

`is:shared` means visible but owned by someone else, including visibility inherited from a profile ACL. `sb usage show --org` returns the organization aggregate and accepts a `between:` window; individual usage remains scoped to visible sessions.

`sb usage limits` shows the provider account's current concurrent-browser entitlement. The dashboard's Usage page displays the same value; successful results are cached for at most one minute, so a plan change requires no Browser redeployment.

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

The frontend uses SolidJS, TypeScript and Vite, with IAM's typography and visual style. Set Vercel's Root Directory to `frontend`, Framework Preset to Vite and Node version to 24.x. Set the backend's `SB_ORIGIN` to the exact frontend origin for CORS and generated live links. Frontend API requests go directly to AWS, and its live iframe connects directly to the authorized remote viewer. See [deployment configuration](docs/DEPLOYMENT.md) and [frontend setup](frontend/README.md).

## Architecture and boundaries

- The backend alone owns IAM application credentials and provider API keys. Authorized callers receive sensitive CDP/live capabilities; these URLs are credentials, not public links. API authorization controls issuance and renewal, but cannot retract a capability already issued by the remote provider. Closing or expiring the remote session ends it.
- Production IAM authorization snapshots are cached per token and organization for at most 15 seconds and never past token expiry. A verified `/webhooks/iam` delivery invalidates the cache. Configure the matching `IAM_WEBHOOK_SECRET` and key version; without the secret the webhook route is disabled. Undisclosed IAM tags grant no access.
- The CLI caches a connection for at most 60 seconds, bounded by session expiry. Local invocations in the same controller namespace are serialized. Separate machines can act concurrently; the backend does not order browser actions.
- Command reports contain command text/flags, client timestamps, exit status and a stable UUID. Silicon-session commands are encrypted at rest; Carbon sessions retain no command history. Logs are cooperative, ordered by receipt, and can omit direct actions or reports still offline when archival begins. Exact retries return the same receipt. A new report after archive closure returns `409 report_window_closed` and stays queued locally. See [command execution](docs/COMMAND_EXECUTION_GAPS.md).
- Native recording capture belongs to the remote browser provider. The backend copies completed MP4s and Silicon command JSONL into the initiator's Briefcase using an independently authorized IAM family. `sb setup` uses a second fresh Browser SLT (`SB_RECORDING_SLT` or masked prompt), never the CLI refresh token. The web UI obtains that separate authorization through IAM sign-in.
- Configure `BRIEFCASE_URL` and `BRIEFCASE_APP_ID` together; session creation requires recording authorization. Transfer staging defaults to 512 MiB per artifact. OBO upload must finish within its proof lifetime, at most 60 seconds. A lost successful response can produce an identical additional file version on retry. Briefcase owns paths and retention; `sb recording rm` hides locally. `sb recording send SESSION_ID` retries eligible exhausted failures while preserving completed receipts. See [delivery](docs/BRIEFCASE_INTEGRATION.md) and [remaining integration limits](docs/BRIEFCASE_INTEGRATION_GAPS.md).

Internally, Browser Use provides remote browsers and native recordings, the pinned `agent-browser` binary supplies local control, and TinyFish supplies search/fetch. Provider adapters isolate those integrations from the public product. Browser Use currently reports aggregate proxy traffic/cost, preserved as unclassified usage; explicit-null incognito proxy metering remains an [external finding](docs/BROWSER_PROVIDER_FINDINGS.md). A profile fingerprint is a stable Silicon profile identity, not a disclosed upstream anti-detect fingerprint. Unsupported TinyFish search/fetch flags fail explicitly; `purpose` is retained as audit intent because its API has no matching field.

A database constraint permits one active session per profile. Lifecycle recovery uses provider correlation metadata; an ambiguous create cannot release the slot merely because an immediate lookup finds nothing. Stop confirmation precedes slot release. These safeguards concern session management, not browser-command execution.

## Workspace and validation

- `crates/shared`: domain types, validation, ACLs and filters.
- `crates/client`: stateless Rust metadata API and local controller with callbacks.
- `crates/cli`: stateful `sb`, backend-specific auth, local runtime and report queue.
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

IAM integration tests use a test application and `IAM_TEST_ENVIRONMENT_KEY`, the environment's secret root key rather than its public UUID. Leave that variable unset in production. Briefcase tests need its separate key paired with the IAM environment. The backend uses `silicon-iam-client 1.2.1`; recorded CLI checks used IAM 1.2.2 and Briefcase 0.1.3.

`scripts/test_live_auth.py` checks exchange/refresh, identity and organization scoping, rejection behavior and optional CLI use without creating provider sessions. Supply `SB_TEST_BACKEND`, `SB_TEST_ORG`, a fresh `SB_TEST_SLT`, and optionally an absolute `SB_TEST_CLI`. It consumes and rotates the resulting test authorization without printing credentials.

External observations remain in local reports. IAM's sibling-OAT revocation behavior and independently owned refresh recovery are documented in [the IAM finding](docs/IAM_1_2_2_EXTERNAL_BUGS.md). Older provider and readiness reports are dated historical evidence, not claims that the removed server controller still exists.
