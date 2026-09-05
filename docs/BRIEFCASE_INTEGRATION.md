# Briefcase recording delivery

The backend now consumes durable recording work automatically. It copies Browser
Use's existing native MP4 into the initiating member's private Briefcase and also
uploads cooperatively reported command logs for Silicon-initiated sessions. It does not implement
another browser recorder. This describes the current workspace implementation;
the complete automatic flow passed the paired-sandbox live retest. This report
makes no production deployment claim.

## Setup and credential ownership

Configure `BRIEFCASE_URL` and `BRIEFCASE_APP_ID` together, alongside Browser's IAM
application credentials. Issuer and audience must be canonical applications in
the same organization. In tests, configure both `IAM_TEST_ENVIRONMENT_KEY` and
`BRIEFCASE_TEST_ENVIRONMENT_KEY` for the paired sandboxes; their values are distinct.
The production backend entrypoint requires recording delivery before permitting a
paid browser session. `BRIEFCASE_URL` alone is insufficient.

`sb setup` first establishes the CLI login. Recording delivery needs a **second,
fresh Browser-targeted IAM `oac_` token**, supplied through the masked prompt or
`SB_RECORDING_SLT`. An already active delivery authorization is reused. Never reuse
the consumed login SLT or share the CLI's refresh token with the worker.

The backend exchanges the second SLT into its own application session, encrypts its
OAT/ORT, serializes refresh, and persists pending exchange/rotation identity before
network mutation. The job stores the initiating IAM principal and membership IDs;
matching a public actor name is insufficient. Historical sessions without this
binding are not silently adopted by a later authorization. Disabling delivery
revokes the backend-owned family and prevents new uploads; an already in-flight
write may still finish. The Rust client exposes authorization status, enrollment,
and disable operations independently of the CLI.

## Transfer lifecycle

1. Session stop/expiry finalizes provider usage and queues native recording lookup.
   Detail GET supplies a fresh presigned URL; `recordingAvailable: false` ends
   unsuccessful readiness polling. Recording capture remains entirely upstream.
2. Durable claims track video and command logs separately. Download resolution
   rejects private addresses, pins public DNS results, follows no redirects, sends
   no IAM/provider credentials, and streams into an anonymous private file.
3. Completed local actions report command metadata separately; stdout/stderr never
   enter this protocol. Silicon logs are serialized as JSONL with receipt sequence,
   client timestamp, actor, command and exit code. Creating the command artifact
   intent closes admission before the first page read. New late reports receive
   `409 report_window_closed`; exact duplicate reports still return their receipt.
   Logs are cooperative and can omit direct/offline activity. Interrupted rows from
   the former server-controller implementation are frozen only for migration
   compatibility. Carbon sessions do not create a command-log artifact.
4. The worker hashes the complete staged file before issuing a fresh request-bound
   IAM proof. It persists digest and size, rechecks ownership/cancellation, and
   uploads the same file handle with explicit Content-Length.
5. Verified receipts persist independently with encrypted links. A successful
   receipt is not uploaded again. A recording is available once all required
   artifacts are complete; an individual failure does not abandon the other
   artifact. Existing video and log links remain independently represented.

Stable names are `<session-id>.mp4` and `<session-id>-commands.jsonl`. The OBO proof
uses an empty destination path: Briefcase chooses its private application folder.
Browser persists the actual path from the receipt, rather than constructing remote
folders. The public `briefcase_path` stays empty until a verified video receipt exists;
legacy pending placeholders are also hidden.

## Retry and size policy

Transient transfer/proof failures receive bounded backoff, up to eight counted
attempts. Waiting for authorization does not exhaust that budget. Expired leases
can be reclaimed. Before another upload, newly staged bytes must match the already
persisted digest and size; changed content is rejected.

After correcting an eligible exhausted outage, proof, timeout, or size-limit
failure, the original initiator can run `sb recording send SESSION_ID`, or use
the Rust client/UI retry action. An active delivery authorization and the same
original IAM principal/membership are required. Retry preserves completed receipts
and bound bytes. Permanent source failures and locally hidden recordings are not
reset; repeating the request does not restart already active work.

