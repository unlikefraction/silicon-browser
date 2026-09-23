# silicon-iam-client

Batch login authenticates once:
select organizations per app, and receive up to 100 independent SLTs. See the
[batch login guide](../BATCH_LOGIN.md) for browser, API, Rust and CLI examples.

A Rust client for the [Silicon IAM](https://backend.iam.teamofsilicons.com) API:
identity, organization governance, application login, and delegated access.

The public integration surfaces have typed methods here, using the contract's
own shapes. The wire types are generated from `docs/openapi.yaml`, and changes
to that contract produce a reviewable source diff. Platform administration,
provider callbacks, and browser navigations remain outside this crate.

```toml
[dependencies]
silicon-iam-client = "4.0.0"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Version 2 uses public membership IDs such as `saket[tos]` and
`helper:tos[tos]` in place of UUID membership references. Upgrade membership
arguments and stored public references to strings. Existing official 1.x clients
retain their UUID wire representation during rollout, including introspection;
all current clients and ordinary API requests use the canonical IDs. Existing
signed v1 webhook envelopes retain their UUID contract for consumer compatibility.
The database mapping covers every existing membership without replacing private
foreign keys or invalidating sessions.

The client speaks HTTP API major `v1` and requires Rust 1.98 or newer.
The crate SemVer and HTTP API major are separate: upgrading the crate within
the 2.x line does not select a different wire major. `Client::new` and
`ClientBuilder::build` perform no network handshake; call
`client.system().negotiate().await?` during startup when you want an upfront
compatibility check. It validates the service identity, ordered version
catalog, highest shared selection, selected-version header/body agreement, and
the required `Vary` header. A `406` becomes `Error::ApiVersionUnsupported`;
an inconsistent success becomes `Error::Decode`. Every request advertises
`v1`.

This guide describes the current official API contract. Use the matching client, CLI,
and backend release together. The complete hosted manual is at
[docs.iam.teamofsilicons.com/client](https://docs.iam.teamofsilicons.com/client).

Applications declare IAM and external endpoint permissions separately from webhook event
subscriptions. Direct IAM login reviews those permissions before organization selection.
The SDK exposes scope review discussions, application bundles, cross-organization OBO,
and application-initiated test environments with transitive dependency provisioning.

The service URL must use HTTPS, except for literal `localhost`, `127.0.0.1`,
or `::1` during local development. The builder rejects missing hosts, embedded
credentials, port zero, queries, and fragments. Requests do not follow
redirects, so IAM authority stays on the configured endpoint, and every
response body is capped at 4 MiB. An oversized response becomes
`Error::ResponseTooLarge`.

## Application IAM reads and writes

For an ordinary application bearer, use `client.application_reads()` and
`client.application_mutations()` to preserve scope-filtered responses. A write
permission does not grant permission to read the full resource: mutation
successes may contain only IDs/version/status or `{}`. The projected mutation
methods return `models::ApplicationMutationObject` (`serde_json::Value`) so a
successful write does not fail decoding as a complete first-party resource.
Generated Silicon credentials remain in their one-time creation response.

The existing typed organization/member/tag/governance methods remain for direct
IAM sessions. Deletes, SSO, verification-code delivery, and completed credential
rotation keep their existing typed methods. The separate [scoped backend](../SCOPED_BACKEND.md)
accepts application bearers at `https://scoped.backend.iam.teamofsilicons.com`;
login, token exchange/refresh, and first-party step-up remain at main IAM.

## Runtime state stays with your application

`Client` does not store sessions, cache API responses, or refresh credentials
behind your back. An expired token produces an error; deciding what to do
about it stays with you, because only your program knows where its credentials
live.

If you want the stateful version — a token store, automatic refresh, a
configured default service — that is the `silicon-iam-cli` crate, which is
built on nothing but this one.

## Dependency versions

The client never runs Cargo, changes a lockfile, or checks for updates during
API requests. Choose dependency versions in your project and rebuild after
reviewing updates. Legacy `auto_update` and `update_manifest` settings are no-ops;
`update_status()` always returns `Disabled`. Honeycomb manages installed CLI
releases. See [release management](updates.html).

## Your first Application login

```rust
use silicon_iam_client::{Client, Credential, Mutation};

#[tokio::main]
async fn main() -> silicon_iam_client::Result<()> {
    let app_id = "checkout";
    let application = Client::new("https://backend.iam.teamofsilicons.com")?
        .with_credential(Credential::application(app_id, "ask_your_application_secret"));

    // Receive this only from the IAM-hosted login redirect or token screen.
    let slt = "slt_from_iam";
    let tokens = application
        .oauth()
        .login(app_id, slt, &Mutation::new())
        .await?;
    println!("Application session expires in {} seconds", tokens.expires_in);
    Ok(())
}
```

An Application never starts or verifies an OTP challenge. IAM performs that
identity ceremony on its own hosted login surface; the Rust client accepts
only the resulting single-use SLT for a new Application login.

## Application identity verification

Use an app's own credentials to issue a short-lived identity key, then the
receiving app's credentials to verify it. No user session or SLT is involved:

```rust
use silicon_iam_client::{Client, Credential, models};

let caller = Client::new("https://backend.iam.teamofsilicons.com")?
    .with_credential(Credential::application("checkout", caller_secret));
let issued = caller.app_verification()
    .issue(&models::AppAccessKeyIssue { ttl_seconds: Some(300) }).await?;
// Send issued.app_id and issued.app_access_key to the receiving application.
let receiver = Client::new("https://backend.iam.teamofsilicons.com")?
    .with_credential(Credential::application("vendor>billing", receiver_secret));
let verified = receiver.app_verification().verify(&models::AppAccessKeyVerify {
    app_id: issued.app_id,
    app_access_key: issued.app_access_key,
}).await?;
if verified.valid_key {
    // Identity is verified; authorize the requested action separately.
}
```

An omitted `ttl_seconds` uses 300 seconds; 60–3600 seconds inclusive is accepted.
Each issuance creates a distinct key and returns `app_access_key`, `valid_till`
and `app_id`. The SDK stores nothing; never log or archive the secret. The two
key-bearing models redact their `Debug` output, but serialization includes the key.
Verification does not consume a key. Invalid keys return `valid_key: false` with
no app details; invalid receiver credentials produce an authentication error.
Secret rotation and application disablement revoke keys. For tests, set the
same `EnvironmentKey` on each client and use that environment's app secrets;
keys cannot cross environments or survive cleaning generations. Identity keys
do not grant user permissions or replace OBO.

## What the API groups look like

Every group hangs off the client and borrows it, so obtaining one is free:

| Group | Covers |
| --- | --- |
| `client.system()` | Version negotiation, liveness, readiness |
| `client.signup()` | Creating a Carbon |
| `client.auth()` | IAM refresh/logout, step-up, Silicon authentication, and SLT minting; direct Carbon login is CLI-feature-only |
| `client.carbons()` | The signed-in Carbon; sessions; Carbon lookup |
| `client.organizations()` | Organization tenancy and ownership |
| `client.members()` | Members and the directory view of them |
| `client.invitations()` | Inviting Carbons, and joining |
| `client.tags()` | Organization tags |
| `client.trust()` | Advisory trust: default, rules, evaluation |
| `client.governance()` | Approvals, direct role and tag changes, history |
| `client.silicons()` | Silicons, credentials, webhooks |
| `client.applications()` | Applications, secrets, webhooks |
| `client.oauth()` | Short-lived-token exchange, introspection, revocation |
| `client.app_verification()` | Issue and verify short-lived application identity keys |
| `client.obo()` | Catalog-bound signing and delegated access between applications |
| `client.sso()` | An organization's SSO configuration |
| `client.environments()` | Testing environments |

Platform administration, the inbound provider webhooks and the browser login
screen are deliberately absent: they belong to the operator, to the provider,
and to the browser.

## Idempotency is explicit

Ordinary mutating routes require an idempotency key, so those mutations take a
`Mutation` that carries one. Application identity key issuance is an exception:
it creates a fresh independent key on each call and has no replay storage. The service binds the key to the caller, the route
and the exact body, then replays the original response for a repeat of the same
request — which only helps if a retry presents the *same* key:

```rust
# use silicon_iam_client::{Client, Mutation, models};
# async fn run(client: &Client, input: &models::OrganizationCreate) -> silicon_iam_client::Result<()> {
let creating = Mutation::new();

let organization = match client.organizations().create(input, &creating).await {
    Ok(organization) => organization,
    // The same `creating` replays the first outcome instead of creating a
    // second organization.
    Err(error) if error.is_retryable() => {
        client.organizations().create(input, &creating).await?
    }
    Err(error) => return Err(error),
};
# let _ = organization;
# Ok(())
# }
```

Routes that change an existing resource take its `version` as an ordinary
argument, so optimistic concurrency cannot be forgotten. Routes that change
authority or reveal a credential also need a step-up assertion, attached with
`Mutation::step_up`.

JSON Merge Patch deliberately distinguishes three states. In generated patch
models, a field typed `Option<Option<T>>` uses `None` to omit the field and
leave it unchanged, `Some(None)` to send JSON `null` and clear it, and
`Some(Some(value))` to replace it. Do not collapse the outer and inner options.

## Signing users in to an application

A login produces one short-lived token, and that token is the only thing your
application ever receives — never a password, never a verification code.

Send someone to `<auth_base_url>/login?app_id=…&redirect_uri=…`; they come back
to your callback with `?slt=…`; you trade it for a session:

```rust
# use silicon_iam_client::{Client, Credential, Mutation};
# async fn run(base: &str, app_id: &str, app_secret: &str, slt: String)
#     -> silicon_iam_client::Result<()> {
let application = Client::new(base)?
    .with_credential(Credential::application(app_id, app_secret));

let tokens = application
    .oauth()
    .login(app_id, &slt, &Mutation::new())
    .await?;
# let _ = tokens;
# Ok(())
# }
```

The token lives two minutes and is good for one exchange. `OAuth::login` has no
OTP or refresh-token input: the SLT is the only credential that can begin an
Application session. Renew an existing session separately with
`OAuth::refresh(app_id, refresh_token, mutation)`.

For a direct IAM Carbon/Silicon session, call `auth().login_organizations(app_id)`
and display its `scopes` before selecting organizations. After the user approves, call
`auth().short_lived_token_for_organizations(app_id, &selected_org_ids, choices.scope_version, &approved_scopes, &Mutation::new())`.
Supply the exact reviewed flat scope strings, including `obo:{app_id}:{endpoint_id}` external
permissions. The selected organization set is additive on the same parent login. For a fully
specified request, `short_lived_token(&ShortLivedTokenRequest, mutation)` also supports a callback.
An application change invalidates stale consent; fetch a new view and obtain consent again.
Applications never call these methods with their own credentials; initiate
IAM browser login and receive an SLT instead.

See [organization consent](../ORGANIZATION_CONSENT.md) for scope consent and CLI details.

For OBO, hash the exact downstream bytes with
`api::obo::body_sha256`, build one `OboExchangeRequest`, and pass that request,
the discovered audience catalog, and one `Mutation` to
`obo().exchange_signed(...)`. The client selects the registered path, uses the
same Application secret for Basic authentication and HMAC, and signs a fresh
timestamp. An immediate uncertain retry reuses the request and `Mutation` but
gets a new timestamp/signature; restoring an old timestamp can fall outside the
60-second signature window.

## Refresh, introspection, revocation, and logout

Refresh one Application session with the same Application Basic client. Keep
one refresh in flight per family and persist the `Mutation` key before sending:

```rust
# use silicon_iam_client::{Client, Mutation};
# async fn refresh(application: &Client, app_id: &str, old: &str) -> silicon_iam_client::Result<()> {
let refreshing = Mutation::new();
let key_to_persist = refreshing.key().as_str().to_owned();
let replacement = application.oauth().refresh(app_id, old, &refreshing).await?;
// Atomically replace `old` with `replacement`; retain `key_to_persist`
// until the operation is committed.
# let _ = (replacement, key_to_persist);
# Ok(())
# }
```

Reuse of a consumed refresh token under a new key revokes that Application
refresh family and its related access authority. It does **not** revoke the
parent IAM session, other devices, or unrelated Applications. Recover an
uncertain request with
`Mutation::with_key(IdempotencyKey::parse(saved_key)?)` and the exact same
input. The client does not expose the `Idempotency-Replayed` response
header.

Tokens are opaque. Ask for their current state and optional exact organization
context with `oauth().introspect(&TokenIntrospectionRequest, Some("acme"))`.
A well-formed organization mismatch, unknown token, expiry, or revocation
returns `active: false`; malformed organization context is an API error.
Revoke with `oauth().revoke(&OAuthRevocationRequest, &Mutation)`: an access
token revokes only itself, while a refresh token revokes its family. Unknown
tokens deliberately succeed.

Application-triggered global logout is a different operation. Build a bearer
client from the Carbon Application access token and call
`auth().logout(&LogoutRequest { mode: None }, &Mutation)`. If the token's client
and audience are that Application, IAM revokes the parent IAM session and all
authority bound to it. This form cannot request account-wide `all_sessions`.

## Applications, discovery, and secret rotation

Application creation takes a local handle and an owning organization. IAM
returns the canonical public identifier `{org_id}>{handle}`; use that canonical
value for every later login, credential, path, discovery, and OBO call. The
required `base_url` is the pathless application-backend origin without a
trailing slash, such as `https://billing.example`. It is not a login redirect
and IAM does not call it automatically.

```rust
# use silicon_iam_client::{Client, Mutation, models};
# async fn create(client: &Client) -> silicon_iam_client::Result<()> {
let created = client.applications().create(
    &models::ApplicationCreate {
        app_id: "billing".to_owned(),
        org_id: "acme".to_owned(),
        app_name: Some("Billing".to_owned()),
        app_logo: None,
        webhook_url: "https://billing.example/hooks/iam".to_owned(),
        webhook_secret: "replace-with-at-least-32-random-characters".to_owned(),
        base_url: "https://billing.example".to_owned(),
        obo_endpoints: None,
    },
    &Mutation::new(),
).await?;
// Persist created.app_secret; IAM generated no webhook secret.
# Ok(())
# }
```

An Application authenticating with `Credential::application` can discover any
verified Application's base URL, even across organizations:

```rust
# use silicon_iam_client::{Client, Credential};
# async fn discover(base: &str, caller_secret: &str) -> silicon_iam_client::Result<()> {
let caller = Client::new(base)?.with_credential(Credential::application(
    "checkout",
    caller_secret,
));
let billing = caller
    .applications()
    .discover_base_url("other>billing")
    .await?;
println!("{}", billing.base_url);
# Ok(())
# }
```

Client and webhook signing credentials rotate independently. Both operations
take the current Application `version`, an idempotency key, and a
verified-channel step-up assertion. The Application supplies its own webhook
successor during explicit webhook rotation:

```rust
# use silicon_iam_client::{Client, Mutation, models};
# async fn rotate(client: &Client, step_up: &str) -> silicon_iam_client::Result<()> {
let app = client.applications().get("checkout").await?;
let mutation = Mutation::new().step_up(step_up);
let rotated = client
    .applications()
    .rotate_webhook_secret(
        &app.app_id,
        app.version,
        &models::ApplicationWebhookSecretRotate {
            webhook_secret: "replace-with-32-or-more-random-characters".to_owned(),
        },
        &mutation,
    )
    .await?;
assert_eq!(rotated.webhook_secret_version, app.webhook.secret_version + 1);
# Ok(())
# }
```

The webhook rotation assertion uses action
`application.webhook_secret.rotate`; client-secret rotation uses
`application.client_secret.rotate`. A webhook URL change is a separate
operation and reuses a test-owned or production signing secret unless its
request explicitly supplies a new one. A testing URL replacement installs the supplied
secret or generates a fresh test-only secret and returns it as `webhook_signing_secret`.

### Approving a pending webhook

`applications().approve_webhook(app_id, version, &mutation)`
activates a verified Application's pending first or replacement endpoint.
The authenticated Carbon must currently be the owning organization's owner
or admin, or an IAM platform administrator with `applications.review`.
Creating the Application confers no separate authority. This operation changes
neither Application status nor scopes, and cannot bypass platform review of
an Application whose status is still `under_review`.

Read `applications().webhook(app_id)` to obtain `application_id`, the internal
UUID available to both organization managers and platform webhook reviewers.
Obtain verified-channel step-up for
`models::StepUpAction::ApplicationWebhookApprove` with that UUID as the
resource. Use the assertion and current aggregate version:

```rust
# use silicon_iam_client::{Client, Mutation};
# async fn approve(client: &Client, step_up: &str) -> silicon_iam_client::Result<()> {
let app_id = "checkout";
let current = client.applications().webhook(app_id).await?;
let mutation = Mutation::new().step_up(step_up);
let webhook = client.applications()
    .approve_webhook(app_id, current.version, &mutation)
    .await?;
assert!(webhook.active_url.is_some());
# Ok(())
# }
```

The SDK sends an empty JSON object and returns `models::ApplicationWebhook` with
the new Application aggregate version; no signing secret is returned. Keep
the same mutation key and input after an uncertain outcome. A stale version
fails its precondition; no pending endpoint or a non-verified Application is
a conflict. Test endpoints normally activate immediately, so they do not
need approval.

## Organization listing and SSO

`organizations().list(&paging)` returns organizations where the caller has an
active membership. Use `list_with_status(Some("removed"), &paging)` for the
caller's removed memberships. That query value describes **membership** status;
the `status` on each returned `Organization` still describes the organization
itself (`active` or `disabled`).

SSO starts locked until a platform administrator grants the organization an
entitlement. A Carbon owner or admin with `sso.manage` can inspect it, obtain a
five-minute WorkOS setup link, and test the active connection:

```rust
# use silicon_iam_client::{Client, Mutation};
# async fn sso(client: &Client) -> silicon_iam_client::Result<()> {
let configuration = client.sso().get("acme").await?;
let setup = client.sso().setup_link("acme", &Mutation::new()).await?;
let tested = client.sso().test("acme", &Mutation::new()).await?;
# let _ = (configuration, setup, tested);
# Ok(())
# }
```

Disabling SSO additionally needs the current configuration `version` and a
verified-channel step-up assertion for `organization.sso_change` bound to the
organization's internal UUID:

```rust
# use silicon_iam_client::{Client, Mutation};
# async fn disable(client: &Client, version: i64, step_up: &str) -> silicon_iam_client::Result<()> {
client.sso().disable(
    "acme",
    version,
    &Mutation::new().step_up(step_up),
).await?;
# Ok(())
# }
```

The browser authorization and callback redirects are intentionally not client
methods. SSO never creates a Carbon: the person signs up normally first, then
begins SSO while authenticated in the same bound browser session.

## Testing environments

An environment is the same API against a separate database, starting empty.
The lifecycle is controlled from production. A successful creation returns the
public UUID and the 32-character root key:

```rust
# use silicon_iam_client::{Client, Mutation, models};
# async fn create(production: &Client) -> silicon_iam_client::Result<()> {
let created = production.environments().create(
    "acme",
    &models::TestingEnvironmentCreate {
        name: "checkout-e2e".to_owned(),
        description: Some("CI proof run".to_owned()),
    },
    &Mutation::new(),
).await?;

// Store created.id as the safe selector and created.key in a secret store.
# let _ = created;
# Ok(())
# }
```

Move a client onto that environment and every ordinary method uses the same
route against isolated test data:

```rust
# use silicon_iam_client::{Client, EnvironmentKey};
# async fn run(client: &Client, key: &str) -> silicon_iam_client::Result<()> {
let sandbox = client.with_environment(EnvironmentKey::new(key)?);
let organizations = sandbox.organizations().list(&Default::default()).await?;
# let _ = organizations;
# Ok(())
# }
```

The root key selects the database plane; it does not replace endpoint
authentication. A protected call still needs the bearer or Application Basic
credential issued inside that environment. Email and SMS delivery are
suppressed, and signup, login, invitation and step-up verification accept the
fixed code `000000`.

Credentials do not cross the boundary in either direction: production access
and refresh tokens, short-lived tokens, STKs, Application secrets, sessions,
and OBO proofs are refused in a test environment, and test credentials are
refused in production. IAM does not currently expose a caller API-key
credential; a future API-key surface must retain this same plane binding. Keep
one credential store per environment or key it by the environment UUID.

### Create or import a test Application

Creating a new test Application uses the ordinary method on the plane-selected client:

```rust
# use silicon_iam_client::{Client, Mutation, models};
# async fn create_app(sandbox: &Client) -> silicon_iam_client::Result<()> {
let created = sandbox.applications().create(
    &models::ApplicationCreate {
        app_id: "checkout".to_owned(),
        org_id: "acme".to_owned(),
        app_name: Some("Checkout".to_owned()),
        app_logo: None,
        webhook_url: "https://hooks.example.test/iam".to_owned(),
        webhook_secret: "test-webhook-secret-with-32-characters".to_owned(),
        base_url: "https://checkout.example".to_owned(),
        obo_endpoints: None,
    },
    &Mutation::new(),
).await?;
assert_eq!(created.application.app_id, "checkout");
# Ok(())
# }
```

The local handle is qualified with the owning organization. A newly created
test application cannot claim a canonical ID that already exists in
production.

Import copies a production Application into the selected environment. It can
also create the corresponding test organization and make the authenticated
test Carbon its owner. The response returns a fresh test-only Application
secret, but only confirms that the production webhook secret was inherited;
it never reveals that secret:

```rust
# use silicon_iam_client::{Client, Mutation};
# async fn import(sandbox: &Client) -> silicon_iam_client::Result<()> {
let imported = sandbox
    .applications()
    .import_from_production("google>drive", &Mutation::new())
    .await?;
assert!(imported.webhook_secret_inherited);
// Persist imported.app_secret now. The replay window is ten minutes.
# Ok(())
# }
```

`import_from_production` refuses locally when the client has no environment
key, before any request is sent.

### Discover a base URL in the correct plane

Any authenticated Application may discover any other verified Application,
including one outside its organization. Build the caller with its own Basic
credential. For a test lookup, keep the environment key on the same client so
both caller and target resolve in that environment:

```rust
# use silicon_iam_client::{Client, Credential, EnvironmentKey};
# async fn discover(base: &str, key: &str, secret: &str) -> silicon_iam_client::Result<()> {
let caller = Client::new(base)?
    .with_environment(EnvironmentKey::new(key)?)
    .with_credential(Credential::application("checkout", secret));
let target = caller
    .applications()
    .discover_base_url("google>drive")
    .await?;
println!("{}", target.base_url);
# Ok(())
# }
```

Test webhook delivery is real. Its signed JSON body is wrapped as
`{"test": {"testing_key": "…", "metadata": {…}, "data": {…}}}` rather
than using the production top-level `metadata` and `data`. Treat
`testing_key` as the root credential it is: never log it, persist it with an
event record, or forward it beyond the dedicated test receiver. Verify the
signature over the exact body bytes before reading the envelope.

## Errors

`Error::Api` carries the service's envelope, whose `code` is the stable thing to
match on. `Error::RateLimited` is separate because it is the one failure with a
mechanical remedy — wait the stated interval and repeat. `Error::Transport`
means the request never reached a response, so the outcome is genuinely unknown;
retry it with the original `Mutation` rather than a new one.

```rust
# use silicon_iam_client::Error;
# fn handle(error: Error) {
if let Some(api) = error.api() {
    if api.is_version_conflict() {
        // Someone changed it first: re-read, then decide.
    } else if api.requires_step_up() {
        // Obtain an assertion and attach it to the mutation.
    }
}
# }
```

## Testing against a real service

The authoritative integration proof is the manual CLI walkthrough in the
[CLI guide](https://docs.iam.teamofsilicons.com/cli#end-to-end-application-proof-in-a-test-environment).
Run it against an isolated testing environment and inspect each result. At a
minimum, prove:

- Carbon signup and login use the fixed test-plane code, while an Application
  receives only the resulting SLT; prove explicit scope consent and both single-organization
  and multiple-organization selections;
- token exchange, refresh, current introspection, refresh-family revocation,
  and post-revocation `active: false` all agree;
- an OBO proof verifies exactly once for the registered method, path and exact body,
  including audiences in other organizations; missing declaration, missing critical approval,
  unconsented scopes, wrong subject audience, and unselected organizations are refused;
- valid webhook bytes verify, while a changed byte, stale timestamp, wrong
  secret version, duplicate security header, or wrong environment key fails;
- production credentials fail in the test plane and test credentials fail in
  production;
- SSO entitlement, setup-link, connection test and step-up-protected disable
  behave as documented when SSO is in scope.

Keep the environment UUID as metadata and its root key in secret storage. Do
not substitute mock-only routes: the test plane intentionally uses the same API
paths and authorization rules as production.

For crate-development coverage, the repository also contains ignored live
integration tests. Point them at a disposable running instance:

```sh
SILICON_IAM_LIVE_URL=http://127.0.0.1:8080 \
  cargo test -p silicon-iam-client --test live -- --ignored --test-threads=1
```

## Regenerating the wire types

After changing `docs/openapi.yaml`:

```sh
ruby scripts/generate-client-models.rb
```

The output is committed as ordinary source, so a contract change shows up as a
reviewable diff.

## License

Licensed under the Apache License, Version 2.0. See `LICENSE`.

Copyright 2026 Team of Silicons.

## Scope declarations and review discussions

`ApplicationCreate.app_scope` and `ApplicationPatch.app_scope` use
`ApplicationScope { iam, external }`. Omitted create scopes default to
`self.identity.read` and `self.profile.read`. `webhook_scope` controls event categories
(`full`, `membership`, `updates`, `trust`) independently. Read `effective_app_scope` for the
currently approved configuration and `app_scope` for the requested configuration.

Use `application_scopes().catalog(None)` for the IAM catalog or pass an audience app ID for
its exposed endpoint scopes. Every OBO endpoint declares `critical: true` or `false`.
Use `applications().update` to remove permissions or change noncritical configuration;
submit `application_scopes().request(app_id, version, &ApplicationScopeRequestCreate, mutation)`
when requesting critical approval. Supply a detailed initial message. IAM creates one review
per target authority. `requests(status)` lists discussions; `get(request_id)` returns the
messages and whether you can decide. `reply` and `decide` both require the current request
version and a mutation key. A denial requires a nonempty `reason`.

The initial critical review blocks first use. An upgrade keeps the previous effective scopes
working until the requested additions are approved. Removing scopes does not need approval.
No token receives an unapproved scope, and existing user consent never silently expands.

## Application bundles

`bundles().availability(org_id)` reads whether bundle configuration is available to the
signed-in Carbon in that organization. It returns only `available`; it does not expose
organization policy settings. Use a direct Carbon IAM credential and an active membership.
A successful `false` response means bundle configuration is unavailable for this caller.

`bundles().list/create/get/update/delete` manage named groups of applications in one
organization. Use `list_page(&paging)` for all administered organizations or
`list_for_organization(org_id, &paging)` to apply the organization filter before pagination.
Both return `items` and `page`, including the continuation cursor. Application lists provide
`applications().list_for_organization(org_id, status, &paging)` for the same purpose.
Unavailable organizations return 404. Each member remains independently registered, with its
own secret.

`ApplicationBundleCreate.app_logo` accepts an optional HTTPS image URL.
For `ApplicationBundlePatch.app_logo`, `None` preserves the current logo,
`Some(Some(url))` replaces it, and `Some(None)` removes it. Read the saved logo from
`ApplicationBundle.app_logo`; omitting the URL never uploads or invents an image.
`auth().bundle_login_organizations` returns the bundle and member consent views.
`auth().bundle_short_lived_tokens` accepts the complete list of approved member selections and
returns one independent SLT per app atomically. Each target exchanges its own SLT normally.

## Application testing layer

Authenticate with the production application's `Credential::application`, then call
`applications().create_testing_environment(&ApplicationTestingEnvironmentCreate, mutation)`.
Provide `name`, optional `description`, and optionally `iam_test_key`. An existing valid key
reuses that exact IAM test environment; an invalid supplied key fails; omitting it creates a
new environment. IAM imports the app and all transitive declared external dependencies into
that environment, including cyclic/shared dependency graphs without duplicating apps.
Only the calling application's test secret is returned; dependency secrets remain inside IAM.
`applications().testing_environments(Some("all"), &paging)` lists active and deleted links, including `can_manage`, status, version, and purge deadline. The creating production application can use `environments().get/update/key/rotate_key/clean/delete/restore` for that environment with its production app credential. Imported dependencies do not inherit control-plane authority.

Use the returned `iam_test_key` with `Client::with_environment` and the returned test
`app_secret` for subsequent application API calls. The same production routes operate on
isolated data, and verification uses `000000`. Production and test credentials never mix.
For an application receiving a request, the presence of **`app_secret` in the request itself
requests test mode**: call `applications().testing_context()` with the test credential and environment key to validate it through IAM and route to the matching IAM environment's
isolated application data. Do not select a test database from an unvalidated user flag.
All transitive app calls remain in that IAM environment. Inactivity retention defaults to
30 days and is configurable per app through `testing_idle_days`.

Test webhooks put event fields under `test.data` and `test.metadata` and include the IAM
`testing_key`; production events keep `data` and `metadata` at the top level. Verify the exact
signed outer body, validate the embedded key using `verify_testing_environment`, and avoid
logging it. An imported production webhook signing secret is inherited but never revealed;
replace the test destination to install an independent test secret.

## Scoped IAM reads and API contracts

An application's user access token carries only approved scopes. Organization lists include
only explicitly selected active memberships. Directory lists require the corresponding
`directory.carbons.read` or `directory.silicons.read`; field scopes independently control
profiles, roles, job descriptions, tags, hierarchy, capabilities, and accessible Silicons.
Self permissions never reveal those fields for other members. Email and phone are self-only.
Absent fields mean undisclosed and must not be replaced with cached wider permissions.

Rust integrations use `client.application_reads().me/organizations/organization/members/member/member_authorization/silicon/tags`
with `Credential::bearer(application_access_token)`. These methods preserve scope-dependent
JSON field omission. Direct IAM management methods retain their full typed response shapes.
Use `system().contracts()` or `iam system contracts` (alias `iam api contracts`) to inspect
contract versions and compatibility. Breaking changes receive a new major API version;
a deprecated version can sunset after seven days without requests.

The client supports sanitized Space Station request diagnostics when configured. Use `.telemetry(false)` on the builder to opt out. See [telemetry](../TELEMETRY.md) for keys, buffering and propagated preferences.

## Honeycomb service integration

The server-only `honeycomb::ManagementClient` uses a dedicated service credential,
with a separate live actor token and `Mutation` step-up assertion for authorized
changes. See [the integration contract](../HONEYCOMB_INTEGRATION.md). Ordinary
application clients retain login, discovery, consent and runtime OBO.
