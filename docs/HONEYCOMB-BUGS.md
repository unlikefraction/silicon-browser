# Honeycomb publication findings

Recorded on 2026-09-16 while publishing `tos>browser` version `0.2.3`.
The original findings below are retained as history. Following the platform
repair, the same archive uploaded successfully, installation passed, and the
public request reached validator review. No production IAM or Honeycomb code
was changed by this Browser task.

## Verification after the platform repair

- **IAM-1 resolved for this upload:** accepted release
  `25fe4dc9-90bc-46c0-bdc9-f3ca33a039ae`, version 0.2.3, 45,672,881 bytes,
  with the original SHA-256 and idempotency key.
- **HC-1 planning fixed:** public request
  `17dd5de1-49f7-4a3e-b45e-9a041ec5f3fa`, plan
  `cef53f28-4bb0-47f3-adf7-ecf80c635eca`, revision 1, reached
  `awaiting_validator`. Current upstream implements review and activation;
  Browser's own activation is still untested pending approval.
- **HC-2 source fixed:** [Honeycomb 3912ec0 storage mapping](https://github.com/teamofsilicons/silicon-honeycomb/blob/3912ec0/crates/server/src/storage.rs)
  distinguishes authentication, authorization, rate-limit and service failures,
  preserving allowlisted diagnostic codes and request IDs. No production failure
  was induced to retest the error path.
- **Installation verified:** Honeycomb installed the archive under alias
  `sb-honeycomb`; CLI 0.2.3, help, bundled controller 0.36.0 and fresh IAM login
  passed. Full setup still requires separate user recording-delivery consent.

The current account is a TOS administrator, but the review endpoint returns
`can_decide: false` for the Honeycomb gate. This is an approval requirement,
not a remaining reproduction of the old platform bug. IAM's designated
`honeycomb.applications.review` capability is separate from organization admin.

## HC-1: Public review and activation are not implemented in the production adapter

**Impact:** the normal public-publication workflow cannot complete.

At inspected Honeycomb commit `b84feda7a729ef60d757183269ab814b361868ea`,
`IamManagement` does not implement `publication_plan`, `review_eligibility`,
`review_decision`, or `activate_publication`. The inherited defaults return
unavailable or false. `reviews::plan` catches the planning error and persists
`awaiting_review_plan`, so an application cannot reach validator review.
Even applications requesting only noncritical scopes require the validator gate.

There is also an unfinished accepted-state contract: Honeycomb requires IAM's
`publication_request_id` to match the reviewed request, but the inspected IAM
application record does not expose that identity. Implementing only the first
adapter method would therefore not complete publication.

**Evidence:** source-confirmed; Browser has not reached this step because its
archive upload fails first. This is not a claim that Browser has a pending
publication request. Honeycomb's own integration review documents these gaps.

**Reproduction after storage is repaired:** upload a valid release, request
publication through the normal CLI or console, then inspect the request state.
With this adapter it remains `awaiting_review_plan`.

**Required repair:** implement the IAM review-plan, current reviewer eligibility,
decision and activation contracts, including the accepted request identity;
exercise them through the real production adapter. Existing fixture-only review
tests do not establish this integration. Changing a user's local `validator`
flag or writing public visibility directly does not implement the missing flow.

Sources:

- [Production management adapter](https://github.com/teamofsilicons/silicon-honeycomb/blob/b84feda7a729ef60d757183269ab814b361868ea/crates/server/src/iam_management.rs#L257)
- [Unimplemented defaults](https://github.com/teamofsilicons/silicon-honeycomb/blob/b84feda7a729ef60d757183269ab814b361868ea/crates/server/src/integration.rs#L118)
- [Planning failure state](https://github.com/teamofsilicons/silicon-honeycomb/blob/b84feda7a729ef60d757183269ab814b361868ea/crates/server/src/reviews.rs#L59)
- [Authoritative activation check](https://github.com/teamofsilicons/silicon-honeycomb/blob/b84feda7a729ef60d757183269ab814b361868ea/crates/server/src/activation.rs#L300)
- [Existing IAM contract gap report](https://github.com/teamofsilicons/silicon-honeycomb/blob/b84feda7a729ef60d757183269ab814b361868ea/docs/IAM-CONTRACT-REVIEW.md)

## HC-2: Storage errors incorrectly suggest missing consent or scopes

**Impact:** an upstream server error looks like a permissions problem, encouraging
unnecessary consent renewal and scope changes.

The live archive upload returned HTTP 503, `integration_unavailable`:

> IAM refused the Briefcase OBO proof. Check consent and effective external scopes.

All four required Briefcase scopes were effective on `tos>honeycomb`, and the
session had fresh consent. A direct IAM exchange instead returned HTTP 500,
`internal_error`; its server log classified the failure as `obo_proof_insert`.
The storage adapter maps every exchange error to the same consent/scopes message.

**Required repair:** distinguish upstream service failures from authorization
failures and retain the safe upstream error code/request ID for diagnosis.
Keep credentials, subject tokens and proofs out of responses and logs. Check
that an IAM 500 does not produce consent instructions while an authorization
failure still provides useful guidance.

Source: [unconditional exchange-error mapping](https://github.com/teamofsilicons/silicon-honeycomb/blob/b84feda7a729ef60d757183269ab814b361868ea/crates/server/src/storage.rs#L39).

## IAM-1: Configured proof lifetime conflicts with the database constraint

**Owner:** IAM; an upstream dependency blocking Honeycomb archive storage.

**Observed live:** IAM commit `72767708d9ac2e7bf11873aee9bc8800da3c4835`.
The exchange for `tos>honeycomb` to `tos>briefcase`, endpoint
`briefcase.uploads.reserve`, failed with request ID
`01a0a904-aee3-75c3-b60c-ad97b466c388` at `2026-09-16T07:00:51.782410Z`.
The production API log records `applications feature failure`, category
`obo_proof_insert`.

Migration 0095 defaults endpoint proof lifetimes to 300 seconds, and the exchange
handler uses the configured lifetime. The original `obo_proofs_lifetime`
constraint still limits proofs to 60 seconds. The new default therefore fails
at insertion before Briefcase receives an upload reservation.

**Verified repair candidate:** a new migration aligns the proof constraint with
the existing positive `i32` endpoint-lifetime contract. A local patch also extends
the existing SQL regression. On disposable PostgreSQL 16, all 99 existing
migrations applied, the 300-second insert reproduced the constraint failure,
and the fixture passed after the repair. Boundary checks and the existing live
consent-revocation check passed. Parent-token expiry/revocation still applies.
The migration requires a table lock; it has not been applied to production.

Sources:

- [300-second default](https://github.com/teamofsilicons/silicon-iam/blob/72767708d9ac2e7bf11873aee9bc8800da3c4835/migrations/0095_obo_endpoint_lifetime.sql#L2)
- [Configured expiry in exchange](https://github.com/teamofsilicons/silicon-iam/blob/72767708d9ac2e7bf11873aee9bc8800da3c4835/src/features/applications/obo.rs#L350)
- [Stale 60-second constraint](https://github.com/teamofsilicons/silicon-iam/blob/72767708d9ac2e7bf11873aee9bc8800da3c4835/migrations/0005_governance_sso_and_obo.sql#L968)

## Resolved prerequisite and continuation

Honeycomb originally lacked `briefcase.uploads.reserve`,
`briefcase.uploads.commit`, `briefcase.files.read`, and
`briefcase.link_access.update`. The authorized request and provider approval
completed; all four became effective, and login consent was renewed. Missing
grants are no longer the diagnosed upload blocker.

The original upload completed using this mutation:

```sh
honeycomb --idempotency-key silicon-browser-honeycomb-0.2.3-upload-20260916 \
  releases upload 'tos>browser' target/honeycomb-browser-0.2.3.tar.gz \
  --revision 1 --json
```

Archive SHA-256:
`6360c55926b41351585ff6041e78d1dae7166f9f030d93b456ce997b4a10195a`.
Do not upload a replacement or recreate the application. A designated validator
can inspect the request using:

```sh
honeycomb publication review 17dd5de1-49f7-4a3e-b45e-9a041ec5f3fa honeycomb --json
```

After reviewing, that validator can use `publication decide` for provider
`honeycomb`, decision `approve`, revision `1`, with their review reason. Once the
request reaches `awaiting_activation`, the application administrator can finish:

```sh
honeycomb --idempotency-key silicon-browser-honeycomb-activation-20260916-01 \
  publication activate 17dd5de1-49f7-4a3e-b45e-9a041ec5f3fa --revision 1 --json
honeycomb apps get 'tos>browser' --json
```

Verify public catalog visibility and anonymous installation after activation.
