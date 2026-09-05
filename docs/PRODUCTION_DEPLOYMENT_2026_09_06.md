# Production deployment verification — 2026-09-06

## Deployed services

- Dashboard: <https://browser.teamofsilicons.com>, SolidJS/TypeScript/Vite on Vercel. Deployment `dpl_7jTc4fGE8rdhkgugb8Qg7f14ZcAd`, with the shared IAM mark and requested Space Station wordmark.
- API: <https://backend.browser.teamofsilicons.com>, native ARM64 daemon on a private EC2 `t4g.medium` in `us-east-1`, behind the existing HTTPS ALB. CloudFormation stack `silicon-browser-production`.
- Current native release: `20260905-ab77a4c54d708b50`. Persistent SQLite WAL storage, encrypted retained disk, private runtime configuration from Secrets Manager, automatic service restart and fifteen-minute online backups.
- Both domains pass TLS validation. The ALB target is healthy. Public dashboard HTML uses no-store caching, hashed assets are immutable, and the live iframe connects directly to the remote viewer.

The AWS host does not execute browser commands or relay CDP/live traffic. Completed MP4 delivery and search/fetch remain separate backend integrations.

## Live checks

Fresh production IAM SLTs were exchanged by the real CLI and by the dashboard's nonce-bound popup callback. The actual IAM email sign-in form was rendered separately; automated callback testing used an SLT minted by the already-authenticated IAM CLI. Browser credentials remained in frontend memory, with no localStorage/sessionStorage entries. Separate backend recording authorization became active.

A real remote session was created, navigated by the native local controller to `https://example.com`, inspected with a snapshot and captured to a local PNG. The screenshot was verified as a readable 17,278-byte image. The dashboard's live iframe visibly reflected that exact navigation; fullscreen worked with no provider-hostname navigation links or JavaScript errors.

The production check caught a Browser validation bug: session creation accepted the provider's HTTPS CDP discovery URL, while renewal/client validation only accepted WSS. Backend and client now accept their approved HTTPS/WSS connection forms, and the live command passed after deployment. The backend regression also verifies no-store output and no provider execution during connection renewal.

Native upload initially reported a filename without transferring local bytes; the selected file could not be read. The local adapters now transfer and verify bytes through the direct browser connection. A production 1,048,832-byte binary passed upload, browser-side SHA-256, the pre-existing page change handler, blob-link download and local hash comparison. All copies matched `dd7e5c49d123e860c8bb7016bada722b5d0baa37ef8b19d5e270cf2a3000c31d`. Screenshots/PDFs and the bounded upload/link-download adapters are local; script/POST/button and cross-origin downloads, `wait --download`, and remote download-path overrides explicitly reject instead of returning false local-file success. See [the native download scope](AGENT_BROWSER_0_36_0_DOWNLOAD_GAPS.md).

Production search returned ranked results and fetch returned the rendered text of `example.com` without a profile/session.

The first session ended successfully. Automatic recording delivery initially found an empty production IAM Briefcase OBO catalog. Registering Briefcase's existing `briefcase.files.create` endpoint (`POST /api/v1/obo/files`, string metadata `path`, `name`, `content_type`) allowed the durable retry to complete. The resulting 4,779,477-byte MP4 became available in the initiating member's Briefcase. The owner downloaded it through the real Briefcase CLI; SHA-256 `5885284639e321a13b5d123edebf1e37195c9c58443601e175a495049980f1df` matched the durable proof-bound artifact, and a complete FFmpeg decode reported no errors. Carbon command history remains intentionally omitted by the original product contract.

The second production session ended at 20:20:12 UTC on September 5, and its 646-second recording became available. Final Browser session listings showed both sessions ended, and the authoritative provider account reported zero active browsers.

## Webhook status

The deployed `/webhooks/iam/` receiver accepted a locally signed production-format verification event with secret version 3, acknowledged the identical retry as a duplicate, and rejected modified bytes with HTTP 401.

The owner supplied the required email step-up. IAM activated the exact `/webhooks/iam/` destination with signing-secret version 3 and endpoint version 4. Application display-name changes did not route deliveries to this subscriber. With the owner's explicit approval, a temporary suffix was added to their own IAM profile display name and immediately removed. The backend durably accepted two actual IAM-originated `carbon.updated.v1` deliveries at 20:20:48 and 20:20:49 UTC on September 5. Both passed signature verification and executed authorization-cache invalidation. The original profile name was restored successfully.

## Recovery and capacity evidence

An online production backup was downloaded into private temporary storage. SQLite integrity and foreign-key checks passed; all six SQLx migration checksums and all thirteen application-table schemas matched the release. The restored temporary copy was deleted after checking; the private encrypted/versioned S3 backup remains under its 90-day lifecycle.

The architecture's local test used 500 simultaneous clients and 1,000 connection/report requests against file-backed SQLite, with fake IAM/provider adapters. Its p50 was 431 ms, p95 509 ms and maximum 565 ms. This establishes local control-plane contention behavior, not a quota for 500 paid browsers or a cloud latency guarantee.

A bounded production burst returned 498 HTTP 200 responses, one client connection timeout and one canceled request. Health remained HTTP 200, the backend did not restart, and kernel listener-overflow counters stayed zero; it is not a complete 500-request pass. See [the request-burst evidence](PRODUCTION_METADATA_LOAD.md).

The configured provider account reported a concurrency limit of **3**, independently of Browser control-plane tests. The owner will manage the plan upgrade; Browser makes no purchase. [Account capacity evidence](BROWSER_PROVIDER_CAPACITY.md) distinguishes the live API entitlement from advertised plans. The new `Client::usage_limits()`, `sb usage limits` and dashboard Usage display read the authoritative account limit, cached for up to one minute. The deployed CLI returned `concurrent_browser_limit: 3`; a plan change will be reflected by a later refresh, without hardcoding a plan tier.

Final source checks passed **318 Rust tests**, **18 frontend tests**, formatting, warning-denied Clippy and documentation builds. [GitHub CI for implementation commit `3937ab9`](https://github.com/unlikefraction/silicon-browser/actions/runs/33989780253) passed both Rust and frontend jobs. The real local-file harness also tested authenticated same-origin downloads, hidden/ref targets, concurrent clients, failure cleanup and metadata-only reports. A final production snapshot-reference upload/download matched SHA-256 `b75b22fa66be80a71dd7851ca4eae9c6af480bed56a35766693ed5ee11d91642`. Desktop and 390-pixel mobile dashboard checks passed without horizontal overflow or JavaScript errors.

## Client publication and local setup

Version **0.1.0** of [silicon-browser-shared](https://crates.io/crates/silicon-browser-shared), [silicon-browser](https://crates.io/crates/silicon-browser) and [silicon-browser-cli](https://crates.io/crates/silicon-browser-cli) was published to crates.io. Source is on the repository's `v2` branch. Prebuilt macOS ARM64/Intel and Linux ARM64/x86-64 CLI archives are provided in the [managed v0.1.0 release](https://github.com/unlikefraction/silicon-browser/releases/tag/managed-v0.1.0), with SHA-256 checksums. Linux archives require glibc 2.34 or later. The separately named release preserves the legacy fork's existing release channel.

The user's `sb` command now points to the native 0.1.0 executable. Production setup completed using the default state directory and the controller's normal checked download, without a controller override. It reported controller version 0.36.0, organization `tos`, and active recording delivery. The installed command successfully read production capacity and listed both verification sessions as ended. The previous executable target was retained locally.

[Integration configuration findings](DEPLOYMENT_EXTERNAL_FINDINGS.md) record the IAM/Vercel/DNS workarounds without credentials.
