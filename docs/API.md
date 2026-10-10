# Browser HTTP API

The production base URL is `https://backend.browser.teamofsilicons.com`.
Successful JSON responses use `{"data": ...}`; failures use
`{"error":{"code":"...","message":"..."}}`, with optional field details and
request correlation. The Rust [client](../crates/client/README.md) exposes the
same metadata API without reading credentials or state from disk.

## Accounts authentication

`GET /api/v1/accounts` is public and returns `{"data":{"app_id":"browser"}}`.
Silicons request a Browser SLT through `silicon-accounts login --app browser`
and exchange it once:

```http
POST /api/v1/auth/exchange
Content-Type: application/json

{"short_lived_token":"slt_...","contract_version":2}
```

Refresh uses the current refresh token and the response-contract marker:

```http
POST /api/v1/auth/refresh
Content-Type: application/json

{"refresh_token":"sar_...","contract_version":2}
```

Both return `access_token`, `refresh_token`, `expires_at`, `identity` and
`services` inside `data`. The identity's `id` is its permanent canonical
Accounts UUIDv4; `name` is its current display handle and `kind` is `carbon` or
`silicon`. No organization is selected or returned by the current contract.
Use `Authorization: Bearer ACCESS_JWT` for protected routes. Raw HTTP clients
send `contract_version: 2` for the org-free response. The Rust SDK injects this
marker automatically; its public exchange and refresh request structs contain
only their respective token fields.

Access tokens expire after 30 minutes. Refresh tokens rotate once and their
family expires at most 900 days after initial login; rotation does not extend
that deadline. Serialize refresh and atomically save the new pair before use.
Do not resubmit an SLT, authorization code or refresh token after an uncertain
response. Browser claims submitted credentials before contacting Accounts,
and repeated submission requires fresh sign-in. Accounts refresh-token reuse
can revoke the entire family.

The website starts Carbon sign-in with
`POST /api/v1/auth/attempts` and `{"identity_kind":"carbon"}`. The response
contains `attempt_id`, `state` and `authorize_url`. Browser owns the PKCE
verifier. After the registered `/auth/callback` redirect, submit `state`,
`code` and `contract_version: 2` once to
`POST /api/v1/auth/attempts/{attempt_id}/exchange`.

Browser checks Accounts issuer, audience/client ID, account kind, permanent
UUIDv4 and app membership. Short legacy IDs are retired. Public `c:` and `si:`
handles never act as bearer subjects or implicit ownership aliases. New clients
send no organization header or request field. Requests that omit the contract
marker receive the 1.1 response's fixed Browser partition for compatibility
with already installed clients.

`GET /api/v1/me` returns the current identity;
`GET /api/v1/services` returns currently enabled services. The CLI and website
persist their own credentials; the Rust client leaves storage and refresh
coordination to its caller. `browser logout` clears the saved local CLI login.

## Resource routes

All routes below require the current account's bearer token. Resource access
uses ownership and explicit grants; a Silicon's custodian receives no implicit
access. List filters use the CLI's AND-separated pipeline grammar in a
URL-encoded `filter` query parameter.

| Resource | Routes |
| --- | --- |
| Proxy locations | `GET /api/v1/proxy-locations` |
| Profiles | `GET/POST /api/v1/profiles`; `GET/PATCH /api/v1/profiles/{id}`; `POST /api/v1/profiles/{id}/end` |
| Sessions | `GET/POST /api/v1/sessions`; `GET /api/v1/sessions/{id}`; `POST /api/v1/sessions/{id}/end` |
| Live links | `POST /api/v1/sessions/{id}/live`; `POST /api/v1/sessions/{id}/live/redeem` |
| Local control | `GET /api/v1/sessions/{id}/connection`; `POST /api/v1/sessions/{id}/commands` |
| Command history | `GET /api/v1/sessions/{id}/logs?date=YYYY-MM-DD` |
| Recordings | `GET /api/v1/recordings`; `GET /api/v1/recordings/{session_id}`; `POST /api/v1/recordings/{session_id}/trash` |
| Usage | `GET /api/v1/usage/limits`; `GET /api/v1/usage`; `GET /api/v1/usage/{session_id}` |
| Discovery | `POST /api/v1/search`; `POST /api/v1/fetch` |

Profile creation accepts `name`, fixed `location` and an optional `access`
array. Selectors use `@UUID`, `@c:handle` or `@si:handle`; handles are resolved
when access changes. Owners remain in the access list. Session creation uses
a profile or incognito, name, description and TTL. See the
[shared request types](../crates/shared/src/model.rs) for complete fields and
validation, and [CLI examples](CLI.md) for common operations.

A session connection is a secret direct provider capability. The controller
executes locally and sends no action output through the API. Command reports
contain a stable command UUID, command/flags, timestamps and result metadata;
they never execute actions. Identical report retries acknowledge the original
receipt. Do not retry a browser action because metadata reporting failed.

Accounts webhook registration and exact-origin CORS are documented in
[deployment](DEPLOYMENT.md). Current upstream identity and token rules are
documented in [Accounts IDs](https://developers.teamofsilicons.com/docs/accounts/learn/ids-and-uuids)
and [tokens and sessions](https://developers.teamofsilicons.com/docs/accounts/learn/tokens-and-sessions).
