# Deployment layout

Vercel serves `https://browser.teamofsilicons.com`. A dedicated AWS host runs the native API daemon at `https://backend.browser.teamofsilicons.com`. Clients control remote browsers directly; the AWS host has no controller or Chromium installation and serves no frontend assets. Containers are not required.

## Traffic paths

| Operation | Path |
| --- | --- |
| IAM auth, profiles, session lifecycle, connection grants, usage and log metadata | CLI or frontend → AWS API |
| Browser actions, CDP responses and browser file operations | Local CLI controller ↔ remote browser |
| Interactive live view | Frontend iframe ↔ remote viewer |
| Completed recordings | Remote recording source → bounded AWS staging → initiator's Briefcase |
| Search/fetch | CLI or Rust client → AWS key pool/queue → search provider |

“Metadata API” describes the browser control plane, not a claim that all server requests are tiny: completed recording delivery and the existing shared-key search/fetch service are separate server-side integrations. Browser action traffic and live-view traffic are never proxied through AWS or Vercel.

## Vercel frontend

Set the project Root Directory to `frontend`, Framework Preset to **Vite**, and Node to **24.x**. The frontend uses SolidJS, TypeScript and Vite. `frontend/vercel.json` defines the build, `dist` output, security headers, and rewrites for `/auth/callback` and `/sessions/:id/live`. There is no API proxy or serverless function. See [frontend setup](../frontend/README.md).

The public API origin defaults to `https://backend.browser.teamofsilicons.com`. `SB_PUBLIC_BACKEND_URL` is build-time public configuration, never a place for credentials. Provider keys, IAM application secrets, webhook secrets and test-environment keys belong only on AWS.

## Native AWS backend

Build `silicon-browser-backend` for the host's operating system and architecture. Run it under the host's service manager with startup on boot and restart on failure. The daemon stays in the foreground and handles SIGTERM.

- Set `SB_ORIGIN=https://browser.teamofsilicons.com`; this exact frontend origin controls CORS and generated live links.
- Set the server values in [`.env.example`](../.env.example). Process environment overrides `.env`, which is read relative to the service working directory.
- Keep SQLite on a private persistent local data volume and preserve the same `SB_ENCRYPTION_KEY` across upgrades. SQLite uses WAL and an eight-connection pool. Do not place this database on a shared network filesystem or infer multi-host write coordination from the single-host tests.
- Provide writable private temporary storage for bounded recording staging and outbound HTTPS access to IAM, recording sources, Briefcase and configured providers. Terminate public TLS at the ingress and forward requests to the configured `PORT`.
- `/healthz` reports process availability. It does not verify IAM, provider quotas, Briefcase, disk capacity or webhook delivery. Back up the database consistently, including its WAL state, and preserve the encryption key separately with restricted access; verify restoration before relying on a backup.

Background work in this same daemon handles expiry/recovery, independent IAM delivery authorization, and completed recordings. It is not a fleet of browser controllers and does not capture another recording. Briefcase authorization and configuration are required before new managed sessions can start.

The checked-in [CloudFormation template](../deploy/aws-native.json) provisions a private ARM64 EC2 host, ALB target/rule/certificate attachment, least-privilege instance role, encrypted retained storage, private artifact/backup bucket and runtime secret. Populate that secret with a JSON object containing the server environment values; never include CLI credentials or test keys in production.

After building the ARM64 binary, deploy through S3 and SSM:

```sh
CARGO_INCREMENTAL=0 cargo zigbuild --release --locked --target aarch64-unknown-linux-gnu.2.34 -p silicon-browser-backend
python3 deploy/release-native.py
```

The [installer](../deploy/install-native.sh) verifies startup through `/healthz`, preserves the previous release and restores its environment/unit files on failure. SQLite migrations must remain compatible with the previous binary; rollback does not reverse database changes. The systemd service runs as an unprivileged user with restricted writable paths. A fifteen-minute timer creates an online SQLite backup, checks its integrity and uploads it to the private bucket. Backups expire after 90 days. A separate restore check must compare schema/migrations and verify the restored database; successfully uploading a file is not enough.

## IAM webhook and authorization cache

Configure the IAM application to deliver to `https://backend.browser.teamofsilicons.com/webhooks/iam/`. Both forms of the route are supported. Set `IAM_WEBHOOK_SECRET` and its matching positive `IAM_WEBHOOK_KEY_VERSION` on the backend. Without a configured secret, this endpoint is not mounted.

The IAM SDK verifies signatures, timestamps, event identity and key version against the exact request bytes. Production rejects test-plane envelopes; paired test deployments verify their configured IAM environment. A verified delivery invalidates all local authorization snapshots before acknowledgement. Event IDs make receipt storage idempotent; only minimal receipt metadata is retained for 45 days, not raw webhook bodies. A retry also invalidates the cache.

The production cache holds at most 4,096 token/organization snapshots for 15 seconds, bounded by token expiry. Concurrent misses share work, and a webhook racing a lookup fences its stale result. Without a successful webhook, authorization changes can remain cached for that interval. The webhook does not close browsers or revoke previously issued remote CDP/live URLs; see [the direct-capability boundary](COMMAND_EXECUTION_GAPS.md).

Verify a real signed IAM delivery reaches the configured production endpoint, an invalid signature is rejected, and the receipt is recorded. Local signature tests do not establish production registration or delivery. A single process owns this cache; additional backend instances would need coordinated invalidation.

## Frontend authentication and local verification

The frontend sends explicit bearer/organization headers directly to the API, omits cookies and refuses redirects. Exact-origin CORS exposes `x-sb-auth-rejected` for one safe retry after a pre-handler authentication rejection. IAM sign-in returns to the Vercel callback; its nonce and popup source are checked. Callback tokens and live grants are removed from the URL immediately, and session credentials stay in memory.

For local verification, run the backend on `http://127.0.0.1:8091` with `SB_ORIGIN=http://127.0.0.1:8092`, then:

```sh
npm ci --prefix frontend
SB_PUBLIC_BACKEND_URL=http://127.0.0.1:8091 npm run build --prefix frontend
node frontend/dev.mjs
```

Rebuild without the override for production. Actual service installation, DNS/TLS, IAM redirect/webhook registration, account quotas, package publication, sustained load and backup restoration require operational evidence; this layout document does not certify them. Vercel configuration follows its [project configuration](https://vercel.com/docs/project-configuration/vercel-json) and [rewrite](https://vercel.com/docs/routing/rewrites) contracts.
