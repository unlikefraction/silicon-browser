# Briefcase integration: remaining limits and verification

The automatic delivery worker and backend-owned authorization lifecycle are now
implemented. Earlier reports saying the outbox has no consumer or that IAM lacks
background authorization support are superseded. See
[the current flow](BRIEFCASE_INTEGRATION.md). Automatic live delivery passed for
real Carbon and Silicon sessions; this document does not claim production deployment.

## Implemented contract

Browser copies native Browser Use recordings and, for Silicon initiators, JSONL
logs of cooperatively reported local commands. Browser actions and stdout/stderr
remain local/direct; the worker only transfers completed artifacts. A separate fresh SLT enrolls the backend's own encrypted application
session; durable refresh does not share the CLI's ORT. Jobs retain original IAM
principal and membership IDs. Leased artifact work binds stable names, SHA-256,
size, and current authorization before a single proof-bound HTTP upload.

Briefcase delegates `briefcase.files.create` through `POST /api/v1/obo/files`.
The exact body and destination metadata are bound by IAM; headers carry the OBO
proof, issuing app, and organization, without bearer authentication. Briefcase's
empty-path behavior selects the member's private app folder. Folder creation and
remote deletion are intentionally outside Browser's delegated scope.
[Briefcase OBO handler](https://github.com/teamofsilicons/silicon-briefcase/blob/5f27dd915c88d2cb839de368f273436f9254f9bf/src/api/handlers/obo.rs),
[IAM OBO implementation](https://github.com/teamofsilicons/silicon-iam/blob/bab75c0a909481ad2d5dca5bd7d52df08476eaf3/src/features/applications/obo.rs).

## Remaining product and operational limits

- **Pending path representation corrected:** the public `briefcase_path` remains
  a string for compatibility but is empty until a verified video receipt exists.
  Legacy invented pending paths are masked; actual paths come from Briefcase.
- **Explicit terminal recovery implemented:** `sb recording send SESSION_ID`
  (also Rust client/UI and POST `/api/v1/recordings/{id}/retry`) retries eligible
  exhausted outage/proof/timeout failures or a corrected size limit. It requires
  the original initiating principal/membership and active delivery authorization.
  Completed receipts and bound bytes stay unchanged; permanent unavailable,
  invalid, or changed sources and locally hidden recordings cannot be reset.
- **Slow upload deadline:** proof verification occurs after body staging, within
  an at-most-60-second proof lifetime. The worker's 512 MiB default and configurable
  size bound do not guarantee that a particular connection can transfer the file
  fast enough. There is no Browser-integrated delegated multipart protocol.
- **Uncertain commit:** OBO does not expose a delegated receipt lookup. The worker
  uses fresh proofs with stable bytes and names after an unconfirmed upload;
  duplicate versions are possible. This is the accepted at-least-once policy,
  not a claim of exactly-once delivery.
- **Historical jobs:** sessions without original IAM owner bindings are not
  automatically adopted on reauthorization. Any migration needs independently
  verified owner identity rather than matching a reused public handle.
- **Local hiding:** Browser cancels new work and hides links. It does not delete
  Briefcase files or implement remote trash retention through OBO.
- **Report closure:** command archive intent creation freezes log admission before
  the worker reads any pages. Exact retries are acknowledged, but new reports
  arriving later receive `409 report_window_closed` and stay queued on the client.
  Offline/direct actions are not guaranteed to appear in this cooperative log.

Pending-path correction and explicit recovery are implemented Browser behavior,
not missing Briefcase capabilities. Directory ownership and lack of OBO deletion
are accepted behavior.

## Verified flow and remaining release work

The final worker passed real CLI setup with a separate delivery SLT, real browser
activity, automatic receipt persistence, and recipient-side exact retrieval. The
Carbon video, Silicon video, and Silicon JSONL all succeeded on the first attempt;
each had one Briefcase version and matching downloaded SHA-256. Both MP4s were
validated with ffprobe. The Silicon live handoff also recorded its Carbon viewer.
[Automatic live retest](AUTOMATIC_RECORDING_LIVE_RETEST.md).

Prolonged load, operational rollout, and production deployment remain separate
work. Crash/retry/ownership isolation has focused regression coverage, but this
bounded live success does not prove every operational failure mode in production.
The 60-second proof deadline and Browser Use incognito-proxy discrepancy remain
external constraints. [Provider findings](BROWSER_PROVIDER_FINDINGS.md).

Prior live evidence includes text/MP4/300 MiB streaming readback and proof expiry
under deliberately slow transfer. The hosted Briefcase edge required a descriptive
User-Agent; the adapter now sends one. The paired test app's endpoint catalog was
registered in the test plane only.
[Large-upload retest](BRIEFCASE_0_1_3_RETEST.md),
[slow-upload reproduction](BRIEFCASE_SLOW_UPLOAD_RETEST.md),
[external findings](BRIEFCASE_0_1_3_EXTERNAL_BUGS.md).
