# Briefcase integration findings — 2026-09-05

Historical report. The sandbox-creation and CLI-release blockers passed the
[0.1.3 retest](BRIEFCASE_0_1_3_RETEST.md). See [current findings](BRIEFCASE_0_1_3_EXTERNAL_BUGS.md).

The reviewed Briefcase source is commit `6721a09d6fdd8348470c72ef0558fcfc59ba9eda`. The public service reports `silicon-briefcase`, API `v1`, contract `0.3.0`, build `0.1.0`. Public `/healthz` and `/readyz` both returned 200.

No raw credentials, environment keys, OTPs, or application secrets are included here. No environment was cleaned, deleted, or re-paired. Nothing was posted upstream.

## Published CLI does not implement the current documented setup

The installed CLI reports `briefcase 0.1.1`, and crates.io also reports **0.1.1** as the latest release. However:

- `briefcase env create --help` fails with `unrecognized subcommand 'env'`.
- `briefcase login --help` offers `--token-stdin` and describes saving an IAM access token. It has no `--slt-stdin` exchange workflow.
- Current repository `docs/testing-environments.md` and `docs/cli/README.md` instruct users to use `briefcase env create` and organization-bound SLT login.
- The repository's current Rust workspace still declares version 0.1.1 despite these additional capabilities.

**Impact:** Installing or updating the published CLI cannot execute the current documented pairing workflow. This is a release/documentation mismatch, not a missing local installation.

**Recommendation:** Publish a new client/CLI version containing the reviewed SLT/session/test-environment implementation, and make documentation identify the minimum release. Do not silently replace the installed binary with an unpublished build during an integration test.

For this audit, the reviewed HTTP contract was used instead.

## Hosted paired-environment creation returns unexplained 403

The existing IAM environment is `01a07025-85ea-7413-8045-24a897b86f88`. It originally contained the `sbaudit` test organization. Setup imported canonical production applications `tos>briefcase` and `tos>browser` into that test plane. IAM created the corresponding test `tos` organization; existing `sbaudit` fixtures were preserved. Both imported applications report `verified` and include `memberships.read`, `roles.read`, and `obo.issue` in their requested scopes.

### Successful prerequisites

1. Reused the existing production IAM session to mint an organization-bound SLT for `tos>briefcase`; no new user credentials were required.
2. Exchanged that SLT with `POST /api/v1/auth/slt`, `X-Org-ID: tos`, and a persisted idempotency key. Briefcase returned **200**, a scoped `tos` access/refresh pair, and the expected membership/role scopes.
3. With that exact Briefcase access token, `GET /api/v1/entries` returned **200** and two roots.
4. With the same token, `GET /api/v1/organizations/tos/testing-environments` returned **200** and two existing active environments.
5. Using the supplied IAM test root, `GET /api/v1/testing-environment` returned **200** and the exact expected IAM environment UUID.
6. Using the freshly imported test-only Briefcase Application secret with Basic authentication and that same IAM root, `GET /api/v1/application-directory/tos%3Ebriefcase` returned **200**, canonical `tos>briefcase`, and its expected public base URL.

The two existing Briefcase environments are paired to different IAM UUIDs (`01a07138-dad8-7ac3-9756-71ebbe6523bf` and `01a07131-abfa-7e01-b0dd-4ce49a8e5bb9`), so the visible listing does not show an existing pairing for the requested environment.

### Failed operation

Submitted once, through the production Briefcase control plane:

```http
POST /api/v1/organizations/tos/testing-environments
X-Org-ID: tos
Authorization: Bearer <production-Briefcase-access-token>
Idempotency-Key: <persisted-request-key>
Content-Type: application/json

{
  "name": "browser-recording-audit",
  "description": "Browser recording integration in existing IAM test plane",
  "iam_environment_id": "01a07025-85ea-7413-8045-24a897b86f88",
  "iam_environment_key": "<existing-IAM-root>",
  "iam_app_id": "tos>briefcase",
  "iam_app_secret": "<fresh-test-only-imported-secret>"
}
```

No Briefcase test-selector header was sent on this production management request. Response:

```json
{
  "error": {
    "code": "forbidden",
    "message": "The actor is not authorized for this action.",
    "request_id": "01a071f6-fdb1-7280-ab0c-d5a10ff06696"
  }
}
```

HTTP status was **403**. No new Briefcase environment UUID or root was returned. The exact request and idempotency key remain in private local storage; no retry with a new key was attempted.

### Diagnosis and next step

The reviewed docs allow any current production organization member to create a sandbox. Source `src/api/handlers/testing.rs::create` authenticates the production bearer, checks replay, validates the supplied IAM pairing, then calls the store. Source `src/infrastructure/testing.rs::require_environment_admin` rejects a test-plane execution context but otherwise allows this production path. The underlying IAM root and Basic directory reads both succeeded independently.

**Confirmed:** Authentication and ordinary member reads work, but the documented create operation rejects this setup. **Unconfirmed:** Which deployed authorization/pairing check produces the rejection; whether hosted code/configuration differs from the reviewed source. A role bug, duplicate pairing, or IAM credential failure must not be asserted from this generic response alone.

**Required upstream diagnostic:** Inspect the Briefcase server trace for request `01a071f6-fdb1-7280-ab0c-d5a10ff06696`, especially the pairing-validation stage and any IAM request correlation. Correct the implementation or document the actual additional permission if intended. Preserve the exact idempotency intent when retrying an ambiguous or recovered create operation.

The Browser-to-Briefcase test recording upload cannot yet run against this IAM plane because it has no paired Briefcase root. This is an integration prerequisite failure, not a completed end-to-end recording validation.

## Separate setup observation: empty imported OBO catalog

The first real Browser `issue_recording_proof` call successfully authenticated
its test initiator but rejected the imported Briefcase catalog because it had no
`briefcase.files.create` endpoint. The saved import response had `obo_endpoints: []`.
Briefcase's IAM runbook explicitly calls for separate OBO registration, so this
is a setup prerequisite, not a confirmed product defect. The published endpoint was registered only on the newly imported test Briefcase
application; read-back confirmed verified version 2 with no pending changes.
Production registration is unchanged. A subsequent real Browser proof request
succeeded. No file was uploaded because the Briefcase sandbox root is still
unavailable.
