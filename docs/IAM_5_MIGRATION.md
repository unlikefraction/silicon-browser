# IAM 5 and Honeycomb 0.6 migration

This checkout implements the IAM 5 protocol from the
[migration guide](https://docs.iam.teamofsilicons.com/migrating-to-iam-5/), using
the pinned `silicon-iam-client = 5.2.1` SDK and
[Briefcase 3 delegated upload contract](https://docs.briefcase.teamofsilicons.com/api/delegated-uploads/).
The documentation was read on 2026-10-03. It advertises live platform releases,
but **this Browser migration has not been deployed or tested against them**.
No Honeycomb catalog, credentials, test environment or release was changed.

## Code and configuration changes

- Login binds the Carbon/Silicon choice and unpredictable state to a backend
  attempt, verifies the exchanged principal kind, and selects exactly one account
  and organization. The SPA callback passes only the one-use SLT, state and attempt ID to its exact
  app-origin opener, which verifies the popup source and waits for the backend
  exchange before completing sign-in. A full-page fallback remains.
- Frontend contexts retain distinct sessions and isolate requests/results by
  account, organization and environment. Selecting another context does not
  rewrite the organization on an existing bearer.
- Recording access starts a separate IAM OBO request with the current Browser
  subject token. Approval yields one backend-owned OBO family per Briefcase root.
  One-use code exchange and rotating refresh retain their original mutation keys
  after uncertain responses. Ordinary login/logout do not stand in for OBO consent.
- Recording uploads use reserve, capability-only transfer, current-authority
  commit and status reconciliation. Persisted operation identity and immutable
  manifests survive worker retries. Receipt lookup uses the committed entry UUID
  to retain the provider's real metadata and authenticated link.
- Honeycomb metadata requests `briefcase.uploads.reserve`,
  `briefcase.uploads.commit`, `briefcase.uploads.status` and
  `briefcase.entries.list`. Browser exposes no OBO/ATA receiver and requires no
  new incoming endpoint graph or ATA credential.

The retired IAM `/obo-access/exchanges`, `/chained-exchanges` and `/verify`
proof routes cannot be used as compatibility fallback. Briefcase's old
`POST /api/v1/obo/files` returns 410; changing its token header is insufficient.
Old recording authorization families cannot be converted into OBO grants.
Users must approve recording access again; old delivery work without a verified
provider destination must not be adopted silently.

The database migration marks unfinished legacy artifacts with already-bound bytes,
including failed retries, as `legacy_upload_reconciliation_required`. Their earlier single-use upload may
have committed without a receipt, so it cannot safely acquire a new operation ID
and retry automatically. Reconcile those artifacts against the original recipient
before rollout; preserve completed receipts and investigate each unknown outcome.

## Consent and command flow

Ordinary `browser setup` performs login/controller setup and checks status; it
never approves recording access. Start recording consent when the feature is used:

```sh
browser recording authorize --request-key REQUEST_UUID
browser recording authorize --authorization-id AUTHORIZATION_UUID --code-stdin
browser recording authorization-status
```

The first command returns an IAM review URL. The second reads the one-use approval
code from stdin. Preserve the request UUID and completion input after an uncertain
response. Decline or closing IAM keeps the pending action and ordinary login.
`browser recording disconnect` explicitly disables delivery. It is separate from
ordinary sign-out.

All four Briefcase roots must select the same provider account, organization,
testing environment and cleaning generation. That destination may differ from
the Browser login context. Existing jobs keep their original destination through
refresh, retries and reauthorization. Test credentials are selected from the
verified recipient context; they cannot fall back to production.

## Local checks and release gate

The new protocol is covered with local HTTP/SQLite fixtures and frontend/CLI
checks. These validate implementation behavior, not platform deployment,
accepted catalog state, provider reviews, quotas or live webhook delivery.

Before enabling a release, complete this integration check in an isolated testing
environment with refreshed Browser and Briefcase imports:

- Verify running IAM/Briefcase contracts and accepted Honeycomb endpoint IDs,
  paths, disclosures and approvals. Updating production configuration does not
  update existing testing snapshots automatically.
- Complete Carbon and Silicon login, popup/fallback and CLI approval. Reject wrong
  principal kind, callback state/attempt, revoked membership and mismatched org.
  Check decline and blocked/closed popup behavior without losing pending work.
- Exercise two Browser account/organization contexts and a different selected
  Briefcase destination; confirm old jobs never follow a newly selected account.
- Create/stop one real session for each identity kind. Confirm MP4 delivery for
  both and command JSONL for Silicon; verify the recipient's size, digest and link.
- Lose reserve, transfer and commit responses deliberately. Reconcile the same
  operation, refresh after byte transfer, and verify one published file version.
- Revoke consent, change the endpoint graph, remove membership and clean the test
  environment; verify rejection without production fallback. Exercise signed,
  duplicate and stale webhook deliveries.
- Build/check all six native CLI targets, publish an immutable development package,
  and verify installation and the complete CLI workflow. Coordinate a new package
  version before production publication; do not overwrite earlier release bytes.

Only after those checks should callers, receiver deployments and accepted catalog
changes be enabled together. A permission-expanding release remains private until
its required reviews finish. See [Silicon Apps distribution](APPS.md) and
[recording delivery](BRIEFCASE_INTEGRATION.md).

References: [IAM OBO HTTP contract](https://docs.iam.teamofsilicons.com/api/obo/),
[Briefcase OBO reference](https://docs.briefcase.teamofsilicons.com/obo/),
[Honeycomb scope configuration](https://docs.honeycomb.teamofsilicons.com/scopes-and-obo/),
[Honeycomb upload channels](https://docs.honeycomb.teamofsilicons.com/upload-an-app/),
[publication/review workflow](https://docs.honeycomb.teamofsilicons.com/publication/).
