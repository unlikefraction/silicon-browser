# Recording integration gaps rechecked — 2026-09-05

Fresh source: Briefcase `5f27dd915c88d2cb839de368f273436f9254f9bf`; IAM `bab75c0a909481ad2d5dca5bd7d52df08476eaf3`. Installed CLIs: Briefcase 0.1.3 and IAM 1.2.2. This check used current source, the live Briefcase operation catalog, and a live IAM refresh/proof flow in the existing test environment.

| Gap | Current finding | Work belongs to |
| --- | --- | --- |
| Automatic session-end recording delivery | Implemented and verified with real Carbon video and Silicon video/command-log receipts, exact full downloads, and one retained version each. | Browser backend: completed; [live evidence](AUTOMATIC_RECORDING_LIVE_RETEST.md) |
| Background authorization renewal | IAM already supports app-bound refresh tokens. Live refresh, exact retry, new-token introspection, and new proof verification passed. Backend-owned encrypted credential lifecycle, durable refresh intents, revocation, and late-replay recovery are implemented. | Browser backend; no new IAM API required |
| Directory layout and OBO deletion | Accepted product boundaries: Briefcase owns its default app folder; the user does not require deletion through OBO. Neither is a blocker. | Briefcase directory policy; no new Browser folder/trash integration required |
| Recovery after an uncertain upload | Design constraint: consumed-proof replay fails, but fresh proof + stable destination/name supports at-least-once delivery with possible duplicate versions of one file. Lost-response recovery is not live-tested. | Implemented bounded Browser retries; exactly-once version publication is not a stated requirement |
| Large/slow recording transfer | 300 MiB streamed upload and complete readback pass. A separate 70 KiB request deliberately stretched beyond proof expiry returns 401 after 69 seconds. 300 GiB is untested; multipart does not extend proof lifetime. | Browser staging/streaming; longer-lived authorized transfer support if incoming uploads must exceed the proof window |

## Capture and delivery are separate

BrowserUse already captures the visual recording, and Browser reconciles recording URLs that appear after session end. A new recorder is not required. The Browser worker now delivers those captured bytes and commits their Briefcase entry/status, with backend-owned refresh, immutable owner binding, bounded retries, and explicit owner retry.

The user clarified that Briefcase owns directory layout and lack of OBO deletion is not a concern. Earlier reports treating those capabilities as blockers are superseded; historical observations are retained without carrying those requirements forward.

## Correction to the earlier conclusion

Describing durable authorization as requiring a new IAM delegation protocol was too strong. A separately owned Browser backend application session can refresh its ORT using existing IAM APIs, obtain a fresh OAT, and mint a recording proof. It needs encrypted storage, serialized rotation, persisted idempotency intent, atomic credential replacement, and terminal-revocation handling. Sharing a CLI-owned rotating token family between independent refreshers remains unsafe.

The live refresh check started with an active OAT, rotated successfully, returned the same credentials on an exact idempotent retry, and obtained a new proof that Briefcase's IAM audience credential successfully verified. No file was uploaded or deleted. After-expiry refresh support was checked in IAM source; this run did not wait for an access token to expire.

## What changed upstream

The latest Briefcase commit prevents changing the IAM UUID of an initialized sandbox while allowing same-plane credential updates. It also adds tests and documents that behavior. The OBO router, file handler, and transfer/replay rules are unchanged. This sandbox protection was reviewed without re-pairing or cleaning the existing test environment.

The live version endpoint still advertises API v1, contract 0.3.0, 42 operations, and exactly one OBO operation: `createFileOnBehalfOfMember` 1.0.0 at `POST /obo/files`. Normal member upload/trash endpoints are not interchangeable with OBO grants.

The subsequent native-feature and large-file recheck adds Browser Use metadata
correlation, terminal recording-readiness handling, and Briefcase streaming from
the same hashed file handle. The workspace suite now passes 251 tests. See the
[300 MiB results](BRIEFCASE_0_1_3_RETEST.md), [slow-transfer evidence](BRIEFCASE_SLOW_UPLOAD_RETEST.md),
and [Browser Use feature review](BROWSER_USE_FEATURES.md). The
[detailed gap analysis](BRIEFCASE_INTEGRATION_GAPS.md) separates missing Browser
implementation from accepted upstream design constraints. At-least-once delivery
can accept duplicate versions; no new upstream retry API is established as a
prerequisite. The former missing worker and viewer gates are closed by the [automatic live retest](AUTOMATIC_RECORDING_LIVE_RETEST.md). Remaining release boundaries are tracked in [current readiness](PRODUCTION_READINESS.md).
