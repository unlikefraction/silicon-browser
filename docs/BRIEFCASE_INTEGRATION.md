# Briefcase recording delivery

This describes the October 2026 local consumer cutover. It is not a deployed
release claim. Earlier live recording reports exercised the preceding proof
protocol and do not validate this implementation.

Browser copies its provider's existing native MP4 and, for Silicon initiators,
cooperatively reported command logs. It does not introduce another recorder.

## Separate feature approval

Configure `BRIEFCASE_URL` and `BRIEFCASE_APP_ID` together with Browser's IAM
application credentials and retain `SB_ENCRYPTION_KEY`. Ordinary login discloses
IAM identity/membership information. It never grants Briefcase actions.

The website, Rust SDK, and `browser recording-access` commands start a separate
approval request for `briefcase.uploads.reserve`, `briefcase.uploads.commit`,
`briefcase.uploads.status`, and `briefcase.entries.list`. All four roots must name
the same selected Briefcase account, organization, environment, and generation.
The user opens IAM, chooses the Briefcase account and
organization, approves, then supplies the returned one-use code. No paid browser
session starts as a side effect of approval. `browser setup` offers the same
manual-code flow. `SB_RECORDING_SLT` and the old delivery SLT endpoint are retired.

The backend API uses the ordinary authenticated Browser account and `X-Org-ID`:

- `POST /api/v1/auth/delivery/authorizations`, body `{}`, and a 16–255 character
  `Idempotency-Key`: create or resume the exact pending request.
- `GET /api/v1/auth/delivery/authorizations/{id}`: read a request owned by that
  exact account and membership.
- `POST /api/v1/auth/delivery/authorizations/{id}/complete`, body `{code,state}`:
  redeem and encrypt the approved credentials.
- `GET /api/v1/auth/delivery`: inspect local grant status.
- `POST /api/v1/auth/delivery/end`: disable local use and erase its credentials.

Responses use the normal `data` envelope. A pending/completed consent exposes
`authorization_id`, `consent_url`, `state`, `status`, and `expires_at`; no access
or refresh tokens reach the browser/CLI. A mistyped code returns
`400 invalid_recording_consent` and keeps login valid. Correcting it uses a
separate redemption retry identity; repeating the same code remains idempotent.
Expired/withdrawn approval requires a fresh feature request, not another login.

SQLite stores encrypted dedicated OBO pairs under account, organization,
membership, and testing-context AAD. It serializes refresh and uses the old
high-entropy refresh token's digest as the retry identity. Lost responses or a
process restart repeat that same mutation. New paid sessions force IAM refresh
validation first; a revoked grant stops them before provider creation. Recipient
verification checks every downstream call again. Ordinary logout does not erase
approval. Users can revoke the IAM grant through IAM's OBO management screen;
Browser's local Disable control blocks pending/new work and erases its copy.
Already in-flight work may complete.

The consent request's IAM identifier, originating application/account/org and
expiry must match before Browser saves its consent link. The chosen downstream
provider account may differ after explicit approval. A delayed rejection of an
older upload credential cannot disable a replacement approval.

Migration `0010_recording_obo_consent.sql` disables legacy login-derived delivery
families and queues their revocation. Later migrations retain completed receipts,
hold earlier attempted uploads for explicit reconciliation, and require fresh
approval for grants missing the fourth status root. No historical ordinary token
becomes OBO consent, and earlier attempted artifacts are never republished under
a newly invented operation ID.

## Recording publication

1. Session stop/expiry finalizes usage and queues provider recording lookup.
   Source downloads reject private addresses, pin DNS results, follow no
   redirects, and send no IAM/provider credentials. Files stage anonymously.
2. Video and Silicon command-log work retain separate durable claims. Log intent
   freezes report admission before reading pages; exact existing reports can
   replay, but late new reports remain rejected. Carbon sessions have no log file.
3. The worker hashes the exact file and persists the operation ID, manifest,
   digest and size. It obtains a current token for each control call, preserving
   the selected Briefcase actor/org. That
   destination is pinned before the first upload so an uncertain write cannot be
   redirected by a later approval.
