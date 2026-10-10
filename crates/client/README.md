# Silicon Browser Rust client

A stateless metadata API client plus a local native browser controller.
Authentication uses Silicon Accounts. The application UUID is the stable
identity key; c:/si: handles are display selectors.

```rust,no_run
use silicon_browser::{Auth, Client};
use silicon_browser::shared::AuthExchangeRequest;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let backend = "https://backend.browser.teamofsilicons.com";
let info = Client::accounts(backend)?;
assert_eq!(info.app_id, "browser");
let session = Client::exchange(backend, &AuthExchangeRequest {
    short_lived_token: "slt_from_silicon_accounts".into(),
    org_id: None,
})?;
let client = Client::new(backend, Auth::new(session.access_token)?)?.org(session.org.id)?;
let profiles = client.profiles(None)?;
println!("{} profiles", profiles.len());
# Ok(())
# }
```

The retained `org` API selects Browser's application workspace (`browser`),
not a Silicon Accounts organization. The library owns no credentials or local
state. Callers securely store rotated refresh tokens and must not replay an
uncertain token exchange. The CLI implements local renewal locks and persistence.

`session_connection` returns a direct remote capability bound to a permanent
Accounts UUID. Treat it as a credential. `LocalController` executes actions on
the caller's machine and exposes streaming callbacks. Command reports contain
metadata, never browser output, and can be retried by their stable command UUID.

Inject `Transport` for local fakes. HTTP redirects are refused. `HttpTransport`
can recover once from a marked, pre-handler authentication rejection using a
caller-owned renewal callback; it never owns a refresh token itself. IAM test
world and OBO delivery methods are retired in 1.1.0. See the
[CLI reference](../../docs/CLI.md) and [deployment](../../docs/DEPLOYMENT.md).
