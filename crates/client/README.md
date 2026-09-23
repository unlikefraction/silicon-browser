# silicon-browser

The stateless, primary interface to Silicon Browser. Construct an explicit [`Auth`] and [`Client`],
select an organization, then call the same operations exposed by the `browser` command.

```rust,no_run
use silicon_browser::{Auth, Client};

let auth = Auth::new("oat_short_lived_value")?;
let browser = Client::new("https://backend.browser.teamofsilicons.com", auth)?.org("tos")?;
for profile in browser.profiles(None)? {
    println!("{} {}", profile.id, profile.name);
}
# Ok::<(), silicon_browser::Error>(())
```

The same client also redeems authenticated live-link fragment grants through
`Client::redeem_live`, which associates the viewer without putting the grant in a request URL.

`Client` never reads environment variables or credentials from disk, and never refreshes credentials implicitly. Authentication recovery can be explicitly supplied through `HttpTransport`.

Browser commands execute on the caller's machine. `Client::session_connection` returns a sensitive direct connection after access checks. Pass it to `controller::LocalController`, which invokes the installed native controller with the connection only in its environment. Screenshots, PDFs and local recordings write locally. Uploads transport local file bytes. Downloads copy ordinary same-origin HTTP link targets with browser credentials, or blob/data links, into a local file finalized only after byte verification. Button/script/POST downloads, cross-origin frames, `wait --download` and `--download-path` are unsupported. The backend receives no browser control traffic, file bytes or command output. Local daemons exit after five idle minutes.

```rust,no_run
use silicon_browser::{Auth, Client, controller::LocalController};
let browser = Client::new("https://backend.browser.teamofsilicons.com", Auth::new("oat_value")?)?.org("tos")?;
let connection = browser.session_connection("session-id")?;
let namespace = LocalController::namespace(browser.base_url(), "tos", &connection.principal_id, &connection.session_id);
// The caller creates this explicit configuration file containing {}.
let execution = LocalController::new("/path/to/sb-browser-engine").run(
    &connection, &namespace, std::path::Path::new("/private/controller.json"),
    "screenshot ./capture.png", &[], |event| println!("{event:?}"),
)?;
let report = execution.command_report("screenshot ./capture.png", &[]);
// Retain the exact report on failure. Retrying report delivery never repeats the action.
browser.report_command("session-id", &report)?;
# Ok::<(), silicon_browser::Error>(())
```

`setup::ensure_runner` installs the pinned native controller into an explicit caller-owned directory and verifies its compiled-in release digest. It requires neither Node/npm nor local Chromium. The `setup_controller` example exercises this installation separately from authentication.


## IAM testing environments

The API verifies the test app secret with IAM; no root key is required. Select
its returned environment UUID and keep that transport attached to every call:

```rust,no_run
use std::sync::Arc;
use silicon_browser::{Auth, Client, HttpTransport};
use silicon_browser::shared::{AuthExchangeRequest, TestingCredentials};

let root = "https://backend.browser.teamofsilicons.com";
let credentials = TestingCredentials {
    app_secret: "ask_test_application_secret".into(),
    iam_test_key: None,
    briefcase_test_environment_key: None,
};
let environment = Client::testing_context(root, &credentials)?;
let base = format!("{root}/testing/{}", environment.environment_id);
let session = Client::exchange_testing(&base, &AuthExchangeRequest {
    short_lived_token: "si:worker".into(),
    org_id: Some("tos".into()),
}, credentials.clone())?;
let transport = Arc::new(HttpTransport::default().with_testing(&base, credentials)?);
let browser = Client::with_transport(base, Auth::new(session.access_token)?, transport)?.org(session.org.id)?;
let profiles = browser.profiles(None)?;
# Ok::<(), silicon_browser::Error>(())
```

Test actor IDs or test SLTs use the same authenticated APIs as production.
Creating browser sessions additionally requires the imported Briefcase IAM app
secret (`ask_` plus 43 base64url characters) from the same world, passed through
`briefcase_test_environment_key`, and recording authorization. See the [API and CLI guide](https://browser.teamofsilicons.com/docs).
