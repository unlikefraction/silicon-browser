# Changelog

## 5.0.0

This is a breaking release. Deploy callers and receivers with the matching IAM
backend and require explicit feature consent before delegated actions.

- Replace single-use OBO proof exchanges with separate endpoint consent and reusable access/refresh tokens. Use `authorize`, `authorization`, `consent`, `decide`, `exchange_code`, `refresh`, `verify`, `delegate`, `grants` and `revoke` on `client.obo()`.
- Verify current endpoint authority without consuming the access token. A reviewed dependency chain shares the same access token; each receiver verifies its own endpoint and selected account/organization. Never inherit another account's login disclosures.
- Limit each application login to one account and organization. Ordinary login consent covers IAM scopes and cannot be converted into an OBO grant. Existing unscoped application sessions must log in again.
- Add ATA endpoint discovery, token exchange and verification, plus actor-bound Honeycomb verification management. ATA credentials never provide user or OBO authority.
- Add optional-phone Carbon signup, Google/Apple provider signup, independent Silicon enrollment and custodian approval, Silicon invitations and custody controls, and directory visibility configuration.
- Preserve immutable callback/state binding, idempotent token rotation, testing-plane isolation and revocation checks across the new typed contracts.

## 1.9.0

Add public project links and explicit GitHub bug reporting with optional PR validation. Add installation-root-aware binary updates. Add optional Space Station diagnostics, sanitized request context, request correlation IDs, and propagated telemetry opt-out.

## 1.8.0

- Add application-secret-only test selection and verified IAM environment metadata. App selectors remain restricted to the selected application’s existing OAuth and directory authority.

## 1.7.0

- Added authenticated test configuration inspection with `applications().testing_context()`.
- Application environment listing now accepts a status filter and reports lifecycle ownership, state, version, and recovery deadline.
- Existing environment lifecycle methods accept the creating production application credential.

## 1.6.0

- Added `bundles().availability(org_id)` for derived bundle configuration availability.
- Added organization filtering before pagination through `applications().list_for_organization` and `bundles().list_for_organization`.
- Added `bundles().list_page` and exposed the existing bundle response's `page` metadata.
- Documented bundle logo URLs and the distinct preserve, replace, and clear patch values.
- Login history preserves authorized events when directory permissions hide an actor's public identifier.