4. The Briefcase adapter reserves a stable operation for the session/artifact
   with empty relative parent, exact name, media type, size, and SHA-256. It
   transfers the same open file using only the narrow staging capability, then
   refreshes authority after transfer and commits with the separately approved
   OBO endpoint token. The obsolete raw
   `/api/v1/obo/files` route is never called.
5. A retry queries status for the persisted operation before sending bytes again,
   recovers reserved/staged/committed state,
   and resolves the published entry through the approved list endpoint. It checks
   selected org, entry UUID, safe destination path, name, media type, and size before
   saving the encrypted receipt. Original entry-creator provenance is retained by
   Briefcase. Completed artifacts are not uploaded again.

The destination is `apps/browser/private/<selected-public-id>/`; filenames are
`<session-id>.mp4` and `<session-id>-commands.jsonl`. Recordings remain owned and
visible in their original Browser organization even when stored in a different
approved Briefcase organization. Reapproval to a different destination cannot
silently move an already attempted artifact. Restore its original destination to
resume it. `browser recording send` retries eligible failed artifacts without
resetting successful receipts or accepting changed source bytes. Local hiding
never deletes Briefcase files.

## Testing and release

Shared testing routes authenticate Browser's test app secret before opening an
isolated environment/generation database. The approved token response supplies
Briefcase's imported app secret; no production fallback is allowed. Production
and different testing worlds cannot decrypt each other's grant envelopes. IAM
clean/credential rotation prevents retired generations from delivering.

The preceding 0.4.1 implementation's local verification on 2026-10-03 passed 214
backend tests, a separate legacy
schema-upgrade regression, 91 SDK/CLI tests, 39 frontend tests, the production
frontend build and strict Rust lint. Synthetic desktop/mobile checks exercised
wrong-code recovery, context switching, revoked approval and completion without
automatically starting a paid session. The former Briefcase SDK wire suite
passed 27 tests. Current builds use the registry IAM version pinned in `Cargo.toml`
and `Cargo.lock`; Briefcase controls use the bounded HTTP adapter with separate
per-root token acquisition. The retained vendor directories document previous
release provenance and are not runtime dependencies. Those counts are
historical evidence, not a verification claim for the current four-root adapter.

Apply IAM's complete new OBO migration set, including selected-provider metadata
in `0140`, and deploy the matching Briefcase reservation/status/commit receiver.
Register/approve all four endpoint definitions, import Briefcase into the same
test world, and deploy Browser migration/API/website/CLI together. Verify one
real Carbon and Silicon recording, cross-org selection, uncertain retry,
revocation, and test clean after deployment. Local mock-provider/SQLite evidence
does not prove paid provider compatibility, hosted storage, or production rollout.

The shared Honeycomb environment `7c761acc-adb5-40e9-9498-a8b3e5a86396` was checked
on 2026-10-03 UTC. Its import has not completed: Browser reports
`Protected lifecycle transport is not configured for this application`, while
IAM, Briefcase and Honeycomb report ready. Browser still needs the protected
participant receiver and dedicated service credential. Honeycomb operators must
then register Browser's backend origin and secret reference in
`HONEYCOMB_LIFECYCLE_PARTICIPANTS`. The receiver contract is
`PUT /internal/honeycomb/organizations/{org}/testing-environments/{environment}/operations/{operation}`;
it must authenticate the dedicated bearer, durably apply each operation once,
fence retired environments/generations, and return the exact matching receipt.
After this work, retry the original pending operation before refreshing the
import or running shared tests.
An environment-level `ready` label does not override a failed participant or
`operation_pending: true`; never fall back to production credentials.

References: [delegated upload contract](https://docs.briefcase.teamofsilicons.com/api/delegated-uploads/),
[IAM OBO](https://docs.iam.teamofsilicons.com/api/obo/),
[Honeycomb shared testing](https://docs.honeycomb.teamofsilicons.com/testing-environments/).
