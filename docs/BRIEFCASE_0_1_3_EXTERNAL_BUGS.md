# Briefcase 0.1.3 findings — 2026-09-05

Current source: `5f27dd915c88d2cb839de368f273436f9254f9bf`. Installed CLI and published Rust client are 0.1.3. The full hands-on results are in [the retest](BRIEFCASE_0_1_3_RETEST.md). No issues were posted upstream.

## Previous blockers: resolved in this retest

- The original paired-sandbox creation request, retried with its persisted body/idempotency key, returned **201**. It previously returned 403. Login, root selection, and member listing in that new sandbox succeeded.
- Published CLI 0.1.3 implements the documented environment commands and SLT login. Real Carbon and Silicon logins work; the 0.1.1 release/documentation mismatch is resolved.

## Undocumented hosted requirement: User-Agent

A raw HTTP client without `User-Agent` receives **403**, `Content-Type: text/html`, `Server: awselb/2.0`, even on public `GET /api/version`. The same public request with `User-Agent: silicon-browser/0.1.0` receives **200 application/json**. Python urllib's default agent also succeeds. This was checked without credentials or file mutations.

Two Browser Rust OBO upload attempts initially returned 403. An equivalent Python request succeeded. Our adapter used reqwest without a configured User-Agent; the official Briefcase Rust client sets one. After adding a descriptive User-Agent to Browser, its actual IAM-proof-to-Briefcase upload returned **201**. Both text and MP4 round trips then succeeded.

**Classification:** Browser integration defect fixed locally; the hosted edge's additional header requirement is a documentation gap. The observations identify the edge behavior, but do not establish which WAF rule produced it. It is not evidence of a broken OBO permission check.

**Suggested upstream change:** Document the required User-Agent for raw integrations or allow headerless API clients. Prefer the API's structured error envelope where possible; an HTML edge response bypasses its normal request-ID diagnostics.

A read-only reproduction:

```python
import urllib.request
opener = urllib.request.build_opener()
opener.addheaders = []
url = 'https://backend.briefcase.teamofsilicons.com/api/version'
# Without a User-Agent: HTTPError 403 on the tested hosted deployment.
opener.open(urllib.request.Request(url))
# With a User-Agent: 200.
opener.open(urllib.request.Request(url, headers={'User-Agent': 'silicon-browser/0.1.0'}))
```

Run the two requests separately or catch HTTPError so the first result does not prevent the second.

## Browser response handling: corrected

Briefcase preserves a file's original `origin_app_id` when another authorized caller publishes a new version. It may be null for a member-created file. Our adapter incorrectly required it to equal the current uploading app, which could reject an authorized overwrite after it committed.

The field is now optional creator metadata, and the adapter no longer equates creator provenance with the current proof issuer. Organization, entry type, byte count, path, URL, and request-header validation remain. A focused mock regression covers null and different original app IDs; the general cross-app overwrite case was not independently exercised live. Native owner overwrite did pass and preserved the entry ID while adding a version.

## Remaining integration gaps

The current OBO interface exposes file creation/upload. Briefcase owns directory layout and creates/selects the default app folder. The user has explicitly accepted that policy and does not require OBO deletion. Earlier language treating per-session folder creation or remote trash as blockers is superseded; neither is a current external defect.

Browser Use provides native capture. Browser now implements automatic delivery and a separate backend-owned credential lifecycle using existing IAM refresh APIs. Real Carbon video and Silicon video/command-log deliveries passed; see [the automatic live retest](AUTOMATIC_RECORDING_LIVE_RETEST.md).

Confirmed delivery constraints remain: Briefcase stages the entire upload before verifying its 60-second proof, so transfer timing must fit that window. This does not show that every large upload fails; 300 GiB has not been tested. A consumed proof cannot be replayed, but a fresh proof for stable destination/name and identical bytes can publish another version of the same file after an ambiguous success. Browser can implement bounded at-least-once delivery if duplicate versions are acceptable. Exactly-once version creation and a new delegated reconciliation API are not established product requirements or confirmed external defects. The lost-response retry scenario is source-reviewed, not independently live-tested. See [the source-grounded gap analysis](BRIEFCASE_INTEGRATION_GAPS.md).

## Live large-file and slow-transfer follow-up

The real Rust upload adapter successfully stored **300 MiB**, using **18 MiB peak
memory**. Full streamed SHA-256 readback and byte ranges across the source-selected
8 MiB part boundary, 100 MiB threshold, 200 MiB boundary, last part boundary, and
file tail all pass. Carbon CLI sees one version and correct quota accounting;
Silicon cannot see the Carbon's private entry. This confirms the tested multipart
path without exceeding the sandbox's 2 GiB limit. See
[the detailed results](BRIEFCASE_0_1_3_RETEST.md).

**Confirmed external integration limitation: uploads cannot outlive the proof.**
A separate 70 KiB upload began with about 59.9 seconds of proof lifetime remaining
and was deliberately streamed over 69 seconds. It returned **HTTP 401 /
unauthenticated**, request `01a0726d-5a47-7231-8152-fc79661f334b`, after the body
finished about 9.1 seconds after proof expiry. The source stages the body before
IAM verification; the public error does not disclose the exact internal rejection
reason. This is a transfer-duration limitation, not file corruption or a multipart
storage failure. A fast control with identical bytes and a fresh proof returned
201 in 1.654 seconds. Subsequent owner CLI listing confirmed the slow filename
absent and the control present; usage increased only by the successful control's
71,680 bytes. See [timings and reproduction](BRIEFCASE_SLOW_UPLOAD_RETEST.md).

For uploads that must take longer than the proof window, consider an authorized
upload session whose body digest remains bound and whose transfer deadline is
independent of the initial short-lived proof. Simply changing storage chunk size
or refreshing the parent OAT does not extend a proof already attached to a request.
No upstream issue was posted and no authentication lifetime was relaxed locally.

No new file corruption or cross-actor permission defect was confirmed. The hosted
header requirement and tested transfer deadline are recorded separately from the
implemented and live-verified Browser delivery worker. A 300 MiB pass does not establish 300 GiB
transfer support.
