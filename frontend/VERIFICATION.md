# Frontend verification

## Bound IAM 5 sign-in — 3 October 2026

Sign-in now starts a backend attempt with the selected account kind, then
completes that exact attempt with its random state and the one-use IAM token.
Popup messages require the original window, origin, attempt and state. The
popup waits for verified completion before closing. Blocked popups use the
same-tab callback; its pending record stores only the attempt and local return
path. One-use login tokens and live grants remain memory-only. Existing
interactive session persistence is unchanged.

The frontend contracts cover both account kinds, delayed popup completion,
exact callback acknowledgement, blocked-popup fallback, rejected state and
expired attempts, safe local return paths, and cancellation during exchange.
The package manager is pinned to pnpm 12.5.1 with the imported lockfile and
explicit permission for esbuild's install script. All 50 frontend tests and the
TypeScript/Vite production build passed locally (Node 26.9.0); deployment uses
Node 24.x. No live IAM account or paid browser session was created by these checks.


## Separate recording consent — 3 October 2026

The recording permission flow now starts an explicit IAM feature approval and
accepts its one-time code independently of login. The pending request uses a
stable retry key and remains in memory. Changing the account, organization, or
testing context invalidates the pending UI operation and ignores late responses.

Local checks passed:

- Frontend: 39 tests and the TypeScript/Vite production build.
- SDK and CLI: 34 client tests, 28 CLI unit tests, and 29 binary contracts; strict
  all-target Clippy for both packages.
- An isolated headless Chromium fixture at 1440 × 1000 and 390 × 844 verified
  that an invalid code preserves login, an organization switch clears pending
  approval, and a session rejection refreshes recording status. Completing
  approval did not retry or create a browser session. No horizontal overflow or
  uncaught JavaScript errors occurred. Mobile organization selectors have
  accessible names even when their text labels are hidden.

The fixture intercepted all API responses and blocked external requests. It
used synthetic credentials and created no paid browser sessions. Screenshots
were captured in `/tmp/browser-consent-qa/` (`1440-pending.png`,
`1440-complete.png`, `390-pending.png`, and `390-complete.png`). The fixture server
and headless browser were stopped after verification. This establishes local
UI behavior, not live IAM approval, metered-provider operation, deployment, or
real recording delivery.

## IAM testing additions — 14 September 2026

The current frontend adds the visible Testing environment selector, memory-only
test authentication, actor/token recording authorization, and environment-bound
live invitations. API regressions cover secret-only enrollment, production
session preservation, test refresh headers, cancellation by client closure,
invalid contexts, and refusal to route test requests or grants to production.

Validation uses the CLI: `npm test --prefix frontend` and
`npm run build --prefix frontend`. Browser UI automation was not run for these
changes. Deployment and live IAM/provider checks must be established separately.

## Historical verification — 6 September 2026

The standalone frontend is SolidJS + TypeScript with Vite. The workspace follows the deployed IAM interface's gray rail, white content area, blue controls and IBM Plex Sans/Mono typography. Browser provider names are absent from the normal user flows.

## Automated checks

- `npm test --prefix frontend`: 27 passing contracts covering authentication replay boundaries, concurrent refresh, sign-out races, identity/organization binding, native browser fetch receiver, nonce/source/origin callback checks, detached-opener handoff, safe URLs and command exports, approved API origins, static deep links and headers.
- `npm run build --prefix frontend`: TypeScript and production Vite build pass.
- `npm audit --prefix frontend --omit=dev --audit-level=high`: zero production dependency vulnerabilities reported.
- Production main JavaScript: 44.51 kB, 15.06 kB gzip. Static assets have immutable cache headers; HTML remains `no-store`.

## Real browser checks

Used a separate native browser session with the built static preview on `127.0.0.1:8092`, strict deployment headers and an isolated API fixture on `127.0.0.1:8091`. The fixture uses synthetic tokens and data; these checks do not establish production IAM token exchange, provider browser creation or real recording delivery.

Verified by clicking and filling the actual interface:

1. Login opens the real canonical IAM sign-in page with the Browser application ID; IAM handles organization consent. Returning through a synthetic one-use callback signs in to the fixture API. The callback URL is stripped immediately.
2. A callback whose opener is absent completes through a same-origin BroadcastChannel keyed by the initiating random nonce. This found a gap that strict `window.opener.postMessage` alone did not handle.
3. Sessions render; a profile's name and access list can be edited and saved.
4. Starting a profile session submits its name, description and selected duration, then opens its details.
5. Live view loads the returned HTTPS URL directly in its iframe. No Browser API route proxies viewer content.
6. Ending a session submits the closing note and removes active controls. Command logs render safely quoted CLI commands.
7. Recordings and usage pages render. Recording authorization can be disabled, then re-enabled through a separate fresh popup callback.
8. Mobile checks at 390 × 844 show no document overflow. Origin-scoped sessionStorage restores the interactive session after reload; localStorage remains empty and credentials do not survive a browser restart.
9. No JavaScript errors appeared in the completed journey. A browser-native fetch receiver error discovered during the first run was fixed and covered by a regression contract.

The fixture servers and test browser were closed. The last build was restored to the production backend origin. Public deployment and real account/provider verification are recorded separately by the release owner.
