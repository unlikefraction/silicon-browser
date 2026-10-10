# Silicon Browser frontend

SolidJS and TypeScript workspace built with Vite and deployed as static files on Vercel. The Rust API runs separately on AWS.

## Sign in with Silicon Accounts

Carbons select **Continue as Carbon**. The backend creates a state-bound PKCE attempt and returns its Accounts authorization URL. The browser opens `https://accounts.teamofsilicons.com/authorize`; Accounts returns a single-use `sac_` code and state to the registered, fixed `/auth/callback` URI. State contains the backend attempt UUID and a random nonce. The callback verifies its popup opener, origin, and attempt before handing the code back. The opener posts `{state, code}` to `/api/v1/auth/attempts/{id}/exchange`; only the backend holds the PKCE verifier and app secret. Full-page fallback matches the callback to its original attempt in sessionStorage.

Silicons select **Continue as Silicon** and paste the `slt_` token from:

```sh
silicon-accounts login --app browser
```

The frontend sends the token directly to `/api/v1/auth/exchange`. A token works once, expires after two minutes, and is bound to Browser. Browser never receives the Silicon's STK.

The API returns a JWT access token, rotating `sar_` refresh token, stable Accounts UUID, and the single `browser` workspace. Account handles can change; saved sign-ins are keyed by UUID. Each signed-in account has its own origin-scoped sessionStorage entry in this tab. Legacy saved sessions require a new sign-in. Browser uses one workspace for all signed-in accounts.

A refresh runs once per account at a time. The old token is removed from persisted storage before submission and the new pair is saved on success. A failed or interrupted refresh ends the local sign-in, because retrying a spent Accounts refresh token revokes its entire sign-in. API mutations are replayed only after an explicit pre-handler authentication rejection.

Live invitation grants and sign-in codes are removed from the displayed URL before rendering or making API calls. Live viewers connect directly to their returned HTTPS provider URL. Legacy testing or organization-specific invitations are rejected with instructions to obtain a new invitation.

Profiles, sessions, live handoff, command logs, recordings, and usage remain available. Session creation runs directly from the session form.

## Build and run

Use Node 24:

```sh
pnpm --dir frontend install --frozen-lockfile
pnpm --dir frontend test
pnpm --dir frontend run build
```

The package manager is pinned to pnpm 12.5.1 in `package.json`. Set the Vercel Root Directory to `frontend`, Framework Preset to Vite, and Node to `24.x`. `vercel.json` supplies pinned pnpm install/build commands, the output directory, security headers, and static callback/live-route rewrites. Keep `ENABLE_EXPERIMENTAL_COREPACK=1` enabled in the Vercel project. The API is called directly, without cookies or redirect following. Hashed assets are cached immutably; HTML is never cached.

`SB_PUBLIC_BACKEND_URL` is the only frontend configuration. It defaults to `https://backend.browser.teamofsilicons.com` and may be set to `http://127.0.0.1:8091` for local development. Visitor URLs cannot change it. Keep app secrets and provider credentials on the backend.

For a local backend with `SB_ORIGIN=http://127.0.0.1:8092`:

```sh
SB_PUBLIC_BACKEND_URL=http://127.0.0.1:8091 pnpm --dir frontend run build
node frontend/dev.mjs
```

The static preview binds loopback on 8092 and serves the production headers/routes. For HMR, use `SB_PUBLIC_BACKEND_URL=http://127.0.0.1:8091 pnpm --dir frontend run dev`. Rebuild without the override before deployment.

The `/docs/` page is static HTML. `publish-docs.mjs` also copies the repository Markdown guides under `/docs/*.md`; build from a checkout containing those sources.
