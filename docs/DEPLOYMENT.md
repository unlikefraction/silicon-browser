# Deployment

The frontend runs at `https://browser.teamofsilicons.com`; a native Rust API runs
at `https://backend.browser.teamofsilicons.com`. Local CLI controllers connect
directly to remote browsers. The API host needs no Chromium or browser controller.

## Backend configuration

Build `cargo build --locked --release -p silicon-browser-backend` for the host.
Run under its service manager with the private environment shown in
[.env.example](../.env.example). Set `SB_ORIGIN` to the exact frontend origin,
`ACCOUNTS_APP_ID=browser`, `ACCOUNTS_APP_SECRET` to Browser's `sa_app_` secret,
and `ACCOUNTS_URL=https://accounts.teamofsilicons.com`. The browser-provider key
is required; search is enabled by a comma-separated `TINYFISH_API_KEYS` pool.

Keep SQLite WAL data on private persistent local storage. Retain the stable
`SB_ENCRYPTION_KEY` across restarts and upgrades. Terminate HTTPS at ingress.
`/healthz` verifies process availability; test authenticated API/provider paths
separately. Existing systemd and AWS infrastructure templates can host the
binary, but their stored runtime secret must be updated to Accounts variables.
AWS IAM instance roles in infrastructure templates are infrastructure access
controls and are unrelated to the retired Silicon IAM identity service.

The Accounts release defaults to `silicon-browser-accounts.db` and checks its
authorization schema before serving. Do not point it at the previous IAM
resource database. Preserve that database, encryption key and delivery receipts;
importing ownership requires an authenticated mapping to permanent Accounts
UUIDs. Never infer continuity from a reused c:/si: handle. Historical SQL
migrations remain unchanged so backups retain their original checksums.

The backup service reads the API's runtime environment and working directory,
so it snapshots the file selected by `SB_DATABASE_URL`. Confirm that setting
points to the new Accounts database; retain the previous database separately.

## Register sign-in

Configure Browser's Silicon Accounts redirect URI as
`https://browser.teamofsilicons.com/auth/callback` and allow the frontend origin.
Carbon login uses server-owned PKCE, state and a one-use callback claim. Silicon
login uses an application-bound `slt_` token. The server introspects Accounts
JWTs live, checks audience and membership, and uses the permanent 128-bit UUID
for ownership. Accounts has no organizations or tags; Browser maintains one
application workspace and explicit sharing.

Configure the Accounts webhook at
`https://backend.browser.teamofsilicons.com/webhooks/accounts` and set
`ACCOUNTS_WEBHOOK_SECRET`. Verification uses `X-Accounts-Timestamp`,
`X-Accounts-Signature` and the exact raw body. Verified events invalidate the
bounded authorization cache. The cache can delay a revocation by at most its
configured short lifetime. Already issued provider CDP/live capabilities remain
valid until the provider ends the session.

Refresh tokens rotate once. OAuth exchanges and refreshes claim a hashed
credential before contacting Accounts. A duplicate or uncertain submission
requires fresh sign-in; the API never retries it against Accounts. Run local
tests for wrong audiences, invalid signatures and callback replay, then verify
real redirect and webhook registration before deployment.

## Frontend

Use Node 24, Vercel root `frontend` and the Vite preset. Set the public
`SB_PUBLIC_BACKEND_URL` at build time; no service secret belongs in frontend
configuration. Requests omit cookies and refuse redirects. Sign-ins are saved
in origin-scoped localStorage shared across tabs; callback codes and live
invitation secrets are removed from URLs.

For local checks:

```sh
SB_ORIGIN=http://127.0.0.1:8092 PORT=8091 cargo run -p silicon-browser-backend
SB_PUBLIC_BACKEND_URL=http://127.0.0.1:8091 pnpm --dir frontend build
node frontend/dev.mjs
```

Register the corresponding local Accounts callback URI if exercising hosted
sign-in. [Frontend setup](../frontend/README.md) describes the UI and tests.

## Release

Silicon Apps hosts CLI packages and updates, not the API/frontend. Follow
[Apps distribution](APPS.md) to build native packages and publish Browser 1.1.0.
Do not release a CLI against an API still running the old auth contract.
Keep secrets, backups and operational data outside packages. Run an actual
restore check and a provider session smoke test before production cutover.
Automatic Briefcase OBO delivery is retired; see [recording status](BRIEFCASE_INTEGRATION.md).

For the coordinated existing-Accounts UUID upgrade, use [the offline cutover runbook](ACCOUNT_UUID_CUTOVER.md). It does not adopt the preserved IAM database.
