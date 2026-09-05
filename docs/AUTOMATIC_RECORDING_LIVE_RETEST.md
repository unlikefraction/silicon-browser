# Automatic recording delivery: live retest, 2026-09-05

> Historical evidence from before the September 6 local-controller and SolidJS rewrite. The native recording/Briefcase observations remain valid for those runs; the old controller placement, UI implementation and test totals are not current architecture claims. See [current readiness](PRODUCTION_READINESS.md) and [execution path](COMMAND_EXECUTION_GAPS.md).

The complete Browser session-end path passed against real Browser Use, IAM, and Briefcase services. The local Browser backend used the existing paired IAM/Briefcase testing environments; mock responses were not used for these walkthroughs. Both metered browser sessions were explicitly ended.

## Human and Silicon walkthroughs

- Installed IAM CLI 1.2.2, Briefcase CLI/client 0.1.3, IAM Rust SDK 1.2.1, and the repository's exact managed runner version, agent-browser 0.36.0. The earlier global runner was 0.27.0; this test used an isolated 0.36.0 installation.
- Actual `sb setup` for Carbon `sbauditfive` and Silicon `browser-audit-reader:tos` succeeded. Each used a fresh login SLT and a second fresh Browser SLT for the backend's independent recording authorization. Backend access became active for both.
- Carbon created a named, described incognito session through `sb`, opened Selenium's public test form, filled text, selected an option, checked a box, submitted it, and read “Form submitted / Received!”.
- Silicon created a second named, described session, opened example.com, took an interactive snapshot, followed its IANA link, and read the title.
- Silicon issued an `sb session live` link. Carbon opened the link on Browser's own frontend and authenticated with a fresh SLT. The embedded Browser Use viewer loaded and worked. Carbon used its address bar to navigate back to example.com; Silicon's subsequent CLI title read confirmed that change in the same remote browser. Browser persisted Carbon in the session's participants.
- Both sessions were explicitly ended through the initiating identity's CLI. Native recording URLs became available asynchronously. Browser's workers obtained fresh proofs, uploaded the artifacts, and persisted their receipts without a manual recording-upload command.

## Verified receipts

| Initiator | Artifact | Bytes | Briefcase entry | Attempts / versions |
| --- | --- | ---: | --- | --- |
| Carbon | H.264 MP4 | 17,409,685 | `01a0728d-d539-7971-bbfe-9d1b127b4765` | 1 / 1 |
| Silicon | H.264 MP4 | 12,827,907 | `01a0728e-84a7-7053-acf4-f44f27e505af` | 1 / 1 |
| Silicon | Commands JSONL | 622 | `01a0728d-ea93-73d0-812f-05beb884bfb3` | 1 / 1 |

Carbon session: `01a07287-0c64-70f3-af89-ccec61f62d2e`.
Silicon session: `01a07287-ce57-7833-97a6-b51d7eae3249`.

Both Browser recordings reached `available`. Briefcase selected each destination beneath `private/<initiator>/apps/tos>browser/`; Browser did not choose a directory. Carbon has no command-log artifact. Silicon's log contains all five CLI commands with exact strings, ordered sequences, actor identity, timestamps, and exit code zero.

Each initiating identity used the real Briefcase CLI to `stat`, list `versions`, and `get` its files. Downloaded lengths and SHA-256 digests exactly matched the worker's persisted proof-bound bytes:

- Carbon video: `2128ae9f66cecf76b94ac1405a5df6296090dec561f4fa97a6a000688cde4eb3`
- Silicon video: `734edb12c3e3397f5d65e310601bcc74817f5653d6281b32ce151190640bba2b`
- Silicon command log: `9e2e1c836cfffc5cb24e5fd6a7eecd6f1bb0c218ed58850707fda996b2eca764`

`ffprobe` found 1920×1080 H.264 video lasting 387.1 and 393.5 seconds. Both entire files decoded without errors through ffmpeg. A viewed frame from Carbon's downloaded recording shows the successful form submission. The Browser frontend displayed both recording entries, the video links, and Silicon's command-log link.

## Scope and external observations

