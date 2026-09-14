# Silicon Browser frontend

A minimal SolidJS and TypeScript workspace, built with Vite and deployed as static files on Vercel. It follows Silicon IAM's IBM Plex typography, gray navigation rail, white workspace and blue actions. The Rust API runs separately on AWS.

## Use an IAM testing environment

The [public documentation](https://browser.teamofsilicons.com/docs/#testing)
provides CLI, website, and direct API instructions for IAM testing environments.

1. Select **Testing environment** on the welcome screen or in the workspace header.
2. Enter the Browser test app's `ask_` secret. IAM verifies it and supplies the environment UUID; its root key is optional.
3. Enter an existing IAM test actor ID (`alice` or `worker:tos`), or a test `oac_` short-lived token, and select its organization by entering the organization ID.
4. To create browser sessions or save recordings, enter the **Briefcase test environment key** using the imported Briefcase IAM app secret from this same environment. It is optional for sign-in and metadata use, but required for browser sessions because Browser requires recording storage.
5. Select **Enter test mode**. The badge shows the verified environment name and UUID. In **Settings**, enable recording access using the same test actor or a fresh test token. Test authorization uses a masked form in this page.
6. Select **Exit test mode** to return to your production workspace. Reloading also clears the test sign-in and restores any saved production sign-in.

To obtain a short-lived token instead of using an existing actor ID:

```sh
iam --test <environment-uuid> login --app-id 'tos>browser' --grant-org <org> -o json
```

Use the returned `slt`. The optional IAM root key contains exactly 32 ASCII letters or digits. The
Briefcase field instead contains its imported app secret (`ask_` plus 43 base64url
characters) from the same IAM world. The app secret and all test credentials stay
in memory in this tab; they never enter localStorage or sessionStorage. Inputs
clear on submit, cancellation, or switching. Failed enrollment preserves the
current workspace and production sign-in. Missing or rejected test credentials
cannot route requests to production.

A shared test live link carries only its environment UUID and one-use grant in
the URL fragment. Opening it in a new tab first asks you to enroll that exact
testing environment. The URL fragment is cleared immediately, and the grant
cannot be redeemed against production or another test environment.

Test requests use the normal API and real, metered browser/search providers.
Browser data is isolated by IAM environment UUID and clean generation; rotating
an app secret preserves that generation's data. Browser sessions require the
paired Briefcase test configuration and recording authorization. No production
Briefcase fallback is permitted. See [CLI testing](../README.md#use-iam-testing-environments)
for equivalent terminal instructions and backend storage requirements.

## Build and run

Use Node 24:

```sh
npm ci --prefix frontend
npm test --prefix frontend
npm run build --prefix frontend
```

Set the Vercel project Root Directory to `frontend`, Framework Preset to Vite and Node to `24.x`. `vercel.json` supplies the build command, `dist` output, security headers and static rewrites for `/auth/callback` and `/sessions/:id/live`. There is no API proxy or serverless function. Hashed JavaScript, styles and fonts are hosted with the frontend and cached immutably; HTML is never cached.

The `/docs/` landing page is static HTML. The same build publishes the repository's Markdown under `/docs/*.md`. Deploy from the repository root so `publish-docs.mjs` can read those sources outside the frontend directory; no Markdown renderer or documentation service is required.

The public API origin defaults to `https://backend.browser.teamofsilicons.com`. On the backend, set `SB_ORIGIN=https://browser.teamofsilicons.com`. Requests go directly to that API using explicit bearer and organization headers, omit cookies and refuse redirects. CORS must allow Authorization, Content-Type, X-Org-Id, x-sb-test-app-secret, x-testing-environment-key, and x-sb-test-briefcase-key, and expose `x-sb-auth-rejected` for one safe pre-handler authentication retry. Test context verification posts to `/api/v1/testing/context`; all later test calls use `/testing/<verified-environment-uuid>/api/v1` with test headers. Only authentication, profile/session management, metadata and logs use the API. The live iframe connects straight to its returned HTTPS viewer URL.

Production sign-in opens the canonical IAM login in a popup, with application `tos>browser`; organization selection is handled by IAM after authentication. Its one-use callback is accepted only from the exact popup window and frontend origin with the initiating nonce. If the sign-in window loses its opener, a same-origin BroadcastChannel keyed by that random nonce completes the handoff. The callback query and live-link fragment are removed immediately. The production interactive access/refresh pair is saved in origin-scoped sessionStorage, keyed by API origin, so the same tab restores its identity after reload. Successful renewal replaces the saved pair before another authenticated request; sign-out or a terminal renewal rejection clears it. Temporary network errors preserve the saved session. Codes and live grants remain memory-only. This is tab-session persistence, not a persistent login across browser restarts or synchronized sign-in across tabs. Production recording delivery uses a second IAM sign-in so its independent background authorization survives closing the tab. It can be disabled in Settings. Test mode uses memory-only clients and test actors/tokens for both sign-in and recording authorization; it never opens production IAM popup authentication.

For a local backend on port 8091, set its `SB_ORIGIN=http://127.0.0.1:8092`, then:

```sh
SB_PUBLIC_BACKEND_URL=http://127.0.0.1:8091 npm run build --prefix frontend
node frontend/dev.mjs
```

The preview server binds loopback on 8092, serves the same headers/routes as Vercel, and cannot serve dotfiles or proxy APIs. For development with Vite HMR, run `SB_PUBLIC_BACKEND_URL=http://127.0.0.1:8091 npm run dev --prefix frontend`; production security-header testing should use the static preview instead. Rebuild without the override before deployment.

`SB_PUBLIC_BACKEND_URL` is the only frontend configuration and must be the dedicated production origin or `http://127.0.0.1:8091`. Visitor URLs cannot change it. Never put IAM app secrets, environment keys or provider credentials in frontend configuration.
