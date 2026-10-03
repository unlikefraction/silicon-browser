# Coordinated IAM 5 release

This release separates feature OBO consent from ordinary login, refreshes the
vendored IAM client to the exact 5.0.0 release commit, and retires implicit
login-derived delegated authority. Existing users keep their ordinary sessions
and authorize delegated features when needed.

IAM client source: `f1e9c4768029aacabe337ca41be52e05023d1631`.
See `vendor/silicon-iam-client/VENDORED.md` for package provenance.

Production rollout is coordinated with IAM 5 and the receiving providers. Build
artifacts are candidates until integration checks and database backups pass.

The website also binds Carbon/Silicon login choices to expiring backend attempts.
State and token replay checks precede session creation, popup completion waits for
backend success, and blocked popups use a full-page fallback. Recording permission
revocation and changed consent graphs return an explicit reauthorization state.

The public 0.4.0 CLI artifacts remain immutable. Backend and website fixes are
tracked by their deployment source revision; they preserve the CLI exchange API.