This is **at-least-once delivery**, not exactly-once storage. If Briefcase committed
but its response was lost, another fresh proof may upload the same filename and
bytes again, creating an identical extra version. A consumed proof is never
replayed. The adapter itself does not automatically retry HTTP mutations.

`SB_RECORDING_MAX_BYTES` defaults to **512 MiB per artifact**. Increasing it changes
our bound, not Briefcase's limits or the proof deadline. The reusable adapter and
standalone example retain their separate 64 MiB default unless explicitly
configured. Staging is disk-backed; hashing uses a 64 KiB buffer. Download and
upload requests have bounded deadlines; the worker also bounds each job.

IAM proofs last at most 60 seconds. Briefcase stages the body before proof
verification, so upload must finish within the remaining proof lifetime. A
120-second HTTP deadline does not extend it. Slow transfer rejection was reproduced;
see [the slow-upload report](BRIEFCASE_SLOW_UPLOAD_RETEST.md).

`sb recording rm` hides the recording locally and cancels new pending delivery.
It does not call OBO deletion, delete the remote Briefcase file, or assert a remote
45-day trash guarantee. A late upload receipt is retained without making a hidden
recording visible again. Briefcase owns remote retention and directory layout.

## Validation and evidence boundaries

Focused tests cover private-source rejection, transient DNS recovery, bounded
streaming, exact JSONL preservation, proof/body binding, concurrent leases, stale
receipts, independent artifacts, immutable retry digests, late command completion,
local hiding, and original-owner authorization. Tests reproduced and fixed a
command-log failure that blocked delayed native video resolution. Full workspace
validation is reported separately after integration; old checkpoint test totals
are not evidence for the final worker.

The automatic flow now passed with real Carbon and Silicon browser sessions.
Both used CLI setup with a separate delivery SLT. The Carbon completed a real form
submission; the Silicon ran five commands and shared its live browser with a Carbon
through the authenticated iframe, which recorded the viewer association. Both
sessions were stopped, and all three artifacts completed on their first attempt.

Recipient-side Briefcase CLI stat, version listing, and download checks passed:

| Artifact | Downloaded size | Stored versions |
| --- | ---: | ---: |
| Carbon native video | 17,409,685 bytes | 1 |
| Silicon native video | 12,827,907 bytes | 1 |
| Silicon command JSONL | 622 bytes | 1 |

Every downloaded SHA-256 matched its delivery digest. Both videos decoded as H.264
at 1920×1080, with durations of 387.1 and 393.5 seconds. Exact hashes, timestamps,
and fixture details are in [the automatic live retest](AUTOMATIC_RECORDING_LIVE_RETEST.md).
This is bounded end-to-end verification, not prolonged load testing or a deployment.
The external proof deadline and observed incognito-proxy discrepancy remain.

Earlier explicit adapter tests also covered text, synthetic MP4, 300 MiB streaming,
byte ranges, and deliberately slow proof expiry.
[Adapter retest](BRIEFCASE_0_1_3_RETEST.md),
[native feature evidence](BROWSER_USE_FEATURES.md),
[external findings](BRIEFCASE_0_1_3_EXTERNAL_BUGS.md).

## Explicit sandbox verification harness

`crates/backend/examples/briefcase_upload.rs` remains a one-shot adapter harness.
It requires the two sandbox keys, `IAM_APP_ID`, `IAM_APP_SECRET`,
`BRIEFCASE_APP_ID`, `SB_AUTHTOKEN`, `SB_TEST_ORG`, `SB_TEST_ACTOR`,
`SB_TEST_UPLOAD_FILE`, and `SB_TEST_UPLOAD_NAME`. Optional settings are
`SB_TEST_UPLOAD_CONTENT_TYPE`, `SB_TEST_UPLOAD_MAX_BYTES`, `SILICON_IAM_URL`,
and `BRIEFCASE_URL`. Inject secrets from private storage, not shell literals.

```sh
cargo run -p silicon-browser-backend --example briefcase_upload
cargo run -p silicon-browser-backend --example briefcase_upload -- --proof-only
```

Proof-only mode requires the IAM sandbox and file/identity settings, omits the
Briefcase sandbox requirement, and never uploads. It prints proof ID, expiry, and
digest, not the opaque proof. This example is separate from automatic delivery.
