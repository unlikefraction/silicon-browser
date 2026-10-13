# Silicon Browser architecture

Browser 3.0.0 uses Silicon Accounts for identity and Silicon Apps for CLI
distribution. Carbons sign in on the website; Silicons exchange a Browser-bound
short-lived token from `silicon-accounts`. There is no organization selection.
Login persists across CLI invocations and website reloads. The CLI's
`browser logout` clears its saved local login.

The native Rust API on AWS owns authorization, profile/session lifecycle,
recording metadata and resource usage. SQLite stores these
records on private persistent storage. Accounts UUIDv4s are permanent owner and
access keys; public `c:` and `si:` handles are resolved selectors. Custodians
receive no implicit access to a Silicon's resources.

The CLI runs its bundled native controller on the caller's machine and
connects directly to the remote browser. Actions and their output stay outside
the API. Browser does not collect or send command telemetry, including to
Space Station. Older clients' command-report requests are rejected. The Next.js website, a static export built with Silicon UI, is served by nginx on the existing AWS
host and connects live viewers directly to the browser provider.

Profiles retain cookies, a stable fingerprint and a fixed proxy location. A
profile has at most one active session. Incognito sessions have no profile.
Access changes apply to subsequent API checks; an issued provider connection
remains a capability until the remote session ends.

Silicon Apps owns installation and updates. The CLI bundles its pinned
controller and needs no local Chromium. Search/fetch use the API's configured
provider key pool without requiring a profile or browser session. Automatic
Briefcase delivery through the retired IAM OBO protocol is unavailable.

Use the [README](README.md), [CLI reference](docs/CLI.md),
[HTTP API reference](docs/API.md), [deployment guide](docs/DEPLOYMENT.md) and
[Apps release guide](docs/APPS.md) for current behavior. The
[readiness index](docs/PRODUCTION_READINESS.md) distinguishes source checks from
dated production evidence. The [original IAM-era brief](docs/legacy/UNDERSTANDING-IAM.md)
is preserved as historical context.
