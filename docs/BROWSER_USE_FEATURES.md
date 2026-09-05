# Browser Use native features and Silicon Browser integration

Reviewed 2026-09-05 against the current official documentation and SDK revision
[e2be431d43bec7ffdfed938e5aae832066f7110f](https://github.com/browser-use/sdk/tree/e2be431d43bec7ffdfed938e5aae832066f7110f).
This is a contract/source review, not evidence that every feature was exercised live.

## Bounded live verification

The initial phase performed read-only checks against three existing, stopped
Browser Use sessions. All three detail responses exposed `recordingAvailable:
true`, a populated `recordingUrl`, a `metadata` field, and usage counters. These
checks observed available native recordings without starting another paid browser.
An unfiltered browser list returned HTTP 200 with `totalItems: 12`; an exact filter
for a nonexistent metadata value returned HTTP 200 with `totalItems: 0`.

That initial phase used no new paid session; existing metadata did not prove those
older sessions carried our new correlation label. Later, a separate end-to-end
retest created real Carbon and Silicon sessions and verified native video delivery,
Silicon command delivery, and live viewer association. Ambiguous-creation recovery
continues to have captured-wire/mock coverage rather than a deliberately induced
live ambiguous POST. Recording URLs and credentials are omitted.
[Automatic live retest](AUTOMATIC_RECORDING_LIVE_RETEST.md).

## Capability matrix

This table compares the direct v3 OpenAPI snapshot reviewed at
`/tmp/sb-browser-use-docs/v3.json` with `providers/browser.rs`, the backend session
handlers, and the CLI. "Default" means the adapter omits the provider option, not
that we independently tested the advertised behavior on every website.

| Capability | Browser Use contract | Silicon Browser today |
| --- | --- | --- |
| Persistent profiles | Profile CRUD; browser sessions load a profile's saved state. | Creates/renames profiles and recovers creation by `userId`; profile sessions pass `profileId`. Incognito omits it. Provider deletion is not wired. |
| Remote control and live view | CDP WebSocket and interactive live URL. | `sb run` controls remote CDP directly through its local controller; the backend only issues the authorized connection and accepts command metadata. The frontend loads its authorized live viewer directly. |
| Recording | Optional native MP4; detail GET exposes URL/readiness after stop. | Always enables recording; tracks readiness and automatically delivers native bytes through the Briefcase worker. |
| Stealth | Managed hardened Chromium; session fingerprints randomized by provider. | Inherited; no custom fingerprint controls. Our profile fingerprint is a logical identifier, not a pinned browser fingerprint. |
| CAPTCHA solving | `solveCaptchas` defaults true. | Provider default; no creation flag exposed by our CLI. |
| Managed proxies | Country selection; omitted country defaults US; explicit null documented to disable. | Profile location selects country; incognito sends null. Actual measured traffic remains recorded; see the observed null/proxy discrepancy below. |
| Custom proxy | Host/port, optional credentials and certificate-error setting. | Not exposed by our adapter or CLI. |
| Screen dimensions | Width 320–6144; height 320–3456 pixels. | Provider defaults; no creation options exposed. |
| Resizing | `allowResizing` defaults false. | Default retained; raw CDP commands do not imply provider resize support. |
| PDF handling | In-tab rendering defaults true; PDFs also enter session downloads. | Default retained; no renderer creation option. |
| Website downloads | Cursor-paginated downloaded files with optional download URLs. | Provider downloads endpoint not integrated; distinct from browser recordings. |
| Timeout and stop | Session timeout 1–240 minutes; default 60; explicit stop action. | CLI accepts 15/30/45/60/120/240 minutes; incognito defaults 15, profiles require TTL. Backend stops/reconciles sessions explicitly. |
| Usage | Start/end/timeout timestamps, proxy volume, proxy and browser cost. | Validates and accounts for measured counters; exposed through session/usage commands. |
| Metadata and discovery | Up to ten string labels; exact AND filters and paginated listing. | Sends internal `sb_session_id`, uses it for ambiguous-create recovery; arbitrary caller labels not exposed. |

Contract sources: [direct browser create](https://docs.browser-use.com/cloud/api-v3/browsers/create-browser-session),
[official v3 models](https://github.com/browser-use/sdk/blob/e2be431d43bec7ffdfed938e5aae832066f7110f/browser-use-python/src/browser_use_sdk/generated/v3/models.py),
[profile guide](https://docs.browser-use.com/cloud/guides/authentication),
[stealth](https://docs.browser-use.com/cloud/browser/stealth),
[browser resource including downloads](https://github.com/browser-use/sdk/blob/e2be431d43bec7ffdfed938e5aae832066f7110f/browser-use-python/src/browser_use_sdk/v3/resources/browsers.py).
The live incognito proxy discrepancy is documented in
[BROWSER_PROVIDER_FINDINGS.md](BROWSER_PROVIDER_FINDINGS.md); explicit null is the
request contract, not proof that provider traffic never used a proxy.

## Direct browser v3

- Browser Use creates the remote browser, exposes CDP and a live view, captures its
  recording, and reports lifecycle timestamps and usage. Silicon Browser already
  requests `enableRecording: true`; there is no reason to implement another capture
  system. [Create contract](https://docs.browser-use.com/cloud/api-v3/browsers/create-browser-session)
- Creation accepts up to ten string metadata pairs. Silicon Browser now sends
  `metadata: {"sb_session_id": "<local session id>"}`. Recovery queries the browser
  list with `metadata=sb_session_id=<local session id>`; repeated metadata filters
  are ANDed. The adapter validates pagination and exact response labels, rejects
  duplicate matches, then retrieves the matched browser's detail and validates its
  identity again. Correlation is not provider-enforced POST idempotency: an empty
  search cannot prove that an earlier request will never commit, and historical
  browsers created without this label cannot be recovered with it.
  [List contract](https://docs.browser-use.com/cloud/api-v3/browsers/list-browser-sessions),
  [SDK browser resource](https://github.com/browser-use/sdk/blob/e2be431d43bec7ffdfed938e5aae832066f7110f/browser-use-python/src/browser_use_sdk/v3/resources/browsers.py)
- Stop starts recording upload. Recording URLs are absent from stop/list responses;
  detail GET supplies a presigned download URL when ready. The new
  `recordingAvailable: false` definitively ends polling; missing/null preserves
  compatibility with older responses. Silicon Browser's adapter preserves this
  signal for lifecycle handling. Provider lookup failure is not evidence that a
  recording can never arrive.
  [GET contract](https://docs.browser-use.com/cloud/api-v3/browsers/get-browser-session)
- Native recordings are MP4. Live view URLs permit interaction and must be handled
  as credentials. Recording is unavailable for Zero Data Retention projects.
  [Live preview and recording](https://docs.browser-use.com/cloud/browser/live-preview)

The generated v3 recording field descriptions still mention `/api/v2/browsers`;
the official v3 resource actually uses the v3 endpoint. This appears to be a
copied-description inconsistency, not a reason to switch this adapter's API version.

## Agent APIs and events are distinct

V4 additionally offers hosted natural-language agent runs, conversation follow-ups,
structured outputs, persistent workspace files, and ordered run-event polling.
Those features orchestrate an agent's work; they are not a drop-in replacement for
`sb run` forwarding commands to our managed remote browser. They are not currently
called by Silicon Browser's provider adapter.
[Agent quickstart](https://docs.browser-use.com/cloud/agent/quickstart),
[structured output](https://docs.browser-use.com/cloud/agent/structured-output),
[workspaces](https://docs.browser-use.com/cloud/agent/workspaces),
[observability](https://docs.browser-use.com/cloud/agent/observability).

The current infrastructure quickstart also presents V4 direct-browser REST routes
alongside the SDK's explicit V3 namespace. This does not make V4 agent runs necessary
for remote CDP automation. Our tested adapter remains on `/api/v3/browsers`.
[Infrastructure quickstart](https://docs.browser-use.com/cloud/browser/quickstart)

The SDK's v3 agent `sessions.waitForRecording` polls agent-session `recordingUrls`
for 15 seconds by default at two-second intervals and returns an empty list on
expiry. That helper timeout is not a direct-browser upload SLA. Our adapter uses
direct browser detail polling because it owns the browser lifecycle.
[SDK helper](https://github.com/browser-use/sdk/blob/e2be431d43bec7ffdfed938e5aae832066f7110f/browser-use-node/src/v3/resources/sessions.ts)

Current V4 run examples configure `browserSettings.record`; it applies when a new
browser is provisioned, and a follow-up reusing the browser retains that browser's
recording state. V4 run/workspace APIs are agent orchestration, not required to
capture a direct v3 browser controlled through CDP.
[SDK V4 models](https://github.com/browser-use/sdk/blob/e2be431d43bec7ffdfed938e5aae832066f7110f/browser-use-python/src/browser_use_sdk/generated/v4/models.py)

Documented webhook event types are `agent.task.status_update` and `test`. No
standalone browser-stopped or recording-ready webhook is documented there, so
these agent events do not replace direct-browser reconciliation/polling.
[Webhooks](https://docs.browser-use.com/cloud/guides/webhooks)

## Storage delivery boundaries

The reviewed direct-browser contract and official SDK do not expose a recording
export destination, callback upload, or caller-owned S3 bucket configuration. They
also do not specify recording retention duration, presigned URL lifetime, or a
maximum delay until recording upload completes. Absence from this public contract
is not proof that an enterprise/private feature does not exist.

The browser `downloads` endpoint lists files downloaded by web pages into provider
storage. It is a separate feature from exporting the browser recording.
[SDK browser downloads](https://github.com/browser-use/sdk/blob/e2be431d43bec7ffdfed938e5aae832066f7110f/browser-use-python/src/browser_use_sdk/v3/resources/browsers.py)

Briefcase delivery therefore consumes the existing provider recording bytes; it
does not require recapturing browser activity. Briefcase's automatic directory
placement and lack of Browser-initiated OBO deletion are intended product behavior,
not integration blockers. The implemented worker distinguishes recording readiness, retrieval, authorized
upload, and confirmed destination receipts. It uses a separate backend-owned IAM
refresh family enrolled through a second SLT, and copies the native MP4 without
recapturing it. Video and Silicon command JSONL have independent durable jobs;
retries preserve bytes and names and may create identical extra versions after
an unconfirmed commit. `SB_RECORDING_MAX_BYTES` defaults to 512 MiB per artifact.
The automatic live flow passed for real Carbon and Silicon sessions, with exact
recipient-side downloads and one version per artifact. This is separate from the
initial read-only checks; no prolonged-load or production-deployment claim is made.
[Automatic live retest](AUTOMATIC_RECORDING_LIVE_RETEST.md).
[Delivery implementation](BRIEFCASE_INTEGRATION.md),
[remaining limits](BRIEFCASE_INTEGRATION_GAPS.md).
