# IAM 1.2.2: refresh-family revocation invalidates sibling access tokens

Observed 2026-09-05 against the live test environment. CLI version: 1.2.2; Browser client SDK: 1.2.1. Source inspected: IAM commit `bab75c0a909481ad2d5dca5bd7d52df08476eaf3`. No credentials or signed URLs are included. This report has not been posted externally.

## Live observation

The Carbon UI disabled Browser's backend-owned recording authorization. While its local state was `revoking`, the independently SLT-exchanged Carbon CLI credential still returned HTTP 200 from `whoami`. After the backend's OAuth revocation completed and state became `disabled`, both the Carbon CLI and Carbon UI access credentials returned HTTP 401. The Silicon credential remained usable. These observations were recorded by the root live-test runner; this source review did not mutate external credentials.

The separate SLT exchanges provide separately rotating families, but do not necessarily provide separate parent IAM sessions. A fresh SLT from the same cached login does not isolate its access token from this revocation behavior.

## Source-confirmed cause and documentation discrepancy

[`oauth.rs:2168`](https://github.com/teamofsilicons/silicon-iam/blob/bab75c0a909481ad2d5dca5bd7d52df08476eaf3/src/features/applications/oauth.rs#L2168) resolves the supplied ORT to one family and its parent authentication session. It revokes that family and its refresh members, then at line 2234 executes `OAUTH_SESSION_CLIENT_ACCESS_REVOCATION_QUERY`.

[That query, lines 129–135](https://github.com/teamofsilicons/silicon-iam/blob/bab75c0a909481ad2d5dca5bd7d52df08476eaf3/src/features/applications/oauth.rs#L129), revokes every access token with the same `authentication_session_id` and `client_application_id`. It has no family predicate. This is direct token revocation, not a membership authorization-epoch increment, global logout, or another application's revocation.

The [SDK method documentation](https://github.com/teamofsilicons/silicon-iam/blob/bab75c0a909481ad2d5dca5bd7d52df08476eaf3/crates/client/src/api/oauth.rs#L127) and [client guide](https://github.com/teamofsilicons/silicon-iam/blob/bab75c0a909481ad2d5dca5bd7d52df08476eaf3/docs/client/README.md#L280) describe refresh-token revocation as family scoped. The broader session-and-client access-token invalidation is documented for **refresh reuse detection** in the API guide, but the normal explicit revoke path uses the same query. The observable scope is therefore broader than the normal-revoke documentation suggests. This should be documented explicitly or narrowed upstream if family isolation is intended; it is not evidence of cross-principal authority leakage.

## Safe recovery and Browser mitigation

Other families' ORTs are not revoked by this query. [Refresh exchange, lines 1483 onward](https://github.com/teamofsilicons/silicon-iam/blob/bab75c0a909481ad2d5dca5bd7d52df08476eaf3/src/features/applications/oauth.rs#L1483), checks the presented family and current principal/session/membership/consent/client authority, without requiring its previous OAT to remain active. This supports refresh of an unaffected family's current ORT; actual refresh remains authoritative and may reject expired or otherwise revoked authority.

Browser retains explicit revocation of its backend-owned family. On an unauthenticated recording-proof attempt it persists a new refresh mutation, rotates only that owned credential, and retries proof issuance once. It never re-enrolls or borrows the CLI's ORT. Forbidden authority is terminal; a rejected refresh or a second unauthenticated result does not loop. Stored principal and membership bindings, encrypted credentials, mutation replay keys, lease ownership, and concurrent-disable checks remain enforced.

Regression coverage exercises sibling-access revocation recovery, fully revoked-family rejection, second-rejection loop prevention, and stale-access failure isolation. Parent-run CLI/UI refresh recovery is reported separately from this source-level conclusion.

## Live Browser recovery follow-up

After replacing the backend grant, the prior grant reached disabled. A direct request with the stored Carbon CLI OAT returned 401 with `x-sb-auth-rejected: 1`. Running the real `sb` binary then succeeded without a new SLT or manual cache change, and its persisted replacement OAT returned 200. The already-open Carbon UI recovered on navigation without prompting for sign-in and created a new named/described session. Backend marks only extractor rejections; handler-side 401s and uncertain transport failures are not automatically replayed.

The CLI will not use saved refresh credentials for an explicit environment OAT or adopt credentials replaced by another login while an old request was pending. The stateless Rust transport exposes an optional caller-owned recovery hook; it does not own filesystem state or an ORT.

The final live worker check also passed: the same persisted backend grant rotated its own credentials after sibling-access revocation and automatically delivered the new 590,227-byte native video on its first artifact attempt. Owner download matched the proof digest and only one version existed. [Full evidence](AUTOMATIC_RECORDING_LIVE_RETEST.md).
