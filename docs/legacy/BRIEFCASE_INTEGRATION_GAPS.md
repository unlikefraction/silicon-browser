# Briefcase integration: release gates and limits

The October 2026 local cutover implements separate feature consent, encrypted
OBO credentials, selected provider account/org, and reservation/capability/commit
publication. See [the current contract](BRIEFCASE_INTEGRATION.md). Legacy raw
proof upload and login-derived storage authority are retired.

The old 60-second body-proof deadline no longer applies: bytes transfer with a
reservation capability and commit independently checks current authority. Stable
session/artifact operation IDs reconcile uncertain commits without deliberately
creating an extra version. Configured size and transfer timeouts still bound
local staging and network work; this is not unlimited storage support.

Historical sessions without a verified initiating principal/membership remain
unadopted. Already attempted artifacts retain their approved destination; changing
consent to another org cannot silently redirect them. Permanent invalid/unavailable
or changed source bytes remain terminal. Local Hide does not delete a Briefcase
file. Cooperative logs cannot promise to include offline/direct activity.

Local tests cover changed-code retries, encryption, refresh uncertainty/concurrency,
wrong origin account/world, selected destination, reservation/transfer/commit
replay, and revoked permission before a paid start. The schema-upgrade regression
also proves that legacy families become revoke-only without synthesizing consent
or losing existing receipts and ownership. Synthetic desktop/mobile checks cover
login preservation and explicit recovery without automatically retrying paid work.
A coordinated deployed IAM → Browser → Briefcase walkthrough remains a release
gate. Earlier [live evidence](AUTOMATIC_RECORDING_LIVE_RETEST.md) used the preceding
protocol and cannot be relabeled as verification of this cutover. No production
push or deployment is part of this local change.
