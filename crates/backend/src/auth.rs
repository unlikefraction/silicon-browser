//! Silicon Accounts authentication. Tokens are introspected with the app's server credentials.
//! Browser uses immutable Accounts UUIDs as owners; mutable c:/si: IDs are display values.
use crate::url_policy::is_https_or_loopback_http;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use silicon_browser_shared::{IdentityKind, actor_id};
use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, RwLock},
    time::Duration,
};
use thiserror::Error;
use url::Url;

pub(crate) const WORKSPACE: &str = "browser";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrincipalIdentity {
    pub principal_id: String,
    pub public_id: Option<String>,
    pub tags: Option<Vec<String>>,
    pub scopes: Vec<String>,
    pub kind: IdentityKind,
    pub membership_id: String,
    pub authorization_epoch: i64,
    pub expires_at: DateTime<Utc>,
}
#[derive(Clone, PartialEq, Eq)]
pub struct ExchangedAuth {
    pub access_token: String,
    pub refresh_token: String,
    pub identity: PrincipalIdentity,
    pub scope: String,
}
#[derive(Clone, PartialEq, Eq)]
pub struct ExchangeRequest {
    pub short_lived_token: String,
    pub idempotency_key: String,
}
#[derive(Clone, PartialEq, Eq)]
pub struct RefreshRequest {
    pub refresh_token: String,
    pub idempotency_key: String,
}
#[derive(Clone)]
pub struct CodeExchangeRequest {
    pub code: String,
    pub redirect_uri: String,
    pub code_verifier: String,
}
macro_rules! redacted_debug { ($($ty:ty),*) => {$ (impl fmt::Debug for $ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(concat!(stringify!($ty), " { credentials: [REDACTED] }")) }
})*}; }
redacted_debug!(ExchangedAuth, ExchangeRequest, RefreshRequest, CodeExchangeRequest);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityCapability {
    RevokeApplicationToken,
    HostedSignIn,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpstreamFailure {
    RateLimited,
    Unavailable,
    Transport,
    InvalidResponse,
}
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum IdentityError {
    #[error("authentication is required or no longer active")]
    Unauthenticated,
    #[error("the authenticated account is not authorized for this workspace")]
    Forbidden,
    #[error("identity capability is unavailable: {0:?}")]
    CapabilityUnavailable(IdentityCapability),
    #[error("invalid authentication input in {field}: {reason}")]
    InvalidInput { field: &'static str, reason: &'static str },
    #[error("Accounts rejected the request ({code}, HTTP {status})")]
    Rejected { status: u16, code: String, request_id: Option<String> },
    #[error("Accounts {kind:?} failure")]
    Upstream { kind: UpstreamFailure, request_id: Option<String>, retry_after: Option<Duration> },
    #[error("Accounts contract violation during {operation}: {reason}")]
    Contract { operation: &'static str, reason: &'static str },
}
#[async_trait]
pub trait IdentityProvider: Send + Sync {
    fn app_id(&self) -> &str {
        "browser"
    }
    fn authorization_url(&self, redirect: &str, state: &str, challenge: &str) -> Result<String, IdentityError> {
        authorization_url("https://accounts.teamofsilicons.com", self.app_id(), redirect, state, challenge)
    }
    async fn exchange_authorization_code(&self, _request: CodeExchangeRequest) -> Result<ExchangedAuth, IdentityError> {
        Err(IdentityError::CapabilityUnavailable(IdentityCapability::HostedSignIn))
    }
    async fn resolve_account_id(&self, selector: &str) -> Result<String, IdentityError> {
        Ok(selector.to_owned())
    }
    async fn identify(&self, bearer: &str) -> Result<PrincipalIdentity, IdentityError>;
    async fn exchange_short_lived_token(&self, request: ExchangeRequest) -> Result<ExchangedAuth, IdentityError>;
    async fn refresh(&self, request: RefreshRequest) -> Result<ExchangedAuth, IdentityError>;
    async fn revoke_application_token(&self, _token: &str, _idempotency_key: &str) -> Result<(), IdentityError> {
        Err(IdentityError::CapabilityUnavailable(IdentityCapability::RevokeApplicationToken))
    }
}
#[derive(Clone)]
pub struct AccountsIdentityProvider {
    http: reqwest::Client,
    base: Url,
    app_id: Arc<str>,
    secret: Arc<str>,
    cache: Option<Arc<crate::auth_cache::AuthorizationCache>>,
}
impl fmt::Debug for AccountsIdentityProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccountsIdentityProvider")
            .field("base", &self.base)
            .field("app_id", &self.app_id)
            .finish_non_exhaustive()
    }
}
#[derive(Deserialize)]
struct Introspection {
    active: bool,
    iss: Option<String>,
    sub: Option<String>,
    aud: Option<String>,
    client_id: Option<String>,
    exp: Option<i64>,
    nbf: Option<i64>,
    kind: Option<IdentityKind>,
    id: Option<String>,
    membership_id: Option<String>,
    scope: Option<String>,
    token_type: Option<String>,
}
#[derive(Deserialize)]
struct Account {
    uuid: String,
    membership_id: String,
    kind: IdentityKind,
    id: String,
}
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    token_type: String,
    expires_in: i64,
    membership_id: String,
    account: Account,
    #[serde(default)]
    scope: String,
}
impl AccountsIdentityProvider {
    pub async fn connect(base: &str, app_id: String, secret: String) -> Result<Self, IdentityError> {
        let base = Url::parse(base).map_err(|_| input("accounts_url", "expected an HTTP(S) URL"))?;
        if !is_https_or_loopback_http(&base)
            || base.username() != ""
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || base.path() != "/"
        {
            return Err(input("accounts_url", "expected an HTTPS origin, or loopback HTTP"));
        }
        silicon_browser_shared::app_id(&app_id, "accounts_app_id")
            .map_err(|_| input("accounts_app_id", "expected a bare app ID"))?;
        if !opaque(&secret, "sa_app_") {
            return Err(input("accounts_app_secret", "expected the Accounts sa_app_ application secret"));
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("silicon-browser/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| upstream(UpstreamFailure::Transport))?;
        Ok(Self { http, base, app_id: app_id.into(), secret: secret.into(), cache: None })
    }
    pub fn with_authorization_cache(mut self, cache: Arc<crate::auth_cache::AuthorizationCache>) -> Self {
        self.cache = Some(cache);
        self
    }
    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        fields: &[(&str, &str)],
    ) -> Result<T, IdentityError> {
        let body = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(fields.iter().copied()).finish();
        let request = self
            .http
            .post(self.base.join(path).map_err(|_| input("accounts_url", "invalid Accounts endpoint"))?)
            .basic_auth(&*self.app_id, Some(&*self.secret))
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body);
        self.request(request).await
    }
    async fn request<T: serde::de::DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, IdentityError> {
        let mut response = request.send().await.map_err(|_| upstream(UpstreamFailure::Transport))?;
        let status = response.status();
        let request_id = response.headers().get("x-request-id").and_then(|v| v.to_str().ok()).map(str::to_owned);
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .map(Duration::from_secs);
        if response.content_length().is_some_and(|n| n > MAX_RESPONSE_BYTES as u64) {
            return Err(upstream(UpstreamFailure::InvalidResponse));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| upstream(UpstreamFailure::Transport))? {
            if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(upstream(UpstreamFailure::InvalidResponse));
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            if status.as_u16() == 429 || status.is_server_error() {
                return Err(IdentityError::Upstream {
                    kind: if status.as_u16() == 429 {
                        UpstreamFailure::RateLimited
                    } else {
                        UpstreamFailure::Unavailable
                    },
                    request_id,
                    retry_after,
                });
            }
            let code = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_owned))
                .unwrap_or_else(|| "request_rejected".into());
            if code == "invalid_grant" {
                return Err(IdentityError::Unauthenticated);
            }
            return Err(IdentityError::Rejected { status: status.as_u16(), code, request_id });
        }
        serde_json::from_slice(&bytes).map_err(|_| upstream(UpstreamFailure::InvalidResponse))
    }
    async fn introspect(&self, token: &str) -> Result<PrincipalIdentity, IdentityError> {
        if !jwt(token) {
            return Err(IdentityError::Unauthenticated);
        }
        let claims: Introspection = self.post("v1/oauth/introspect", &[("token", token)]).await?;
        identity_from_claims(claims, self.base.as_str().trim_end_matches('/'), &self.app_id)
    }
    async fn exchange(&self, fields: &[(&str, &str)]) -> Result<ExchangedAuth, IdentityError> {
        // Accounts burns codes and rotates refresh tokens. Never replay this request automatically.
        let token: TokenResponse = self.post("v1/oauth/token", fields).await?;
        if token.token_type != "Bearer"
            || !(1..=1800).contains(&token.expires_in)
            || !jwt(&token.access_token)
            || !opaque(&token.refresh_token, "sar_")
        {
            return Err(contract("token exchange", "invalid token response"));
        }
        let identity = self.introspect(&token.access_token).await?;
        if identity.principal_id != token.account.uuid
            || identity.kind != token.account.kind
            || identity.public_id.as_deref() != Some(token.account.id.as_str())
            || identity.membership_id != token.membership_id
            || token.account.membership_id != token.membership_id
        {
            return Err(contract("token exchange", "account response disagreed with authenticated introspection"));
        }
        Ok(ExchangedAuth {
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            identity,
            scope: token.scope,
        })
    }
}
#[async_trait]
impl IdentityProvider for AccountsIdentityProvider {
    fn app_id(&self) -> &str {
        &self.app_id
    }
    fn authorization_url(&self, redirect: &str, state: &str, challenge: &str) -> Result<String, IdentityError> {
        authorization_url(self.base.as_str(), &self.app_id, redirect, state, challenge)
    }
    async fn exchange_authorization_code(&self, request: CodeExchangeRequest) -> Result<ExchangedAuth, IdentityError> {
        if !opaque(&request.code, "sac_") {
            return Err(input("code", "expected an Accounts authorization code"));
        }
        self.exchange(&[
            ("grant_type", "authorization_code"),
            ("code", &request.code),
            ("redirect_uri", &request.redirect_uri),
            ("code_verifier", &request.code_verifier),
        ])
        .await
    }
    async fn resolve_account_id(&self, selector: &str) -> Result<String, IdentityError> {
        if silicon_browser_shared::account_id(selector, "access").is_ok() {
            return Ok(selector.to_owned());
        }
        actor_id(selector, "access").map_err(|_| input("access", "expected @Accounts UUID or @c:/si: handle"))?;
        let mut url =
            self.base.join("v1/accounts/by-id/").map_err(|_| input("accounts_url", "invalid Accounts endpoint"))?;
        url.path_segments_mut()
            .map_err(|_| input("accounts_url", "invalid Accounts endpoint"))?
            .pop_if_empty()
            .push(selector);
        #[derive(Deserialize)]
        struct Lookup {
            uuid: String,
            id: String,
            kind: IdentityKind,
            status: String,
        }
        let account: Lookup = self
            .request(
                self.http.get(url).basic_auth(&*self.app_id, Some(&*self.secret)).header(ACCEPT, "application/json"),
            )
            .await?;
        if account.id != selector
            || actor_id(&account.id, "access").ok() != Some(account.kind)
            || silicon_browser_shared::account_id(&account.uuid, "access").is_err()
            || account.status != "active"
        {
            return Err(input("access", "sharing requires an active Accounts account"));
        }
        Ok(account.uuid)
    }
    async fn identify(&self, bearer: &str) -> Result<PrincipalIdentity, IdentityError> {
        if let Some(cache) = &self.cache {
            cache.resolve(bearer, || self.introspect(bearer)).await
        } else {
            self.introspect(bearer).await
        }
    }
    async fn exchange_short_lived_token(&self, request: ExchangeRequest) -> Result<ExchangedAuth, IdentityError> {
        if !opaque(&request.short_lived_token, "slt_") {
            return Err(input("short_lived_token", "expected an Accounts slt_ token"));
        }
        self.exchange(&[("grant_type", "urn:silicon:params:oauth:grant-type:slt"), ("slt", &request.short_lived_token)])
            .await
    }
    async fn refresh(&self, request: RefreshRequest) -> Result<ExchangedAuth, IdentityError> {
        if !opaque(&request.refresh_token, "sar_") {
            return Err(input("refresh_token", "expected an Accounts sar_ token"));
        }
        let result = self.exchange(&[("grant_type", "refresh_token"), ("refresh_token", &request.refresh_token)]).await;
        if let Some(cache) = &self.cache {
            cache.invalidate();
        }
        result
    }
    async fn revoke_application_token(&self, token: &str, _: &str) -> Result<(), IdentityError> {
        let _: serde_json::Value = self.post("v1/oauth/revoke", &[("token", token)]).await?;
        if let Some(cache) = &self.cache {
            cache.invalidate();
        }
        Ok(())
    }
}
fn authorization_url(
    base: &str,
    app: &str,
    redirect: &str,
    state: &str,
    challenge: &str,
) -> Result<String, IdentityError> {
    let mut url = Url::parse(base)
        .and_then(|u| u.join("authorize"))
        .map_err(|_| input("accounts_url", "invalid Accounts origin"))?;
    url.query_pairs_mut().extend_pairs([
        ("app_id", app),
        ("redirect_uri", redirect),
        ("state", state),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("response_type", "code"),
        ("scope", "profile"),
    ]);
    Ok(url.to_string())
}
fn input(field: &'static str, reason: &'static str) -> IdentityError {
    IdentityError::InvalidInput { field, reason }
}
fn contract(operation: &'static str, reason: &'static str) -> IdentityError {
    IdentityError::Contract { operation, reason }
}
fn upstream(kind: UpstreamFailure) -> IdentityError {
    IdentityError::Upstream { kind, request_id: None, retry_after: None }
}
fn opaque(value: &str, prefix: &str) -> bool {
    value
        .strip_prefix(prefix)
        .is_some_and(|s| !s.is_empty() && value.len() <= 16384 && value.bytes().all(|b| b.is_ascii_graphic()))
}
fn jwt(value: &str) -> bool {
    value.len() <= 16384
        && value.split('.').count() == 3
        && value
            .split('.')
            .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')))
}
fn identity_from_claims(c: Introspection, issuer: &str, app: &str) -> Result<PrincipalIdentity, IdentityError> {
    if !c.active {
        return Err(IdentityError::Unauthenticated);
    }
    let (Some(id), Some(kind), Some(public), Some(membership), Some(exp)) =
        (c.sub, c.kind, c.id, c.membership_id, c.exp)
    else {
        return Err(contract("introspection", "missing account claims"));
    };
    if c.iss.as_deref() != Some(issuer)
        || c.aud.as_deref() != Some(app)
        || c.client_id.as_deref() != Some(app)
        || c.token_type.as_deref() != Some("access_token")
        || silicon_browser_shared::account_id(&id, "sub").is_err()
        || actor_id(&public, "id").ok() != Some(kind)
        || membership != format!("{app}:{id}")
    {
        return Err(contract("introspection", "invalid issuer, audience or stable account binding"));
    }
    let expires_at = DateTime::from_timestamp(exp, 0).ok_or_else(|| contract("introspection", "invalid expiry"))?;
    if expires_at <= Utc::now() || c.nbf.is_some_and(|nbf| nbf > Utc::now().timestamp()) {
        return Err(IdentityError::Unauthenticated);
    }
    Ok(PrincipalIdentity {
        principal_id: id,
        public_id: Some(public),
        tags: None,
        scopes: c.scope.unwrap_or_default().split_whitespace().map(str::to_owned).collect(),
        kind,
        membership_id: membership,
        authorization_epoch: 0,
        expires_at,
    })
}

