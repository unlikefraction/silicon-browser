# CLI guide

`sb` is the reference IAM application CLI. It is designed for Carbon and
Silicon callers, and its help output is the primary documentation:

```sh
sb --help
sb --help remote-browser
sb --help search-and-fetch
sb session --help
sb run --help
```

Each command explains what it is for, a normal next step, accepted flags, and
the exact error code returned on failure. JSON output is intended for agents;
put `--json` before the command (`sb --json session ls`).

## Install

The supported macOS/Linux install also installs the native runner and starts
the setup flow. It never asks for a password or stores the one-shot IAM token.

Use the one-line setup command at the end of this guide.

For a non-interactive install, append `-s -- --no-setup`, then run
`~/.local/bin/sb setup` when an IAM token is available. The installer verifies
the release checksum and supports x86-64 and ARM64 (Linux requires glibc 2.34+).

## Authentication and state

```sh
sb iam --json
sb login "<short-lived IAM token>"
sb login status --json
```

`iam --json` always returns an object containing `app_id`; `login status --json`
always returns `authenticated` and may include identity, organization, and
expiry details. Treat unknown JSON fields as forward-compatible additions.

Tokens are minted by the official IAM CLI or its web consent flow. Browser
never prompts for a username or password. `SILICON_HOME` is the shared home for
Silicon applications; Browser keeps its private state in
`$SILICON_HOME/.silicon-browser` (or `~/.silicon-browser` when unset).
`SB_HOME` is an explicit test/isolated-run override. Set `SB_BACKEND_URL` or
`--backend` before login to select a non-production backend.

## Command tree

```text
sb
├── iam --json
├── login <SLT>
│   └── status --json
├── setup [--org <id>]
├── profile {ls,show,new,set,end}
├── proxy ls
├── session {new,ls,show,live,logs,sync,end}
├── run <session-id> <browser-command> [args...]
├── recording {ls,show,send,rm}
├── usage {limits,ls,show}
├── search <query> --purpose <text>
└── fetch <url[,url...]> --purpose <text>
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

All IAM app CLIs should also expose `app bug-report` with a reproducible
description, diagnostics, and an optional `--pr <ref>`. The command submits a
report only; agents can inspect the linked repository, patch it, and attach a
pull request themselves.

## Build and develop

The stateless Rust library is published as
[`silicon-browser`](https://crates.io/crates/silicon-browser). The `sb` binary
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

Space Station telemetry is opt-in by default for IAM apps. Events should be
self-contained (`source`, `step`, `progress`, correlation IDs, outcome, and
timestamps); do not include credentials or browser output. Browser command
reports are durable and idempotent. Deployments that add Space Station tables
use the `tos.browser` namespace with separate analytics and frontend-event
stores.

Daemons check for CLI updates hourly and apply a verified release while
preserving the current process. APIs negotiate a contract version during the
handshake; keep a compatibility matrix and deprecation/sunset dates in the
release notes. Breaking protocol changes require a new major version.

## One-line setup

```sh
curl -fsSL https://raw.githubusercontent.com/unlikefraction/silicon-browser/v2/scripts/install.sh | sh && export PATH="$HOME/.local/bin:$PATH"
```
