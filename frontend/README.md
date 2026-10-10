# Silicon Browser frontend

SolidJS and TypeScript workspace built with Vite and deployed as static files on Vercel. The Rust API runs separately on AWS.

## Sign in with Silicon Accounts

Carbons select **Continue as Carbon**. The backend creates a state-bound PKCE attempt and returns its Accounts authorization URL. The browser opens `https://accounts.teamofsilicons.com/authorize`; Accounts returns a single-use `sac_` code and state to the registered, fixed `/auth/callback` URI. State contains the backend attempt UUID and a random nonce. The callback verifies its popup opener, origin, and attempt before handing the code back. The opener posts `{state, code}` to `/api/v1/auth/attempts/{id}/exchange`; only the backend holds the PKCE verifier and app secret. Full-page fallback matches the callback to its original attempt in sessionStorage.

Silicons select **Continue as Silicon** and paste the `slt_` token from:

```sh
silicon-accounts login --app browser
```

The frontend sends the token directly to `/api/v1/auth/exchange`. A token works once, expires after two minutes, and is bound to Browser. Browser never receives the Silicon's STK.

The API returns a JWT access token, rotating `sar_` refresh token, stable Accounts UUID, and optional service grants. Requests authenticate with the bearer token. Account handles can change; saved sign-ins are keyed by UUID. Signed-in accounts persist in origin-scoped localStorage across tabs and browser restarts until their credentials expire, are revoked, or the account signs out. Existing tab-scoped Accounts sessions and saved `browser` context keys migrate automatically without discarding valid tokens or pending refresh claims. Signing out removes that account across tabs.

Native Web Locks serialize refresh across tabs. Each waiting tab rereads the saved pair and adopts an already rotated token. The old token is marked consumed before submission and the new pair is saved on success; pending rotation keeps the workspace visible. A failed or interrupted refresh ends the local sign-in, because retrying a spent Accounts refresh token revokes its entire sign-in. API mutations are replayed only after an explicit pre-handler authentication rejection.

Live invitation grants and sign-in codes are removed from the displayed URL before rendering or making API calls. Live viewers connect directly to their returned HTTPS provider URL. Legacy testing or organization-specific invitations are rejected with instructions to obtain a new invitation.

Profiles, sessions, live handoff, command logs, recordings, and usage remain available. Session creation runs directly from the session form.

## Build and run

Use Node 24:

```sh
pnpm --dir frontend install --frozen-lockfile
pnpm --dir frontend test
pnpm --dir frontend run build
```

The package manager is pinned to pnpm 10.33.0 in `package.json`. Use that version for a frozen install and build. The output in `frontend/dist` is static and the API is called directly, without cookies or redirect following. Hosting must serve callback/live-route rewrites, immutable hashed assets, uncached HTML, and the security headers in `vercel.json`. See the repository deployment guide for the current production host.

`SB_PUBLIC_BACKEND_URL` is the only frontend configuration. It defaults to `https://backend.browser.teamofsilicons.com` and may be set to `http://127.0.0.1:8091` for local development. Visitor URLs cannot change it. Keep app secrets and provider credentials on the backend.

For a local backend with `SB_ORIGIN=http://127.0.0.1:8092`:

```sh
SB_PUBLIC_BACKEND_URL=http://127.0.0.1:8091 pnpm --dir frontend run build
node frontend/dev.mjs
```

The static preview binds loopback on 8092 and serves the production headers/routes. For HMR, use `SB_PUBLIC_BACKEND_URL=http://127.0.0.1:8091 pnpm --dir frontend run dev`. Rebuild without the override before deployment.

The `/docs/` page is static HTML. `publish-docs.mjs` also copies the repository Markdown guides under `/docs/*.md`; build from a checkout containing those sources.