/// Explicit fake for local domain tests. Credential lookup keys are hashed.
#[derive(Clone, Default)]
pub struct FakeIdentityProvider {
    state: Arc<RwLock<FakeState>>,
}
#[derive(Default)]
struct FakeState {
    identities: HashMap<[u8; 32], PrincipalIdentity>,
    exchanges: HashMap<[u8; 32], Result<ExchangedAuth, IdentityError>>,
    refreshes: HashMap<[u8; 32], Result<ExchangedAuth, IdentityError>>,
}
impl FakeIdentityProvider {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn allow_identity(&self, bearer: &str, identity: PrincipalIdentity) {
        self.state.write().unwrap().identities.insert(token_digest(bearer), identity);
    }
    pub fn deny_identity(&self, bearer: &str) {
        self.state.write().unwrap().identities.remove(&token_digest(bearer));
    }
    pub fn allow_exchange(&self, token: &str, result: ExchangedAuth) {
        self.state.write().unwrap().exchanges.insert(token_digest(token), Ok(result));
    }
    pub fn fail_exchange(&self, token: &str, error: IdentityError) {
        self.state.write().unwrap().exchanges.insert(token_digest(token), Err(error));
    }
    pub fn allow_refresh(&self, token: &str, result: ExchangedAuth) {
        self.state.write().unwrap().refreshes.insert(token_digest(token), Ok(result));
    }
    pub fn fail_refresh(&self, token: &str, error: IdentityError) {
        self.state.write().unwrap().refreshes.insert(token_digest(token), Err(error));
    }
}
#[async_trait]
impl IdentityProvider for FakeIdentityProvider {
    async fn identify(&self, bearer: &str) -> Result<PrincipalIdentity, IdentityError> {
        self.state.read().unwrap().identities.get(&token_digest(bearer)).cloned().ok_or(IdentityError::Unauthenticated)
    }
    async fn exchange_short_lived_token(&self, r: ExchangeRequest) -> Result<ExchangedAuth, IdentityError> {
        self.state
            .read()
            .unwrap()
            .exchanges
            .get(&token_digest(&r.short_lived_token))
            .cloned()
            .unwrap_or(Err(IdentityError::Unauthenticated))
    }
    async fn exchange_authorization_code(&self, r: CodeExchangeRequest) -> Result<ExchangedAuth, IdentityError> {
        self.state
            .read()
            .unwrap()
            .exchanges
            .get(&token_digest(&r.code))
            .cloned()
            .unwrap_or(Err(IdentityError::Unauthenticated))
    }
    async fn refresh(&self, r: RefreshRequest) -> Result<ExchangedAuth, IdentityError> {
        self.state
            .read()
            .unwrap()
            .refreshes
            .get(&token_digest(&r.refresh_token))
            .cloned()
            .unwrap_or(Err(IdentityError::Unauthenticated))
    }
}
fn token_digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::test_http::{spawn_json_server, spawn_json_server_with};
    use serde_json::json;
    fn claims(issuer: &str) -> serde_json::Value {
        json!({"active":true,"iss":issuer,"sub":"0080c488-a9e9-4c22-aa91-1c7a639f1d7b","aud":"browser","client_id":"browser","exp":Utc::now().timestamp()+900,"kind":"carbon","id":"c:ada","membership_id":"browser:0080c488-a9e9-4c22-aa91-1c7a639f1d7b","token_type":"access_token","scope":"profile"})
    }
    #[test]
    fn immutable_uuid_is_authority_while_handle_can_change() {
        let original = identity_from_claims(
            serde_json::from_value(claims("https://accounts.example")).unwrap(),
            "https://accounts.example",
            "browser",
        )
        .unwrap();
        let mut renamed = claims("https://accounts.example");
        renamed["id"] = json!("c:ada-new");
        let renamed =
            identity_from_claims(serde_json::from_value(renamed).unwrap(), "https://accounts.example", "browser")
                .unwrap();
        assert_eq!(original.principal_id, renamed.principal_id);
        assert_ne!(original.public_id, renamed.public_id);
    }
    #[test]
    fn rejects_wrong_audience_revocation_and_refresh_credentials() {
        for field in ["aud", "client_id", "iss", "membership_id", "token_type", "sub"] {
            let mut value = claims("https://accounts.example");
            value[field] = json!("wrong");
            assert!(
                identity_from_claims(serde_json::from_value(value).unwrap(), "https://accounts.example", "browser")
                    .is_err()
            );
        }
        let inactive = serde_json::from_value(json!({"active":false})).unwrap();
        assert_eq!(
            identity_from_claims(inactive, "https://accounts.example", "browser"),
            Err(IdentityError::Unauthenticated)
        );
    }
    #[tokio::test]
    async fn exchanges_and_refreshes_use_basic_authenticated_accounts_forms() {
        for (grant, credential) in [
            ("urn:silicon:params:oauth:grant-type:slt", "slt_once"),
            ("refresh_token", "sar_once"),
            ("authorization_code", "sac_once"),
        ] {
            let token = json!({"access_token":"header.payload.signature","refresh_token":"sar_new","token_type":"Bearer","expires_in":900,"membership_id":"browser:0080c488-a9e9-4c22-aa91-1c7a639f1d7b","account":{"uuid":"0080c488-a9e9-4c22-aa91-1c7a639f1d7b","membership_id":"browser:0080c488-a9e9-4c22-aa91-1c7a639f1d7b","kind":"carbon","id":"c:ada"},"scope":"profile"});
            let (base, mut requests, task) =
                spawn_json_server_with(|origin| vec![(200, token.to_string()), (200, claims(origin).to_string())])
                    .await;
            let provider =
                AccountsIdentityProvider::connect(&base, "browser".into(), "sa_app_test".into()).await.unwrap();
            let result = match grant {
                "refresh_token" => {
                    provider
                        .refresh(RefreshRequest { refresh_token: credential.into(), idempotency_key: "ignored".into() })
                        .await
                }
                "authorization_code" => {
                    provider
                        .exchange_authorization_code(CodeExchangeRequest {
                            code: credential.into(),
                            redirect_uri: "https://browser.example/auth/callback".into(),
                            code_verifier: "V".repeat(43),
                        })
                        .await
                }
                _ => {
                    provider
                        .exchange_short_lived_token(ExchangeRequest {
                            short_lived_token: credential.into(),

                            idempotency_key: "ignored".into(),
                        })
                        .await
                }
            };
            assert_eq!(result.unwrap().identity.principal_id, "0080c488-a9e9-4c22-aa91-1c7a639f1d7b");
            let request = requests.recv().await.unwrap();
            assert_eq!(request.target, "/v1/oauth/token");
            assert!(request.headers.to_ascii_lowercase().contains("authorization: basic "));
            let form: HashMap<_, _> = url::form_urlencoded::parse(&request.body).into_owned().collect();
            assert_eq!(form["grant_type"], grant);
            if grant == "authorization_code" {
                assert_eq!(form["code_verifier"], "V".repeat(43));
                assert_eq!(form["redirect_uri"], "https://browser.example/auth/callback");
            }

            assert_eq!(requests.recv().await.unwrap().target, "/v1/oauth/introspect");
            task.await.unwrap();
        }
    }
    #[tokio::test]
    async fn current_handle_lookup_resolves_stable_acl_authority() {
        let (base, mut requests, task) = spawn_json_server(vec![(
            200,
            json!({"uuid":"0080c488-a9e9-4c22-aa91-1c7a639f1d7b","id":"c:ada","kind":"carbon","status":"active"})
                .to_string(),
        )])
        .await;
        let provider = AccountsIdentityProvider::connect(&base, "browser".into(), "sa_app_test".into()).await.unwrap();
        assert_eq!(provider.resolve_account_id("c:ada").await.unwrap(), "0080c488-a9e9-4c22-aa91-1c7a639f1d7b");
        assert!(provider.resolve_account_id("K1E").await.is_err());
        assert_eq!(
            provider.resolve_account_id("bc0f6684-df51-4d9f-b945-bbdf88e599ad").await.unwrap(),
            "bc0f6684-df51-4d9f-b945-bbdf88e599ad"
        );
        let request = requests.recv().await.unwrap();
        assert_eq!(request.method, "GET");
        assert_eq!(request.target, "/v1/accounts/by-id/c:ada");
        assert!(request.headers.to_ascii_lowercase().contains("authorization: basic "));
        task.await.unwrap();
    }
    #[tokio::test]
    async fn revoked_tokens_are_checked_live_and_redirects_never_receive_secrets() {
        let (base, _requests, task) = spawn_json_server(vec![(200, json!({"active":false}).to_string())]).await;
        let provider = AccountsIdentityProvider::connect(&base, "browser".into(), "sa_app_test".into()).await.unwrap();
        assert_eq!(provider.identify("header.payload.signature").await, Err(IdentityError::Unauthenticated));
        task.await.unwrap();
        let (base, _requests, task) = spawn_json_server(vec![(302, "{}".into())]).await;
        let provider = AccountsIdentityProvider::connect(&base, "browser".into(), "sa_app_test".into()).await.unwrap();
        assert!(provider.identify("header.payload.signature").await.is_err());
        task.await.unwrap();
    }
}
