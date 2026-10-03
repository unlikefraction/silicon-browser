# Coordinated IAM 5 release

This release separates feature OBO consent from ordinary login and retires implicit
login-derived delegated authority. Existing users keep their ordinary sessions
and authorize delegated features when needed.

Use the exact IAM client version and checksum in `Cargo.toml` and `Cargo.lock`.
The earlier vendored client remains historical provenance, not the runtime
dependency selection.

Production rollout is coordinated with IAM 5 and the receiving providers. Build
artifacts are candidates until integration checks and database backups pass.

The website also binds Carbon/Silicon login choices to expiring backend attempts.
State and token replay checks precede session creation, popup completion waits for
backend success, and blocked popups use a full-page fallback. Recording permission
revocation and changed consent graphs return an explicit reauthorization state.

The 0.4.2 release adds Briefcase's separate upload-status root, durable logical
operation and upload IDs, status-first recovery, and current commit authority
after byte transfer. Completed receipts remain intact; legacy attempted uploads
need explicit reconciliation. Existing three-root recording grants require fresh
feature approval. The accepted Honeycomb configuration must include all four roots
before enabling new recording consent. See [recording delivery](docs/BRIEFCASE_INTEGRATION.md)
for the contract and outstanding shared-testing participant blocker.

Published 0.4.0 and 0.4.1 CLI artifacts remain immutable. Publish new bytes under
0.4.2 and record the deployed source revision and verification independently.

Deployment and publication evidence is recorded in [the 0.4.2 release report](docs/operations/browser-0.4.2-2026-10-04.md).
