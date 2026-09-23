# CLI guide

`browser` is the reference IAM application CLI. It is designed for Carbon and
Silicon callers, and its help output is the primary documentation:

```sh
browser --help
browser --help remote-browser
browser --help search-and-fetch
browser session --help
browser run --help
```

Each command explains what it is for, a normal next step, accepted flags, and
the exact error code returned on failure. JSON output is intended for agents;
the standard form places `--json` after the command (`browser iam --json`).
The web dashboard is a convenience subset of these API capabilities; automation
should use the CLI or the stateless Rust client.

## Install

The supported macOS/Linux install also installs the native runner and starts
the setup flow. It never asks for a password or stores the one-shot IAM token.

Use the one-line setup command at the end of this guide.

For a non-interactive install, append `-s -- --no-setup`, then run
`~/.local/bin/browser setup` when an IAM token is available. The installer verifies
the release checksum and supports x86-64 and ARM64 (Linux requires glibc 2.34+).

## Authentication and state

```sh
browser iam --json
browser login "<short-lived IAM token>"
browser login status --json
```

The official IAM CLI can mint Browser's token directly. Pass the selected
organization when a Carbon has access to more than one workspace:

```sh
slt=$(iam login --app-id 'browser' --grant-org tos -o json | jq -r .slt)
browser --org-id tos login "$slt"
```

The SLT is single-use and expires quickly; Browser never receives an IAM
password, verification code, Silicon token, or refresh credential.

`iam --json` always returns an object containing `app_id`; `login status --json`
always returns `authenticated` and may include identity, organization, and
expiry details. Treat unknown JSON fields as forward-compatible additions.

Tokens are minted by the official IAM CLI or its web consent flow. Browser
never prompts for a username or password. `SILICON_HOME` is the shared home for
Silicon applications; Browser keeps its private state in
`$SILICON_HOME/.silicon-browser` (or `~/.silicon-browser` when unset).
`SB_HOME` is an explicit test/isolated-run override. Set `SB_BACKEND_URL` or
`--backend` before login to select a non-production backend.

## IAM test environments

Use the app secret from an IAM testing environment to enroll Browser:

```sh
# Reads SB_TEST_APP_SECRET; optional SB_IAM_TEST_KEY and SB_BRIEFCASE_TEST_KEY.
browser testing login
# Or read private JSON containing {"app_secret":"ask_..."} from stdin:
browser testing login --credentials-stdin < test-credentials.json

browser --test <environment-uuid> testing status --json
browser --test <environment-uuid> login si:worker --org-id tos
browser --test <environment-uuid> login status --json
browser --test <environment-uuid> setup
browser --test <environment-uuid> profile ls
```

Enrollment verifies the secret with IAM and reports its environment UUID.
JSON may also include `iam_test_key` and `briefcase_test_environment_key`;
`iam_test_key` is exactly 32 ASCII letters or digits. The Briefcase field holds
the imported Briefcase app secret from the same IAM world: `ask_` followed by
43 base64url characters. Its existing field and `SB_BRIEFCASE_TEST_KEY` names
remain compatible. The Browser app secret is
developer test configuration, never a production login credential. Test login
accepts an actor ID in that environment or an IAM test `oac_` token.
Production login continues to require a short-lived token.

With test recording storage configured, `browser --test <environment-uuid> setup`
authorizes background recording delivery as the authenticated test actor using
its own IAM token family. `SB_RECORDING_SLT` can explicitly supply that actor ID
or a fresh test SLT for the same actor. Production setup still requires a separate fresh SLT.

Pass `--test` on every command. Test calls use the same API routes under
`/testing/<environment-uuid>` and the same exchange/refresh flow. Credentials,
organization selection, runtime connections, and queued command reports are
isolated by backend URL and environment. Missing enrollment fails locally;
test mode never falls back to production. Re-enrollment clears that test
environment's saved login. Test configuration lives in Browser's existing
owner-only state directory. Browser sessions still need configured provider
credentials; the IAM test environment does not supply browser capacity.

## Command tree

```text
browser
├── iam --json
├── login <SLT>
│   └── status --json
├── testing {login [--credentials-stdin],status --json}
├── setup [--org <id>]
├── profile {ls,show,new,set,end}
├── proxy ls
├── session {new,ls,show,live,logs,sync,end}
├── run <session-id> <browser-command> [args...]
├── recording {ls,show,send,rm}
├── usage {limits,ls,show}
├── search <query> --purpose <text>
├── fetch <url[,url...]> --purpose <text>
└── report-bug --title <text> --details <text> [--pr <ref>]
```

Use `session new --incognito` for one-off work, or create a profile when the
same identity and proxy location must be reused. `run --help` is versioned with
the pinned native runner. Browser actions execute locally; command reports
contain metadata only and are retried by `session sync` without repeating an
action.

Proactive IAM apps add two standard branches:

```text
app webhook <url> [--secret <value>]
app unhook
```

The daemon owns one server WebSocket per machine and multiplexes all registered
Silicons. Webhook deliveries use `{type, data, metadata}`.

All IAM app CLIs should expose a bug-report command with a reproducible
description, diagnostics, and an optional `--pr <ref>`. In `browser`, use
`report-bug` (aliases: `bug`, `bug-report`). The command submits a report only;
agents can inspect the linked repository, patch it, and attach a pull request
themselves.

## Build and develop

The stateless Rust library is published as
[`silicon-browser`](https://crates.io/crates/silicon-browser). The `browser` binary
owns local state and talks to the backend; a long-running daemon may reuse the
same library without sharing mutable process state.

```sh
cargo run -p silicon-browser-cli -- --help
cargo test -p silicon-browser-cli
cargo test --workspace --all-targets
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Use the production pathways in tests with a test IAM application and test
backend credentials. Keep test and production transport/auth code identical;
only endpoints and keys should vary. Organization-wide BYO storage or keys are
configuration values, supplied through the app's documented environment
variables, and must never be embedded in the CLI.

## Telemetry, updates, and compatibility

Browser command reports are durable and idempotent. The four Space Station
stores have been created in `tos`: `browserbackend`, `browsercli`,
`browserfrontendanalytics`, and `browserfrontendevents`. This release does not
yet send events to those stores or expose a telemetry opt-out setting. There is
no frontend telemetry proxy or supported `SPACE_STATION_*` configuration yet.

The CLI installs a checksum-verified native controller. It does not yet run an
hourly update service; rerun the installer to update `browser`. The backend negotiates
its IAM v1 dependency contract. Browser's public API uses `/api/v1`; automatic
Browser client/server protocol negotiation is not implemented. The testing
routes in 0.2.1 are additive and preserve existing production API behavior.

## One-line setup

Install the CLI without authentication; then use `browser login` and `browser setup` when ready.

```sh
curl -fsSL https://raw.githubusercontent.com/unlikefraction/silicon-browser/managed-v0.3.1/scripts/install.sh | sh -s -- --no-setup && export PATH="$HOME/.local/bin:$PATH"
```
