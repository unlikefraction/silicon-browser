# Browser CLI

`browser` uses Silicon Accounts and is distributed through Silicon Apps. Install
with `silicon-apps install browser` once Browser 1.1.0 is published. Apps owns
updates; Browser does not run a second updater.

## Discovery and sign-in

```sh
browser --help
browser --help remote-browser
browser --help search-and-fetch
browser accounts --json
browser login status --json
silicon-accounts login --app browser
browser login --slt TOKEN
browser setup
```

The first three commands describe the app and its actions. `accounts --json`
always returns `app_id: browser`, even offline. Signed-out login status returns
`authenticated: false` without contacting a backend. Signed-in status includes
the account identity and verifies it with the API.

A Silicon passes an Accounts `slt_` token once. A Carbon uses the hosted Accounts
sign-in on the Browser website. The CLI accepts a positional SLT for compatibility
with scripts. `--json` may be placed before or after ordinary commands; flags
inside `browser run` belong to the browser action.

Credentials are private and partitioned by normalized backend URL under
`${SILICON_HOME:-$HOME}/.silicon-browser`. `SB_HOME` selects an isolated state
root; `SB_BACKEND_URL` selects the API. `SB_AUTHTOKEN` can override the access
JWT for one invocation, or supply an SLT to `browser setup`. Environment access
tokens never replace a saved login. Tokens from the retired identity service
are ignored and require a fresh Accounts sign-in.

Refresh is serialized locally. Browser consumes a saved refresh token before
sending it, then saves the rotated pair atomically. An interrupted or rejected
refresh needs a new sign-in; resubmitting an old `sar_` token could revoke the
entire Accounts session. Browser actions themselves are never automatically
repeated.

## Profiles and sessions

```sh
browser proxy ls
browser profile new --name Research --location US --access '[@Account123]'
browser profile ls
browser profile show PROFILE_ID
browser profile set PROFILE_ID --access '[@Account123,@si:assistant]'
browser profile end PROFILE_ID --note done
browser session new PROFILE_ID --name Research --description "Read vendor docs" --ttl 30m
browser session new --incognito --name Quick --description "One task" --ttl 15m
browser session ls
browser session show SESSION_ID
browser session live SESSION_ID
browser session end SESSION_ID --note done
```

Ownership and grants store Accounts UUIDs, preserving case. A public `c:` or
`si:` selector is resolved to the account UUID when access changes. The owner
is always retained. Accounts provides no tag or organization authority;
Browser enforces sharing in its application workspace. Existing `--org-id`
wire compatibility selects that Browser workspace, not an Accounts organization.

Each profile has at most one active session. Profile sessions accept 15m, 30m,
45m, 60m, 120m or 240m; incognito defaults to 15m.

## Local actions and logs

```sh
browser run SESSION_ID "open https://example.com"
browser run SESSION_ID "snapshot -i"
browser run SESSION_ID "click @e1"
browser run SESSION_ID "fill @e2 'hello world'"
browser run SESSION_ID "screenshot ./page.png"
browser run SESSION_ID "upload input[type=file] ./document.pdf"
browser session logs SESSION_ID
browser session sync SESSION_ID
```

`browser run --help` prints the pinned controller's supported commands. Setup
uses the bundled native controller when available and checks its version and
integrity. No Node runtime or local Chromium is required by the installed CLI.

Screenshots, PDFs and local recordings write to your machine. Uploads send
actual local bytes directly to the remote browser. Downloads support
same-origin HTTP links and blob/data links; script/button/POST downloads,
cross-origin frames and `wait --download` fail explicitly. End sessions with
`browser session end`.

Command output streams locally. Command reports contain command text/flags,
timestamps, exit code and a stable UUID; browser output is excluded. A failed
metadata report is queued and can be retried without repeating the action.

## Recordings, usage and discovery

```sh
browser recording ls --filter 'profile:PROFILE_ID -> name:^research'
browser recording show SESSION_ID
browser recording rm SESSION_ID
browser usage limits
browser usage ls
browser usage show SESSION_ID
browser search "query" --purpose "Find sources"
browser fetch https://example.com,https://example.org --purpose "Extract claims"
```

Recording and usage filters support AND-separated pipelines; inspect each
command's help for the accepted fields. `recording rm` hides a Browser record.
Automatic Briefcase delivery is retired with the old OBO protocol; see
[recording status](BRIEFCASE_INTEGRATION.md). Native local recording commands
continue writing files on the caller's machine.

Search and fetch require sign-in but no profile/session. The API schedules the
configured provider key pool. Browser capacity limits reflect the shared
provider account rather than a per-user quota.

## Development and releases

The workspace uses Rust 1.98 or later. Run the workspace tests and frontend
checks documented in [README](../README.md). [Apps distribution](APPS.md)
describes the package manifest, native build receipts, supported targets and
publication process. Accounts has no IAM testing worlds; integration tests use
local fake HTTP services or a separately registered Accounts application.
