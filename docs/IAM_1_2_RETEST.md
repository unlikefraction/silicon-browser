# IAM 1.2 retest — 2026-09-05

> Historical IAM 1.2 checkpoint. The September 6 production implementation now uses a 15-second authorization cache invalidated by signed IAM webhooks; statements below about uncached immediate lookup describe this earlier test. See [current deployment/auth behavior](DEPLOYMENT.md).

Updated the installed IAM CLI to **1.2.0** and pinned Browser's IAM client to
**=1.2.0**. Read `UNDERSTANDING.md`, audited released source, and exercised the
authorized IAM testing environment with separate Carbon and Silicon homes.
The release and deployed source provenance is recorded in
[IAM_1_2_PROVENANCE.md](IAM_1_2_PROVENANCE.md). GitHub main remains behind the release.

## Hands-on use

These were actual terminal workflows against live IAM and Browser Use, not just
mocked tests or scripted HTTP assertions:

- Completed Carbon interactive login, including the required private-home permission
  migration, then reused its saved session to obtain Browser application tokens.
- Signed in a Silicon, inspected its own membership, and used the CLI's recovery
  guidance after authorization changes invalidated its tokens.
- Ran Browser setup for each persona with separate saved credentials; installed
  Chromium once and confirmed subsequent setup reused it.
- As Carbon, started an incognito browser, opened example.com, inspected its
  interactive snapshot, clicked `@e2`, and verified navigation to IANA's example-domain page.
  Silicon could not inspect that private session.
- Created a shared profile with a tag ACL. Silicon initially could not list it.
  After granting the tag and obtaining fresh credentials, Silicon could list/show
  the profile, start a real profile-backed browser, and repeat the navigation flow.
- A competing Carbon session on the occupied profile received `profile_busy` with
  the owning actor/session details. No second browser was started.
- Removed the Silicon's tag. Its old token was rejected immediately. After fresh
  login/setup, listing was empty and direct profile lookup returned `not_found`.
- Explicitly ended the paid test sessions and retired the shared test profile.

## Browser fixes prompted by the retest

- Consume IAM 1.2's authoritative authorization snapshot, including scoped tags,
  validated actor/org/audience/epoch context, and testing-plane checks. Introspect
  each authenticated request so revocation and tag removal take effect immediately.
  Avoid redundant identity projection writes when the stored identity is unchanged.
- Accept provider `cookieDomains: null` as an empty collection while rejecting
  incompatible types. Treat malformed mutation responses as potentially committed
  and reconcile remote state before releasing local resources or declaring failure.
- Parse scientific decimal costs exactly without floating-point conversion; bound
  exponent/overflow behavior. A real stop initially failed decimal parsing and
  succeeded on retry, but the original rejected value was not captured, so its
  precise cause is unconfirmed.
- Preserve unexpected incognito proxy counters instead of discarding them while
  retaining associated charges. The provider discrepancy remains external.
- Use provider runtime for recording duration after delayed stop finalization,
  with local elapsed time only as a fallback. The observed old fixture's inflated
  duration was not rewritten. Ended sessions now display zero remaining TTL.
- Stabilize two tests that accidentally depended on short scheduler deadlines or
  a complete HTTP request arriving in one TCP read.

The malformed profile creation had already committed remotely. Recovery was checked
by matching its remote user ID, backing up the isolated test database, and returning
only that failed fixture to provisioning; reconciliation found the existing profile.

## Validation

- Rust workspace/all-target suite: **232 passed** (18 client, 145 backend,
  23 CLI unit, 10 CLI integration, 36 shared).
- Final build, formatting, Clippy with warnings denied, and documentation with
  warnings denied: passed.
- Live Browser authentication checks: **27/27 for Carbon and 27/27 for Silicon**.
- Live IAM token protocol: application discovery, cached login, exchange and
  idempotent retry, used-SLT rejection, introspection and wrong-org rejection,
  refresh/retry, OBO verification/replay rejection, and Silicon exchange passed.
- Live revocation: Browser returned HTTP 401 immediately after token revocation.
- IAM offline checks initially passed all16 groups, including96 concurrent logout
  commands. A later run had one unsuccessful logout; seven instrumented reruns
  passed all16 groups each (672 concurrent commands). The intermittent failure
  remains unclassified because the original harness omitted its stderr/exit detail.
  The harness now retains those diagnostics and distinguishes failed calls from
  lost writes after successful calls.

## External findings and limits

[IAM external findings](IAM_1_2_0_EXTERNAL_BUGS.md) records the intermittent logout
observation, empty-tag confirmation wording, release-source drift, and the prior
bugs' retest status. [Browser provider findings](BROWSER_PROVIDER_FINDINGS.md)
records nonzero proxy metering despite an explicit null proxy-country request.
Actual routing was not independently measured.

Recording upload to Briefcase remains deferred/pending; this run does not establish
end-to-end recording delivery. No deployment, publication, or upstream issue posting
was performed. These results establish bounded regression coverage and live persona
workflows, not exhaustive production certification.
