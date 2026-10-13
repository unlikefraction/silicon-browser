# Deployment

The frontend runs at `https://browser.teamofsilicons.com`; a native Rust API runs
at `https://backend.browser.teamofsilicons.com`. Both are served from the existing
AWS host, with nginx serving the built website. Local CLI controllers connect
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
UUIDv4s. Never infer continuity from a reused c:/si: handle. The existing
Accounts database's coordinated UUIDv4 cutover is recorded in
[the migration runbook and evidence](ACCOUNT_UUID_CUTOVER.md); it is separate
from the preserved IAM database. Historical SQL
migrations remain unchanged so backups retain their original checksums.

The backup service reads the API's runtime environment and working directory,
so it snapshots the file selected by `SB_DATABASE_URL`. Confirm that setting
points to the new Accounts database; retain the previous database separately.

## Register sign-in

Configure Browser's Silicon Accounts redirect URI as
`https://browser.teamofsilicons.com/auth/callback` and allow the frontend origin.
Carbon login uses server-owned PKCE, state and a one-use callback claim. Silicon
login uses an application-bound `slt_` token. The server introspects Accounts
JWTs live, checks issuer, `browser` audience/client ID and application membership,
and uses the permanent canonical lowercase, hyphenated UUIDv4
for ownership. Accounts has no organizations or tags; Browser maintains one
application and explicit sharing. No login payload or client configuration
needs an organization selector.

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

The website is a Next.js static export (Pages Router, `output: 'export'`): a
folder of plain files, with no Node server in production. Use Node 24 or later
and the locked frontend dependencies:

```sh
npx --yes pnpm@10.33.0 --dir frontend install --frozen-lockfile
npx --yes pnpm@10.33.0 --dir frontend test
npx --yes pnpm@10.33.0 --dir frontend build
```

`build` runs `tsc`, `next build --webpack`, `publish-docs.mjs` and
`scripts/check-export.mjs`, and writes `frontend/out` (there is no `dist`). The
last step fails the build on any executable inline script, inline event handler
or source map, because the production policy keeps `script-src 'self'`.

`SB_PUBLIC_BACKEND_URL` is read once at build time and pinned into the bundle.
It defaults to `https://backend.browser.teamofsilicons.com`; the only other
accepted value is `http://127.0.0.1:8091` for local testing, and visitor URLs
cannot change it. No service secret belongs in frontend configuration. Make the
production build last, without the override, so an end-to-end build pinned to
the local API is never deployed: `frontend/out` must contain
`https://backend.browser.teamofsilicons.com` and not `http://127.0.0.1:8091`.

Stage `frontend/out` in a new release-named directory under
`/opt/silicon-browser/web-releases/`. Copy the previous release's `assets/`
(Vite) and `_next/static/` files into it without overwriting anything, so tabs
opened on an earlier release can still load their hashed files; a same-named
file with different bytes is an error. Retain the previous directory for
rollback.

The vhost `/etc/nginx/conf.d/browser-frontend.conf` must:

- Send `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`,
  `Referrer-Policy: no-referrer` and the Content-Security-Policy from
  `frontend/vercel.json` on every response, with `always`. The policy is
  identical to the Vite release's except `style-src 'self' 'unsafe-inline'`,
  which Silicon UI's runtime styles need; `script-src` stays `'self'`.
- Serve `/index.html` for exactly the three app routes `/`, `/auth/callback` and
  `/sessions/<id>/live`. `/docs` serves `/docs/index.html`, and `/docs/` uses
  the directory index.
- Serve content-hashed files from `location ^~ /_next/static/`. They are
  immutable for status 200, 206 and 304 only; any other status, including a
  missing file, stays `no-store`, so a request caught mid-deploy can never pin a
  404 in a browser cache. An `add_header` inside a location drops every
  inherited header, so all security headers are repeated there with `always`.
