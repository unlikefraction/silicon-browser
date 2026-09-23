# Changelog

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
