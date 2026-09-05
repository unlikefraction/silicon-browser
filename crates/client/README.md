# silicon-browser

The stateless, primary interface to Silicon Browser. Construct an explicit [`Auth`] and [`Client`],
select an organization, then call the same operations exposed by the `sb` command.

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