- Send every 404 to the exported branded page with a server-level
  `error_page 404 /404.html;`. `location /` answers that redirect, so the
  response keeps status 404 and the server-level headers, including `no-store`.
- Keep `no-store` everywhere else, the final `location /`, the port-80 redirect
  and the HTTP webroot `/var/www/certbot` used for certificate renewal.

These are the changes to the Vite vhost, inside `http {}` and the 443 `server`:

```nginx
map $status $silicon_browser_static_cache {
    200     "public, max-age=31536000, immutable";
    206     "public, max-age=31536000, immutable";
    304     "public, max-age=31536000, immutable";
    default "no-store";
}

# 443 server, after the server-level add_header lines (CSP: style-src 'self' 'unsafe-inline')
error_page 404 /404.html;
location = /docs { try_files /docs/index.html =404; }
location = /auth/callback { try_files /index.html =404; }
location ~ ^/sessions/[^/]+/live$ { try_files /index.html =404; }
location ^~ /_next/static/ {
    try_files $uri =404;
    add_header Cache-Control $silicon_browser_static_cache always;
    add_header X-Content-Type-Options "nosniff" always;
    add_header Referrer-Policy "no-referrer" always;
    add_header Content-Security-Policy "<the same policy>" always;
}
location / { try_files $uri $uri/ =404; }
```

Run `nginx -t` on the candidate before replacing the live file, then switch the
root and the policy together in one `systemctl reload nginx`. Verify that the
three app routes return the new `index.html`, `/docs` and `/docs/` return
`docs/index.html`, every staged file returns 200 with its content type and one
copy of each header (immutable under `/_next/static/`, `no-store` elsewhere),
and that misses such as `/api/v1/me`, `/auth/callback/` and a missing
`/_next/static/` file return 404 with the branded page and `no-store`. To roll
back, restore the previous vhost file, which brings back the old root and policy
together, and reload; tabs opened on the Next release must then be reloaded.
Preserve the API vhost. Production no longer depends on the former Vercel
project; `vercel.json` only drives preview builds and the local preview.

Requests omit cookies and refuse redirects. Sign-ins are saved in
origin-scoped localStorage shared across tabs, so they survive reloads and
browser restarts until sign-out, expiry or revocation. Callback codes and live
invitation secrets are removed from URLs by `boot.js` before any framework code
runs.

For local checks:

```sh
SB_ORIGIN=http://127.0.0.1:8092 PORT=8091 cargo run -p silicon-browser-backend
SB_PUBLIC_BACKEND_URL=http://127.0.0.1:8091 npx --yes pnpm@10.33.0 --dir frontend build
npx --yes pnpm@10.33.0 --dir frontend preview
```

The preview (`dev.mjs`) serves `frontend/out` on `http://127.0.0.1:8092` with
the routes and headers from `vercel.json`, which mirror nginx, including the
branded 404. With the same local-API build,
`npx --yes pnpm@10.33.0 --dir frontend e2e` runs the end-to-end suite in
headless Google Chrome against an in-memory API mock, so no backend is needed;
set `CHROME_PATH` if Chrome is not installed in the default location. Rebuild
without the override before deploying.

Register the corresponding local Accounts callback URI if exercising hosted
sign-in. [Frontend setup](../frontend/README.md) describes the UI and tests.

## Release

Silicon Apps hosts CLI packages and updates, not the API/frontend. Follow
[Apps distribution](APPS.md) to build native packages and publish Browser 3.0.0.
Do not release a CLI against an API still running the old auth contract.
Keep secrets, backups and operational data outside packages. Run an actual
restore check and a provider session smoke test before production cutover.
Automatic Briefcase OBO delivery is retired; see [recording status](BRIEFCASE_INTEGRATION.md).

For the coordinated existing-Accounts UUID upgrade, use [the offline cutover runbook](ACCOUNT_UUID_CUTOVER.md). It does not adopt the preserved IAM database.
