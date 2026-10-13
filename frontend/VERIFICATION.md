# Frontend verification

## Next.js and Silicon UI — 13 October 2026

The website is now a Next.js static export (Pages Router, `output: 'export'`) that uses Silicon UI components installed verbatim from the registry. Run the checks with the pinned package manager, `npx --yes pnpm@10.33.0`, from the repository root.

`npx --yes pnpm@10.33.0 --dir frontend test` passes 49 tests covering:

- Accounts authorization URL validation and PKCE binding.
- Exact popup origin, source, state, attempt, and authorization-code matching.
- Callback code and live-grant removal from the entry URL, and rejection of retired live-invitation scopes.
- Stable account UUIDs, saved account switching, and display-handle changes.
- Shared browser login across tabs, one-use refresh claims, and safe account switching during rotation.
- Migration to the organization-free account contract while keeping the private cache readable by already-open tabs.
- One refresh per account and no replay after ambiguous failures.
- No cookies or redirect following in API calls; mutation replay only on a pre-handler auth rejection.
- The build contract: backend origin allow-list, a static export with no server routes, headers, or source maps, the `vercel.json` routes and strict policy, `boot.js` stripping one-use secrets before the app starts, and the static preview's deep links, immutable Next assets, docs, licence, and 404s.
- Recording status and provider replay availability without retired delivery actions.

`npx --yes pnpm@10.33.0 --dir frontend build` runs, in order:

1. `tsc --noEmit` (strict TypeScript).
2. `next build --webpack`, which writes the static export to `frontend/out`.
3. `publish-docs.mjs`, which copies the repository Markdown guides to `out/docs/`.
4. `scripts/check-export.mjs`, which fails on a missing required file, an executable inline script (only `__NEXT_DATA__` as `application/json` may be inline), an inline event handler, a `javascript:` URL, or a source map, and requires `/boot.js` to be the first script in `index.html`, synchronous and classic.

Installation uses the frozen pnpm 10.33.0 lockfile, matching CI. Telemetry is off (`NEXT_TELEMETRY_DISABLED=1`).

`npx --yes pnpm@10.33.0 --dir frontend e2e` runs the end-to-end suite in headless Google Chrome (playwright-core) against a build made with `SB_PUBLIC_BACKEND_URL=http://127.0.0.1:8091`. `dev.mjs` serves `out/` with the production headers, and an in-memory API mock answers every request, so no backend is needed. Any Content-Security-Policy violation, uncaught page error, or unexpected console error fails a scenario. The suite covers:

- Both sign-in paths (Silicon token and the Carbon popup, including cancel), account switching, theme, and sign-out.
- Every workspace screen and the exact request bodies it sends.
- The URL contracts: callback codes and live grants are stripped at load and never restored by Back or Forward, and the callback page never restores a saved session.
- The exact live iframe attributes, the mobile drawer, and the 404 page.
- A walk of every screen at 1440, 1024, 768, and 390 px in light and dark themes that fails on sideways page scrolling. `E2E_SCREENSHOTS=<dir>` saves the screens.

Set `CHROME_PATH` when Google Chrome is not in its default location. Rebuild without the override before deployment.

`node frontend/scripts/sync-silicon-ui.mjs --check` downloads every item in `src/components/silicon-ui/registry-lock.json`, verifies its sha256, and fails if any installed registry file differs byte for byte. It needs network access to `ui.teamofsilicons.com`.

CI runs the install, unit tests, the production build, the end-to-end suite on a local-API build, and the registry check.

These checks use synthetic access and refresh tokens. Live Accounts credentials, hosted Carbon sign-in, Silicon SLT exchange, provider browser creation, and recording delivery still require verification against configured services. The frontend build does not deploy the website; see the repository deployment guide.

## Accounts migration — 10 October 2026

`pnpm --dir frontend test` passes 48 tests covering:

- Accounts authorization URL validation and PKCE binding.
- Exact popup origin, source, state, attempt, and authorization-code matching.
- Callback credential removal and rejection of retired live-invitation scopes.
- Stable account UUIDs, saved account switching, and display-handle changes.
- Shared browser login across tabs, one-use refresh claims, and safe account switching during rotation.
- Migration to the organization-free account contract while keeping the private cache readable by already-open tabs.
- One refresh per account and no replay after ambiguous failures.
- No cookies or redirect following in API calls; mutation replay only on a pre-handler auth rejection.
- Static preview deep links, documentation routes, cache policy, CSP, and path restrictions.
- Recording status and provider replay availability without retired delivery actions.

`pnpm --dir frontend run build` passes TypeScript checking, Vite bundling, and Markdown documentation publication. Installation uses the frozen pnpm 10.33.0 lockfile, matching CI.

The official Silicon UI registry Button, motion tokens, foundation and MIT attribution are included. A local desktop Chrome check confirms the Carbon and Silicon controls render correctly and the Silicon control expands the token form. Mobile layout was not rechecked in this pass.

These checks use synthetic access/refresh tokens. Live Accounts credentials, hosted Carbon sign-in, Silicon SLT exchange, provider browser creation, and recording delivery still require verification against configured services. The frontend build does not deploy the website.