This proves real automatic recording delivery and live Carbon handoff for the tested flows. It does not certify unlimited recording size, prolonged multi-instance operation, or production deployment. The separate [300 MiB multipart test](BRIEFCASE_300_MIB_CLI_RETEST.md) and [slow-transfer test](BRIEFCASE_SLOW_UPLOAD_RETEST.md) remain applicable.

Browser Use again reported nonzero proxy traffic/cost for both incognito requests despite explicit `proxyCountryCode: null`. Browser preserves these reported counters and warns; the upstream routing/metering discrepancy remains open in [the provider report](BROWSER_PROVIDER_FINDINGS.md).

Secrets, browser endpoint grants, saved CLI credentials, and raw diagnostic payloads remain in a private temporary directory and are not included here. No issue was posted upstream, production environment changed, or library published.

## Additional UI defect caught and fixed

The real browser retirement walkthrough found that the form helper assigned `type` to a textarea. `HTMLTextAreaElement.type` is read-only, so session descriptions and end/retirement notes failed to render. The helper now assigns type only to input elements. A regression models this actual DOM property and checks both session creation and retirement forms; the frontend suite now has ten passing tests. Profile creation and ACL edits were also exercised: Silicon could see the profile while authorized and immediately lost listing visibility after Carbon removed it.

## Restart and authorization controls

After a graceful backend restart using the same SQLite database and encryption key, both background grants remained active, all three completed receipts were retained, and each real Briefcase file still had one version. No completed artifact was uploaded again. The fixed UI retirement form successfully retired the test profile, preserving its fingerprint and US location.

Disabling Carbon background authorization correctly stopped delivery authority and rejected a new session before any session row or provider browser was created. The live test also exposed IAM normal-revocation behavior that invalidates sibling Browser OATs while leaving their ORTs refreshable. An actual refresh of the independent Carbon CLI family restored access. See [the new IAM report](IAM_1_2_2_EXTERNAL_BUGS.md) for the source-confirmed scope and Browser recovery changes.

## End-to-end verification after IAM recovery fix

The repeat used a real replacement backend grant and waited for its superseded family to be revoked. The old Carbon CLI access token was confirmed rejected with the pre-handler marker. CLI and the already-open UI recovered automatically through their own refresh credentials, without a fresh sign-in. Carbon created and ended session `01a0729c-346f-7802-9128-931739964795` using the UI, including its description and end-note forms, and navigated example.com through `sb run`.

The background worker then rotated credentials within the **same persisted grant**, recovered from the sibling-access invalidation, and delivered native video on its first artifact attempt. Briefcase entry `01a0729d-a988-76c1-95a0-1114ea740bb2` contains **590,227 bytes**, SHA-256 `f50e8e6d0208dc854f1b1d80a407c44ab2b06cd5fd7e797b9744e22436a1f4c6`. Real owner CLI download matched exactly and versions returned one entry. Grant status remained active; the session and recording finished ended/available. This live check covers the final CLI, UI, and background recovery implementations together.

Cleanup: all three test browser sessions were ended, all four artifact jobs completed, and no authorization operations remained pending. The local test backend and local UI browser were stopped after verification. Briefcase artifacts and the supplied paired environments were preserved.

## Separate frontend hosting retest

The frontend now lives in its own static Vercel project; AWS runs only the native API and background tasks. A separate local preview on `http://127.0.0.1:8092` successfully signed a real Carbon into the backend on `http://127.0.0.1:8091`, loaded the existing profile and all three recording entries, and exposed usable recording/log links without CORS or page errors. Browser JavaScript could read `x-sb-auth-rejected: 1` on an unauthenticated API 401. API resource requests went exclusively to the backend origin.

An unauthenticated `/sessions/route-check/live#grant=synthetic-route-check` navigation loaded the frontend and removed its fragment before sign-in. This tested routing and fragment handling; no new live provider session or grant redemption was needed. The previous real Carbon handoff remains documented above. The API's `/` and old asset routes returned JSON 404, while the frontend's `/api/v1/me` returned 404 without proxying. CORS responses used only the configured frontend origin, so an unrelated origin could not obtain permission.

The test browser and both local servers were stopped, and frontend artifacts were rebuilt with the production API origin. No provider sessions or recording uploads were created during this retest. The AWS and Vercel deployments themselves have not been performed. The final automated suite has 306 passing Rust tests and 19 passing frontend tests.
