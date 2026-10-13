# Silicon Browser frontend

React and TypeScript workspace built with Next.js (Pages Router, `output: 'export'`, `next build --webpack`) and the Silicon UI component library. The static export is served by nginx on the AWS host; the Rust API runs separately. See `UI.md` for the component setup.

## Sign in with Silicon Accounts

Carbons select **Continue as Carbon**. The backend creates a state-bound PKCE attempt and returns its Accounts authorization URL. The browser opens `https://accounts.teamofsilicons.com/authorize`; Accounts returns a single-use `sac_` code and state to the registered, fixed `/auth/callback` URI. State contains the backend attempt UUID and a random nonce. The callback verifies its popup opener, origin, and attempt before handing the code back. The opener posts `{state, code}` to `/api/v1/auth/attempts/{id}/exchange`; only the backend holds the PKCE verifier and app secret. Full-page fallback matches the callback to its original attempt in sessionStorage.

Silicons select **Continue as Silicon** and paste the `slt_` token from:

```sh
silicon-accounts login --app browser
```

The frontend sends the token directly to `/api/v1/auth/exchange`. A token works once, expires after two minutes, and is bound to Browser. Browser never receives the Silicon's STK.

The API returns a JWT access token, rotating `sar_` refresh token, stable Accounts UUID, and optional service grants. Requests authenticate with the bearer token. Account handles can change; saved sign-ins are keyed by UUID. Signed-in accounts persist in origin-scoped localStorage across tabs and browser restarts until their credentials expire, are revoked, or the account signs out. Existing tab-scoped Accounts sessions and saved `browser` context keys migrate automatically without discarding valid tokens or pending refresh claims. Signing out removes that account across tabs.

Native Web Locks serialize refresh across tabs. Each waiting tab rereads the saved pair and adopts an already rotated token. The old token is marked consumed before submission and the new pair is saved on success; pending rotation keeps the workspace visible. A failed or interrupted refresh ends the local sign-in, because retrying a spent Accounts refresh token revokes its entire sign-in. API mutations are replayed only after an explicit pre-handler authentication rejection.

Live invitation grants and sign-in codes are removed from the displayed URL by `public/boot.js`, a synchronous script that runs before any framework code, rendering or API call. Live viewers connect directly to their returned HTTPS provider URL. Legacy testing or organization-specific invitations are rejected with instructions to obtain a new invitation.

Profiles, sessions, live handoff, recordings, and usage remain available. Session creation runs directly from the session form.

## Build and run

Use Node 24:

```sh
pnpm --dir frontend install --frozen-lockfile
pnpm --dir frontend test
pnpm --dir frontend run build
```

The package manager is pinned to pnpm 10.33.0 in `package.json`; run it as `npx --yes pnpm@10.33.0` for a frozen install and build. `build` runs `tsc`, `next build --webpack`, `publish-docs.mjs`, and `scripts/check-export.mjs`. The last step fails if the export contains an executable inline script, an inline event handler, or a source map.

The output in `frontend/out` is static, and the API is called directly, without cookies or redirect following. `/`, `/auth/callback`, and `/sessions/<id>/live` must all serve `out/index.html`. `/docs` and `/docs/` serve `out/docs/index.html`. Hosting must also serve uncached HTML, immutable `/_next/static/*` files, and the security headers in `vercel.json`, which production nginx mirrors. See the repository deployment guide for the current production host.

`SB_PUBLIC_BACKEND_URL` is the only frontend configuration. It defaults to `https://backend.browser.teamofsilicons.com` and may be set to `http://127.0.0.1:8091` for local development. Visitor URLs cannot change it. Keep app secrets and provider credentials on the backend.

For a local backend with `SB_ORIGIN=http://127.0.0.1:8092`:

```sh
SB_PUBLIC_BACKEND_URL=http://127.0.0.1:8091 pnpm --dir frontend run build
node frontend/dev.mjs
```

The static preview binds loopback on 8092 and serves the production headers and routes. For hot reload, use `SB_PUBLIC_BACKEND_URL=http://127.0.0.1:8091 pnpm --dir frontend run dev` (`next dev`, without the CSP). `next dev` serves only `/`: use the static preview for `/auth/callback`, live links and `/docs`.

`pnpm --dir frontend e2e` (`scripts/e2e.mjs`, playwright-core) runs headless Google Chrome against a build made with `SB_PUBLIC_BACKEND_URL=http://127.0.0.1:8091`, served by `dev.mjs` with the production headers. An in-memory mock answers every API call, so no backend is needed. The scenarios cover both sign-in paths (Silicon token and the Carbon popup, including cancel), every workspace screen and its exact request bodies, account switching, theme and sign-out, the callback and live-invitation URL contracts (secrets stripped at load and never restored by Back or Forward; the callback never restores a session), the exact live iframe attributes, the mobile drawer, and the 404 page. Any CSP violation, uncaught page error, or unexpected console error fails the run. It also walks every screen at 1440, 1024, 768 and 390 px in light and dark and fails on sideways page scrolling; set `E2E_SCREENSHOTS=<dir>` to save those screens, and `E2E_ONLY=<text>` to run matching scenarios. Set `CHROME_PATH` if Google Chrome is not installed in the default location. Rebuild without the override before deployment.

The `/docs/` page is static HTML. `publish-docs.mjs` also copies the repository Markdown guides under `/docs/*.md`; build from a checkout containing those sources.
