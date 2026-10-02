# briefcase-client

The official Rust client for [Silicon Briefcase][service], the
organization-scoped file service used by Carbons, Silicons, and IAM-authorized
applications.

Everything the service exposes to a client, and nothing it does internally.
Application behavior remains stateless: it holds no login session or API
cache — a `Config` goes in and a `Client` comes out. API calls never query a
package registry, run Cargo, or change the consuming project's lockfile.
Update the dependency explicitly and rebuild. `with_auto_update` and
`with_update_manifest` remain compatibility no-ops; `update_status()` always
reports `Disabled`. Honeycomb manages CLI installation and updates.

```toml
[dependencies]
briefcase-client = "2.0.0"
```

```rust
use briefcase_client::{Client, Config, Destination, ListEntries, Upload};

let client = Client::connect(
    Config::new("https://backend.briefcase.teamofsilicons.com/api/v1/", "tos")?
        .with_token(token),
)
.await?;

for entry in client.list_entries(&ListEntries::default()).await?.items {
    println!("{}", entry.path);
}

let stored = client
    .upload(&Upload::file(Destination::path("private/cos:tos/notes"), "./report.pdf")?)
    .await?;
```

`connect` verifies the service identity, selected/supported API major, and the
exact ID/version/method/path of every operation this client calls before the
first real call. Unknown operation IDs are additive; duplicate IDs are
refused. An incompatible pairing therefore fails at startup rather than
mid-request.

`Config::new` accepts exactly the compiled `/api/v1/` base and requires HTTPS,
with clear-text HTTP limited to `localhost` and loopback IPs for local tests.
The version response header and body must select the same API major.

IAM login uses `Client::login_with_slt`. For the default all-organizations flow,
create the anonymous client with `Config::for_sign_in(base_url)`. It accepts an
unscoped session, and `SessionTokens::organizations` contains the IAM handles
currently reachable by that token. After the caller chooses one, build a file
client with `Config::for_sign_in(base_url)?.with_organization(handle)?` and
attach the same access token. This switches request context without minting a
new login. A `Config::new(base_url, org)` login remains available when the
caller deliberately wants IAM to bind the token to one organization. The IAM
Application secret stays on the Briefcase backend. Testing environments use a
typed IAM app-secret `EnvironmentKey` in `Config::with_environment`,
independently of the bearer credential. Production-only management methods
create, inspect, re-pair, rotate, clean, retire, and restore those planes. Every
environment mutation has a caller-key `_with_key` variant for safely replaying
an unchanged request after an uncertain result; upload, entry update, and
version restore accept caller-owned keys too. Persist that `IdempotencyKey`
before the first attempt.

## Delegated operations

The SDK exposes delegated folder, listing, read, trash, invitation, link-access and resumable upload operations. `OboProof` now wraps a reusable IAM OBO access token. Prepare a typed manifest, pass the calling `ApplicationId` and a valid token, and preserve logical mutation IDs for retries. Tokens may be cloned while valid; the caller owns OBO refresh and secret storage. The SDK never sends its configured actor bearer with delegated credentials.

The legacy raw `create_file_on_behalf_of` operation is retired without transmitting bytes. Use reserve, capability transfer and commit. See the [3.0 migration guide](https://docs.briefcase.teamofsilicons.com/obo/) and [client guide](https://docs.briefcase.teamofsilicons.com/client/) for current examples. Documentation is published ahead of runtime rollout; verify `/api/version` before switching production clients.
