# Silicon Browser frontend

A minimal SolidJS and TypeScript workspace, built with Vite and deployed as static files on Vercel. It follows Silicon IAM's IBM Plex typography, gray navigation rail, white workspace and blue actions. The Rust API runs separately on AWS.

Use Node 24:

```sh
npm ci --prefix frontend
npm test --prefix frontend
npm run build --prefix frontend
```

Set the Vercel project Root Directory to `frontend`, Framework Preset to Vite and Node to `24.x`. `vercel.json` supplies the build command, `dist` output, security headers and static rewrites for `/auth/callback` and `/sessions/:id/live`. There is no API proxy or serverless function. Hashed JavaScript, styles and fonts are hosted with the frontend and cached immutably; HTML is never cached.

The public API origin defaults to `https://backend.browser.teamofsilicons.com`. On the backend, set `SB_ORIGIN=https://browser.teamofsilicons.com`. Requests go directly to that API using explicit bearer and organization headers, omit cookies and refuse redirects. CORS must allow Authorization, Content-Type and X-Org-Id, and expose `x-sb-auth-rejected` for one safe pre-handler authentication retry. Only authentication, profile/session management, metadata and logs use the API. The live iframe connects straight to its returned HTTPS viewer URL.

Sign-in opens the canonical IAM login in a popup, with application `tos>browser` and the selected organization. Its one-use callback is accepted only from the exact popup window and frontend origin with the initiating nonce. If the sign-in window loses its opener, a same-origin BroadcastChannel keyed by that random nonce completes the handoff. The callback query and live-link fragment are removed immediately. The interactive access/refresh pair is saved in origin-scoped sessionStorage, keyed by API origin, so the same tab restores its identity after reload. Successful renewal replaces the saved pair before another authenticated request; sign-out or a terminal renewal rejection clears it. Temporary network errors preserve the saved session. Codes and live grants remain memory-only. This is tab-session persistence, not a persistent login across browser restarts or synchronized sign-in across tabs. Recording delivery uses a second IAM sign-in so its independent background authorization survives closing the tab. It can be disabled in Settings.

For a local backend on port 8091, set its `SB_ORIGIN=http://127.0.0.1:8092`, then:

```sh
SB_PUBLIC_BACKEND_URL=http://127.0.0.1:8091 npm run build --prefix frontend
node frontend/dev.mjs
```

The preview server binds loopback on 8092, serves the same headers/routes as Vercel, and cannot serve dotfiles or proxy APIs. For development with Vite HMR, run `SB_PUBLIC_BACKEND_URL=http://127.0.0.1:8091 npm run dev --prefix frontend`; production security-header testing should use the static preview instead. Rebuild without the override before deployment.

`SB_PUBLIC_BACKEND_URL` is the only frontend configuration and must be the dedicated production origin or `http://127.0.0.1:8091`. Visitor URLs cannot change it. Never put IAM app secrets, environment keys or provider credentials in frontend configuration.
