# Frontend verification — 6 September 2026

The standalone frontend is SolidJS + TypeScript with Vite. The workspace follows the deployed IAM interface's gray rail, white content area, blue controls and IBM Plex Sans/Mono typography. Browser provider names are absent from the normal user flows.

## Automated checks

- `npm test --prefix frontend`: 18 passing contracts covering authentication replay boundaries, concurrent refresh, sign-out races, identity/organization binding, native browser fetch receiver, nonce/source/origin callback checks, detached-opener handoff, safe URLs and command exports, approved API origins, static deep links and headers.
- `npm run build --prefix frontend`: TypeScript and production Vite build pass.
- `npm audit --prefix frontend --omit=dev --audit-level=high`: zero production dependency vulnerabilities reported.
- Production main JavaScript: 40.70 kB, 13.87 kB gzip. Static assets have immutable cache headers; HTML remains `no-store`.

## Real browser checks

Used a separate native browser session with the built static preview on `127.0.0.1:8092`, strict deployment headers and an isolated API fixture on `127.0.0.1:8091`. The fixture uses synthetic tokens and data; these checks do not establish production IAM token exchange, provider browser creation or real recording delivery.

Verified by clicking and filling the actual interface:

1. Login opens the real canonical IAM sign-in page with the selected organization and Browser application ID. Returning through a synthetic one-use callback signs in to the fixture API. The callback URL is stripped immediately.
2. A callback whose opener is absent completes through a same-origin BroadcastChannel keyed by the initiating random nonce. This found a gap that strict `window.opener.postMessage` alone did not handle.
3. Sessions render; a profile's name and access list can be edited and saved.
4. Starting a profile session submits its name, description and selected duration, then opens its details.
5. Live view loads the returned HTTPS URL directly in its iframe. No Browser API route proxies viewer content.
6. Ending a session submits the closing note and removes active controls. Command logs render safely quoted CLI commands.
7. Recordings and usage pages render. Recording authorization can be disabled, then re-enabled through a separate fresh popup callback.
8. Mobile checks at 390 × 844 show no document overflow. Both localStorage and sessionStorage remain empty; no credentials are persisted by the frontend.
9. No JavaScript errors appeared in the completed journey. A browser-native fetch receiver error discovered during the first run was fixed and covered by a regression contract.

The fixture servers and test browser were closed. The last build was restored to the production backend origin. Public deployment and real account/provider verification are recorded separately by the release owner.
