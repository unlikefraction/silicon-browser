# Frontend verification

## Accounts migration — 10 October 2026

`npm test --prefix frontend` passes 34 tests covering:

- Accounts authorization URL validation and PKCE binding.
- Exact popup origin, source, state, attempt, and authorization-code matching.
- Callback credential removal and rejection of retired live-invitation scopes.
- Stable account UUIDs, saved account switching, and display-handle changes.
- One refresh per account, clearing persisted refresh tokens before submission, and no replay after ambiguous failures.
- No cookies or redirect following in API calls; mutation replay only on a pre-handler auth rejection.
- Static preview deep links, documentation routes, cache policy, CSP, and path restrictions.
- Recording status and available retry actions.

`npm run build --prefix frontend` passes TypeScript checking, Vite bundling, and Markdown documentation publication.

These checks use synthetic access/refresh tokens. Live Accounts credentials, hosted Carbon sign-in, Silicon SLT exchange, provider browser creation, and recording delivery still require verification against configured services. The frontend build does not deploy the website.
