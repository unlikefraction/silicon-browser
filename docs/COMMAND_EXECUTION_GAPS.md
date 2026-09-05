# Local command execution — 2026-09-06

The September 5 review found an incorrect execution architecture: `sb run` sent commands to AWS, where a controller process executed them. That implementation has been removed. These were Browser implementation problems, not upstream IAM, Briefcase, or browser-provider bugs.

## Current path

`sb run` obtains an authorized session connection through the Rust client, then runs the native controller on the caller's machine. The controller connects directly to the remote CDP WebSocket. AWS never launches the controller, executes the command, proxies CDP, or relays stdout/stderr. A website live view similarly loads the authorized remote viewer directly in an iframe.

`GET /api/v1/sessions/{id}/connection` returns a sensitive connection capability with the immutable requesting principal and session expiry. The response is `Cache-Control: no-store`. Renewal requires current IAM authorization, an active, unexpired session, and the current profile ACL. Historical participation alone does not restore removed profile access.

The CLI keeps private state separately for each normalized backend URL. Local controller namespaces include backend, organization, immutable principal and session. A local lock serializes concurrent invocations in that namespace. Different machines can control the same remote session concurrently; AWS does not serialize their actions. The CLI caches a connection for at most 60 seconds and never beyond its session expiry.

## Setup and file paths

Setup installs or reuses the pinned native controller. It does not install Chromium, Node, npm, or a second browser runtime. File paths belong to the caller's machine: screenshots, PDF output, uploads, downloads, state files and local recording commands are passed to the controller. The actual browser remains remote, so each command is still subject to the pinned controller's remote-CDP support.

Connection replacement and controller/session lifecycle options remain owned by `sb`; use `sb session end` to end a managed session. This is an explicit exception to the literal “every agent-browser command” requirement, not a claim that every upstream flag is supported. There is no server-side file-transfer channel to build for these local paths.

## Command logs

After an action completes, the CLI saves a report locally and sends `POST /api/v1/sessions/{id}/commands`. This route only stores telemetry. The report contains a stable command UUID, command text, flags, client timestamps, exit code and a truncation indicator. It has no stdout/stderr fields; browser output stays local. For Silicon-initiated sessions, command text is encrypted at rest. Carbon-initiated sessions retain no command history.

Repeating an identical report returns its original receipt. Reusing its UUID with different metadata or another principal returns `409 command_report_conflict`. A failed upload never reruns the browser action. `sb session sync SESSION_ID` retries saved reports; explicit session end attempts to flush that session's queue first.

Reports are cooperative, not proof of every browser action. Sequence numbers reflect server receipt order, and timestamps come from clients. Direct CDP actions outside `sb`, browser UI actions, a client crash before saving its report, and reports still offline when archival begins can be absent from the log.

After a session ends, new reports remain admissible until its command archive intent is created. That transaction closes the log before the worker reads its first page, preventing a report from being acknowledged and then omitted by a racing snapshot. Exact retries remain acknowledgeable afterward; a new report receives `409 report_window_closed` and stays in the local queue. Archived bytes cannot be silently changed.

## Direct-capability boundary

An issued CDP or live-view URL is a credential. IAM cache invalidation and ACL changes control subsequent API access and capability renewal. They cannot retract a URL already issued by the remote provider. Closing or expiring the remote session ends that capability. Immediate revocation of an individual existing direct connection is not claimed.

## Verification

Seven API regressions cover secret handling, current ACL renewal, ending/expired sessions, rejection of execution-only payloads and output-bearing telemetry, idempotent reports, archive fencing, timestamps, and a 500-client metadata burst. The real file-backed SQLite WAL test performed 1,000 authenticated connection/report requests with fake IAM and browser providers and no browser operations: p50 431 ms, p95 509 ms, maximum 565 ms in the recorded local run. This is a synthetic control-plane measurement, not a deployed SLA or a 500-browser provider-capacity test.

## Production integration regression: HTTPS CDP discovery

The first production local-controller walkthrough exposed an internal scheme mismatch: session creation accepted the provider's HTTPS CDP discovery endpoint, but connection renewal only accepted WSS and returned `provider_failure` before running any local command. A read-only provider lookup confirmed the active session used HTTPS discovery. This was a Browser validation bug, not an upstream malformed response.

Creation and renewal now use the same secure CDP validator, accepting HTTPS discovery and direct WSS endpoints. The Rust local controller accepts those forms as well. A regression issues the stored HTTPS capability with `no-store` without calling the provider; the full eight-test control group passes. Deployment and subsequent real command success need their own retest evidence.
