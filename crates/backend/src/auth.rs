//! Silicon IAM authentication boundary.
//!
//! Application access tokens are opaque.  We deliberately derive no identity
//! from their text. Production metadata requests may reuse a short-lived, token-bound
//! introspection snapshot, invalidated by verified IAM webhooks. Session browser traffic
//! connects directly from clients to the provider.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use silicon_browser_shared::{Identity, IdentityKind, Org};
use silicon_iam_client::models::{
    ActorRefType, ApiVersionNegotiation, ApplicationAuthorizationActorType, DirectoryMember, OAuthTokenResponse,
    OrganizationPage, TokenIntrospection, TokenIntrospectionActorType,
};
use silicon_iam_client::{Credential, EnvironmentKey, IdempotencyKey, Mutation};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

use crate::providers::OnBehalfOfGrant;
use crate::url_policy::is_https_or_loopback_http;

const IAM_API_VERSION: &str = "v1";
const IAM_SUPPORTED_VERSIONS_HEADER: &str = "silicon-iam-supported-api-versions";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_ORG_PAGES: usize = 100;
const RECORDING_ENDPOINT_ID: &str = "briefcase.files.create";
const RECORDING_ENDPOINT_PATH: &str = "/api/v1/obo/files";

/// Identity facts an application token can prove through IAM introspection.
///
/// IAM supplies public identity and optional membership tags in the current
/// authorization snapshot. Undisclosed tags remain absent and grant no access.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrincipalIdentity {
    pub principal_id: Uuid,
    pub public_id: Option<String>,
    pub tags: Option<Vec<String>>,
    pub kind: IdentityKind,
    pub org_id: String,
    pub membership_id: Uuid,
    pub authorization_epoch: i64,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrganizationAccess {
    pub id: String,
    /// Only direct-IAM directory/list APIs provide the display name. OAT
    /// introspection therefore returns `None` here rather than inventing one.
    pub name: Option<String>,
}

/// One SLT exchange. Credential fields are always redacted from `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct ExchangedAuth {
    pub access_token: String,
    pub refresh_token: String,
    pub identity: PrincipalIdentity,
    pub scope: String,
}

impl fmt::Debug for ExchangedAuth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExchangedAuth")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("identity", &self.identity)
            .field("scope", &self.scope)
            .finish()
    }
}

/// Delivery-only mutation recovery. A replay can return an expired OAT alongside
/// a still-valid, unconsumed ORT. Such a pair must be persisted then refreshed;
/// it never authorizes an API request or recording proof by itself.
#[derive(Clone, Debug)]
pub struct DeliveryTokenExchange {
    pub auth: ExchangedAuth,
    pub access_active: bool,
}

/// IAM owns organization consent. This optional workspace preference may only
/// select an organization already authorized by the SLT.
#[derive(Clone, PartialEq, Eq)]
pub struct ExchangeRequest {
    pub short_lived_token: String,
    pub required_org_id: Option<String>,
    /// Reuse this value when retrying the same logical exchange.
    pub idempotency_key: String,
}

impl fmt::Debug for ExchangeRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExchangeRequest")
            .field("short_lived_token", &"[REDACTED]")
            .field("required_org_id", &self.required_org_id)
            .field("idempotency_key", &self.idempotency_key)
            .finish()
    }
}

/// Rotates an application refresh token without allowing the caller's selected
/// organization to change under it.
#[derive(Clone, PartialEq, Eq)]
pub struct RefreshRequest {
    pub refresh_token: String,
    pub required_org_id: String,
    /// Reuse this value when retrying the same logical rotation.
    pub idempotency_key: String,
}

impl fmt::Debug for RefreshRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RefreshRequest")
            .field("refresh_token", &"[REDACTED]")
            .field("required_org_id", &self.required_org_id)
            .field("idempotency_key", &self.idempotency_key)
            .finish()
    }
}

/// One exact recording upload. The audience must come from backend configuration,
/// and the actor must be the session initiator, not a shared-profile viewer.
#[derive(Clone)]
pub struct RecordingProofRequest {
    pub expected_org_id: String,
    pub expected_actor_id: String,
    pub audience: String,
    pub path: String,
    pub name: String,
    pub content_type: String,
    pub body_sha256: String,
    /// Keep the same key and request for an uncertain IAM exchange retry.
    /// A downstream upload is a separate, single-use operation.
    pub idempotency_key: String,
}

impl fmt::Debug for RecordingProofRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecordingProofRequest")
            .field("expected_org_id", &self.expected_org_id)
            .field("expected_actor_id", &self.expected_actor_id)
            .field("audience", &self.audience)
            .field("metadata", &"[REDACTED]")
            .field("body_sha256", &self.body_sha256)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingProof {
    pub grant: OnBehalfOfGrant,
    pub proof_id: Uuid,
    /// Bound by both IAM's proof expiry and its parent access token's expiry.
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityCapability {
    /// Reserved for an identity provider that cannot enumerate an unscoped
    /// token's authorized organizations.
    OrganizationsForUnboundApplicationToken,
    RecordingProof,
    RevokeApplicationToken,
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

    #[error("the authenticated principal is not authorized for this organization")]
    Forbidden,

    #[error("identity capability is unavailable: {0:?}")]
    CapabilityUnavailable(IdentityCapability),

    #[error("invalid authentication input in {field}: {reason}")]
    InvalidInput { field: &'static str, reason: &'static str },

    #[error("IAM rejected the request ({code}, HTTP {status})")]
    Rejected { status: u16, code: String, request_id: Option<String> },

    #[error("IAM {kind:?} failure")]
    Upstream { kind: UpstreamFailure, request_id: Option<String>, retry_after: Option<Duration> },

    #[error("IAM contract violation during {operation}: {reason}")]
    Contract { operation: &'static str, reason: &'static str },
}

#[async_trait]
pub trait IdentityProvider: Send + Sync {
    /// Public application identifier used to obtain an SLT from IAM.
    fn app_id(&self) -> &str {
        "tos>browser"
    }

    /// Resolve an OAT inside an explicit organization context.
    async fn identify(&self, bearer: &str, org: &str) -> Result<PrincipalIdentity, IdentityError>;

    /// Return only the organizations currently authorized by an OAT.
    async fn orgs(&self, bearer: &str) -> Result<Vec<OrganizationAccess>, IdentityError>;

    /// Exchange the single-use short-lived token IAM gave the caller.
    async fn exchange_short_lived_token(&self, request: ExchangeRequest) -> Result<ExchangedAuth, IdentityError>;

    /// Atomically rotate an IAM application refresh token.
    async fn refresh(&self, request: RefreshRequest) -> Result<ExchangedAuth, IdentityError>;

    async fn exchange_delivery_token(&self, request: ExchangeRequest) -> Result<DeliveryTokenExchange, IdentityError> {
        self.exchange_short_lived_token(request).await.map(|auth| DeliveryTokenExchange { auth, access_active: true })
    }

    async fn refresh_delivery_token(&self, request: RefreshRequest) -> Result<DeliveryTokenExchange, IdentityError> {
        self.refresh(request).await.map(|auth| DeliveryTokenExchange { auth, access_active: true })
    }

    async fn revoke_application_token(&self, _token: &str, _idempotency_key: &str) -> Result<(), IdentityError> {
        Err(IdentityError::CapabilityUnavailable(IdentityCapability::RevokeApplicationToken))
    }

    /// Mint a short-lived, exact-body proof immediately before uploading bytes.
    /// This does not make the parent credential durable or refresh it.
    async fn issue_recording_proof(
        &self,
        _bearer: &str,
        _request: RecordingProofRequest,
    ) -> Result<RecordingProof, IdentityError> {
        Err(IdentityError::CapabilityUnavailable(IdentityCapability::RecordingProof))
    }
}

/// Silicon IAM adapter.
///
/// Every request travels through redirect-refusing clients, so a bearer or
/// application credential can never be replayed to a redirect target.
#[derive(Clone)]
pub struct SiliconIamIdentityProvider {
    http: reqwest::Client,
    sdk: silicon_iam_client::Client,
    base_url: Url,
    app_id: Arc<str>,
    app_secret: Secret,
    testing_environment: bool,
    authorization_cache: Option<Arc<crate::auth_cache::AuthorizationCache>>,
}

impl fmt::Debug for SiliconIamIdentityProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SiliconIamIdentityProvider")
            .field("base_url", &self.base_url)
            .field("app_id", &self.app_id)
            .field("app_secret", &"[REDACTED]")
            .field("testing_environment", &self.testing_environment)
            .finish()
    }
}

#[derive(Clone)]
struct Secret(Arc<str>);

impl Secret {
    fn new(value: String) -> Self {
        Self(Arc::from(value))
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret([REDACTED])")
    }
}

impl SiliconIamIdentityProvider {
    /// Build a redirect-refusing IAM client and negotiate the service contract
    /// before accepting traffic.
    pub async fn connect(base_url: &str, app_id: String, app_secret: String) -> Result<Self, IdentityError> {
        Self::connect_with_environment(base_url, app_id, app_secret, None).await
    }

    /// Select an isolated IAM test environment for every request, including
    /// negotiation. The key is validated by IAM's SDK and redacted in headers.
    pub async fn connect_with_environment(
        base_url: &str,
        app_id: String,
        app_secret: String,
        environment_key: Option<&str>,
    ) -> Result<Self, IdentityError> {
        let mut headers = HeaderMap::new();
        if let Some(key) = environment_key {
            EnvironmentKey::new(key).map_err(|_| IdentityError::InvalidInput {
                field: "iam_test_environment_key",
                reason: "expected exactly 32 alphanumeric characters",
            })?;
            let mut value = HeaderValue::from_str(key).map_err(|_| IdentityError::InvalidInput {
                field: "iam_test_environment_key",
                reason: "expected exactly 32 alphanumeric characters",
            })?;
            value.set_sensitive(true);
            headers.insert("x-testing-environment-key", value);
        }
        validate_app_credential(&app_id, &app_secret)?;
        let parsed = Url::parse(base_url)
            .map_err(|_| IdentityError::InvalidInput { field: "iam_url", reason: "expected an HTTP(S) URL" })?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(IdentityError::InvalidInput {
                field: "iam_url",
                reason: "expected a credential-free HTTP(S) URL without query or fragment",
            });
        }
        if !is_https_or_loopback_http(&parsed) {
            return Err(IdentityError::InvalidInput {
                field: "iam_url",
                reason: "expected HTTPS unless the IAM host is loopback",
            });
        }

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("silicon-browser/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| IdentityError::Upstream {
                kind: UpstreamFailure::Transport,
                request_id: None,
                retry_after: None,
            })?;
        let negotiation_url = unversioned_url(&parsed, &["api", "version"])?;
        let negotiated: ApiVersionNegotiation = decode_response(
            "version negotiation",
            http.get(negotiation_url)
                .header(IAM_SUPPORTED_VERSIONS_HEADER, IAM_API_VERSION)
                .header(ACCEPT, "application/json")
                .send()
                .await,
        )
        .await?;
        validate_negotiation(&negotiated)?;
        let mut provider = Self::from_parts(http, parsed, app_id, app_secret)?;
        if let Some(key) = environment_key {
            provider.sdk = provider.sdk.with_environment(EnvironmentKey::new(key).map_err(obo_sdk_error)?);
        }
        provider.testing_environment = environment_key.is_some();
        Ok(provider)
    }

    fn from_parts(
        http: reqwest::Client,
        base_url: Url,
        app_id: String,
        app_secret: String,
    ) -> Result<Self, IdentityError> {
        let sdk = silicon_iam_client::Client::builder(base_url.as_str())
            .map_err(obo_sdk_error)?
            .credential(Credential::application(app_id.clone(), app_secret.clone()))
            .auto_update(false)
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(obo_sdk_error)?;
        Ok(Self {
            http,
            sdk,
            base_url,
            app_id: Arc::from(app_id),
            app_secret: Secret::new(app_secret),
            testing_environment: false,
            authorization_cache: None,
        })
    }

    pub fn with_authorization_cache(mut self, cache: Arc<crate::auth_cache::AuthorizationCache>) -> Self {
        self.authorization_cache = Some(cache);
        self
    }

    async fn identify_fresh(&self, bearer: &str, org: &str) -> Result<PrincipalIdentity, IdentityError> {
        let claims = self.introspect(bearer, Some(org)).await?;
        identity_from_claims(&claims, &self.app_id, org, None, Utc::now())
    }

    /// Explicit direct-IAM path. It accepts only `cat_`/`sat_` access tokens;
    /// an OAT is never silently retried against a route with different auth
    /// semantics.
    pub async fn direct_iam_orgs(&self, bearer: &str) -> Result<Vec<Org>, IdentityError> {
        validate_direct_iam_token(bearer)?;
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        let mut organizations = Vec::new();

        for _ in 0..MAX_ORG_PAGES {
            let mut url = versioned_url(&self.base_url, &["organizations"])?;
            {
                let mut query = url.query_pairs_mut();
                query.append_pair("limit", "100");
                if let Some(cursor) = &cursor {
                    query.append_pair("after", cursor);
                }
            }
            let page: OrganizationPage = decode_response(
                "organization listing",
                self.http
                    .get(url)
                    .bearer_auth(bearer)
                    .header(IAM_SUPPORTED_VERSIONS_HEADER, IAM_API_VERSION)
                    .header(ACCEPT, "application/json")
                    .send()
                    .await,
            )
            .await?;
            organizations.extend(page.items.into_iter().map(|org| Org { id: org.org_id, name: org.name }));
            if !page.page.has_more {
                return Ok(organizations);
            }
            let next = page.page.next_cursor.ok_or(IdentityError::Contract {
                operation: "organization listing",
                reason: "has_more was true without a continuation cursor",
            })?;
            if !seen.insert(next.clone()) {
                return Err(IdentityError::Contract {
                    operation: "organization listing",
                    reason: "the continuation cursor repeated",
                });
            }
            cursor = Some(next);
        }
        Err(IdentityError::Contract {
            operation: "organization listing",
            reason: "the listing exceeded the pagination safety bound",
        })
    }

    /// Explicit direct-IAM directory lookup, including a display name that
    /// the application authorization snapshot does not disclose.
    pub async fn direct_iam_identity(&self, bearer: &str, org_id: &str) -> Result<(Identity, Org), IdentityError> {
        let kind = validate_direct_iam_token(bearer)?;
        validate_org_id(org_id)?;
        let mut url = versioned_url(&self.base_url, &["organizations", org_id, "directory", "self"])?;
        url.query_pairs_mut().append_pair("fields", "name,id,org,tags");
        let member: DirectoryMember = decode_response(
            "directory self",
            self.http
                .get(url)
                .bearer_auth(bearer)
                .header(IAM_SUPPORTED_VERSIONS_HEADER, IAM_API_VERSION)
                .header(ACCEPT, "application/json")
                .send()
                .await,
        )
        .await?;
        let public_id = member.id.ok_or(IdentityError::Contract {
            operation: "directory self",
            reason: "the requested id field was absent",
        })?;
        let name = member.name.ok_or(IdentityError::Contract {
            operation: "directory self",
            reason: "the requested name field was absent",
        })?;
        let organization = member.org.ok_or(IdentityError::Contract {
            operation: "directory self",
            reason: "the requested org field was absent",
        })?;
        if organization.id != org_id {
            return Err(IdentityError::Contract {
                operation: "directory self",
                reason: "IAM returned a different organization",
            });
        }
        let tags = member
            .tags
            .ok_or(IdentityError::Contract {
                operation: "directory self",
                reason: "the requested tags field was absent",
            })?
            .into_iter()
            .map(|tag| tag.name)
            .collect();
        Ok((
            Identity { id: public_id, name, kind, tags, verified_aliases: Vec::new() },
            Org { id: organization.id, name: organization.name },
        ))
    }

    async fn introspect(&self, bearer: &str, org: Option<&str>) -> Result<TokenIntrospection, IdentityError> {
        validate_oat(bearer)?;
        self.introspect_credential(bearer, org, "access_token").await
    }

    async fn introspect_credential(
        &self,
        bearer: &str,
        org: Option<&str>,
        hint: &str,
    ) -> Result<TokenIntrospection, IdentityError> {
        let body = form(&[("token", bearer), ("token_type_hint", hint)]);
        let mut request = self
            .http
            .post(versioned_url(&self.base_url, &["oauth", "introspect"])?)
            .basic_auth(self.app_id.as_ref(), Some(self.app_secret.expose()))
            .header(IAM_SUPPORTED_VERSIONS_HEADER, IAM_API_VERSION)
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body);
        if let Some(org) = org {
            request = request.header("x-org-id", org);
        }
        let claims: TokenIntrospection = decode_response("token introspection", request.send().await).await?;
        if let Some(authorization) = &claims.authorization
            && authorization.testing_environment_id.is_some() != self.testing_environment
        {
            return Err(IdentityError::Contract {
                operation: "token introspection",
                reason: "authorization snapshot used a different testing plane",
            });
        }
        Ok(claims)
    }

    async fn exchange_slt(&self, request: &ExchangeRequest) -> Result<OAuthTokenResponse, IdentityError> {
        validate_exchange_request(request)?;
        let body = form(&[("app_id", self.app_id.as_ref()), ("slt", &request.short_lived_token)]);
        let response = self
            .http
            .post(versioned_url(&self.base_url, &["app-auth", "tokens"])?)
            .basic_auth(self.app_id.as_ref(), Some(self.app_secret.expose()))
            .header(IAM_SUPPORTED_VERSIONS_HEADER, IAM_API_VERSION)
            .header("idempotency-key", &request.idempotency_key)
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await;
        decode_response("short-lived-token exchange", response).await
    }

    async fn exchange_refresh(&self, request: &RefreshRequest) -> Result<OAuthTokenResponse, IdentityError> {
        validate_refresh_request(request)?;
        let body = form(&[("app_id", self.app_id.as_ref()), ("refresh_token", &request.refresh_token)]);
        let response = self
            .http
            .post(versioned_url(&self.base_url, &["app-auth", "tokens"])?)
            .basic_auth(self.app_id.as_ref(), Some(self.app_secret.expose()))
            .header(IAM_SUPPORTED_VERSIONS_HEADER, IAM_API_VERSION)
            .header("idempotency-key", &request.idempotency_key)
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await;
        decode_response("refresh-token exchange", response).await
    }

    async fn validate_delivery_exchange(
        &self,
        response: OAuthTokenResponse,
        org: &str,
        operation: &'static str,
    ) -> Result<DeliveryTokenExchange, IdentityError> {
        match self.validate_exchanged(response.clone(), Some(org), operation).await {
            Ok(auth) => return Ok(DeliveryTokenExchange { auth, access_active: true }),
            Err(IdentityError::Unauthenticated) => {}
            Err(error) => return Err(error),
        }
        // Only an authenticated introspection of the returned family credential
        // can recover a late idempotency replay. Exchange metadata alone is not authority.
        let claims = self.introspect_credential(&response.refresh_token, Some(org), "refresh_token").await?;
        validate_common_claims(&claims, &self.app_id, Utc::now())?;
        if claims.org_id.as_deref() != Some(org) {
            return Err(IdentityError::Forbidden);
        }
        let kind = match claims.actor_type {
            Some(TokenIntrospectionActorType::Carbon) => IdentityKind::Carbon,
            Some(TokenIntrospectionActorType::Silicon) => IdentityKind::Silicon,
            _ => {
                return Err(IdentityError::Contract {
                    operation,
                    reason: "refresh actor type was not carbon or silicon",
                });
            }
        };
        let principal_id = claims.principal_id.filter(|id| !id.is_nil()).ok_or(IdentityError::Forbidden)?;
        if let Some(actor) = response.actor.as_ref() {
            let snapshot_public_id =
                claims.authorization.as_ref().and_then(|authorization| authorization.public_id.as_deref());
            if actor.principal_id.is_nil()
                || principal_id != actor.principal_id
                || kind != actor_ref_kind(&actor.type_field, operation)?
                || actor.public_id.trim().is_empty()
                || snapshot_public_id.is_some_and(|public_id| public_id != actor.public_id)
            {
                return Err(IdentityError::Forbidden);
            }
        }
        let membership_id = claims.membership_id.filter(|id| !id.is_nil()).ok_or(IdentityError::Forbidden)?;
        let authorization_epoch =
            claims.authorization_epoch.filter(|epoch| *epoch > 0).ok_or(IdentityError::Forbidden)?;
        let expires_at = DateTime::from_timestamp(claims.expires_at.ok_or(IdentityError::Unauthenticated)?, 0)
            .ok_or(IdentityError::Unauthenticated)?;
        Ok(DeliveryTokenExchange {
            access_active: false,
            auth: ExchangedAuth {
                access_token: response.access_token,
                refresh_token: response.refresh_token,
                scope: claims.scope.unwrap_or_default(),
                identity: PrincipalIdentity {
                    principal_id,
                    public_id: response.actor.as_ref().map(|actor| actor.public_id.clone()),
                    tags: None,
                    kind,
                    org_id: org.into(),
                    membership_id,
                    authorization_epoch,
                    expires_at,
                },
            },
        })
    }

    async fn validate_exchanged(
        &self,
        response: OAuthTokenResponse,
        required_org_id: Option<&str>,
        operation: &'static str,
    ) -> Result<ExchangedAuth, IdentityError> {
        if response.org_id.as_deref().is_some_and(|value| required_org_id.is_some_and(|expected| value != expected)) {
            return Err(IdentityError::Forbidden);
        }
        if response.token_type.as_str() != Some("Bearer") {
            return Err(IdentityError::Contract { operation, reason: "token_type was not Bearer" });
        }
        if response.expires_in <= 0 {
            return Err(IdentityError::Contract { operation, reason: "expires_in was not positive" });
        }
        validate_oat(&response.access_token)
            .map_err(|_| IdentityError::Contract { operation, reason: "access_token did not have IAM's OAT form" })?;
        validate_opaque_token(&response.refresh_token, &["ort_"])
            .map_err(|_| IdentityError::Contract { operation, reason: "refresh_token did not have IAM's ORT form" })?;
        let actor_public_id = response
            .actor
            .as_ref()
            .map(|actor| {
                if actor.public_id.trim().is_empty() {
                    Err(IdentityError::Contract { operation, reason: "exchange actor had no public_id" })
                } else {
                    Ok(actor.public_id.clone())
                }
            })
            .transpose()?;

        // Do not trust exchange metadata alone. Verify that IAM considers the
        // new OAT active in an organization already authorized by the SLT.
        let claims = self.introspect(&response.access_token, required_org_id).await?;
        let org_id = required_org_id
            .map(str::to_owned)
            .or_else(|| response.org_id.clone())
            .or_else(|| {
                organizations_from_claims(&claims, &self.app_id, Utc::now()).ok()?.into_iter().map(|org| org.id).min()
            })
            .ok_or(IdentityError::Contract { operation, reason: "IAM returned no organization authorization" })?;
        let selected_claims = if claims.org_id.as_deref() == Some(org_id.as_str()) {
            claims
        } else {
            self.introspect(&response.access_token, Some(&org_id)).await?
        };
        let identity = identity_from_claims(&selected_claims, &self.app_id, &org_id, actor_public_id, Utc::now())?;
        if let Some(actor) = response.actor.as_ref()
            && (identity.principal_id != actor.principal_id
                || identity.kind != actor_ref_kind(&actor.type_field, operation)?)
        {
            return Err(IdentityError::Contract { operation, reason: "exchange actor did not match introspection" });
        }

        Ok(ExchangedAuth {
            access_token: response.access_token,
            refresh_token: response.refresh_token,
            identity,
            scope: response.scope,
        })
    }
}

#[async_trait]
impl IdentityProvider for SiliconIamIdentityProvider {
    fn app_id(&self) -> &str {
        &self.app_id
    }

    async fn identify(&self, bearer: &str, org: &str) -> Result<PrincipalIdentity, IdentityError> {
        validate_org_id(org)?;
        if let Some(cache) = &self.authorization_cache {
            cache.resolve(bearer, org, || self.identify_fresh(bearer, org)).await
        } else {
            self.identify_fresh(bearer, org).await
        }
    }

    async fn orgs(&self, bearer: &str) -> Result<Vec<OrganizationAccess>, IdentityError> {
        let claims = self.introspect(bearer, None).await?;
        organizations_from_claims(&claims, &self.app_id, Utc::now())
    }

    async fn exchange_short_lived_token(&self, request: ExchangeRequest) -> Result<ExchangedAuth, IdentityError> {
        let response = self.exchange_slt(&request).await?;
        self.validate_exchanged(response, request.required_org_id.as_deref(), "short-lived-token exchange").await
    }

    async fn refresh(&self, request: RefreshRequest) -> Result<ExchangedAuth, IdentityError> {
        let response = self.exchange_refresh(&request).await?;
        self.validate_exchanged(response, Some(&request.required_org_id), "refresh-token exchange").await
    }

    async fn exchange_delivery_token(&self, request: ExchangeRequest) -> Result<DeliveryTokenExchange, IdentityError> {
        let response = self.exchange_slt(&request).await?;
        self.validate_delivery_exchange(
            response,
            request.required_org_id.as_deref().ok_or(IdentityError::InvalidInput {
                field: "org_id",
                reason: "delivery exchange requires an organization",
            })?,
            "delivery SLT exchange",
        )
        .await
    }

    async fn refresh_delivery_token(&self, request: RefreshRequest) -> Result<DeliveryTokenExchange, IdentityError> {
        let response = self.exchange_refresh(&request).await?;
        self.validate_delivery_exchange(response, &request.required_org_id, "delivery refresh").await
    }

    async fn revoke_application_token(&self, token: &str, idempotency_key: &str) -> Result<(), IdentityError> {
        validate_opaque_token(token, &["ort_"])
            .map_err(|_| IdentityError::InvalidInput { field: "refresh_token", reason: "invalid IAM refresh token" })?;
        let mutation = Mutation::with_key(IdempotencyKey::parse(idempotency_key).map_err(obo_sdk_error)?);
        self.sdk
            .oauth()
            .revoke(
                &silicon_iam_client::models::OAuthRevocationRequest { token: token.to_owned(), token_type_hint: None },
                &mutation,
            )
            .await
            .map_err(obo_sdk_error)
    }

    async fn issue_recording_proof(
        &self,
        bearer: &str,
        request: RecordingProofRequest,
    ) -> Result<RecordingProof, IdentityError> {
        validate_recording_proof_request(&request)?;
        let claims = self.introspect(bearer, Some(&request.expected_org_id)).await?;
        let identity = identity_from_claims(&claims, &self.app_id, &request.expected_org_id, None, Utc::now())?;
        if identity.public_id.as_deref() != Some(&request.expected_actor_id) {
            return Err(IdentityError::Forbidden);
        }
        let scopes: HashSet<_> = claims.scope.as_deref().unwrap_or_default().split_whitespace().collect();
        if !["obo.issue", "memberships.read", "roles.read"].iter().all(|scope| scopes.contains(scope)) {
            return Err(IdentityError::Forbidden);
        }
        let catalog = self.sdk.obo().endpoints(&request.audience).await.map_err(obo_sdk_error)?;
        let mut endpoints = catalog.endpoints.iter().filter(|endpoint| endpoint.endpoint_id == RECORDING_ENDPOINT_ID);
        let endpoint = endpoints.next().ok_or(IdentityError::Contract {
            operation: "recording proof",
            reason: "Briefcase file creation is absent from the IAM endpoint catalog",
        })?;
        if catalog.application.app_id != request.audience
            || catalog.application.org_id != request.expected_org_id
            || endpoint.path != RECORDING_ENDPOINT_PATH
            || endpoints.next().is_some()
        {
            return Err(IdentityError::Contract {
                operation: "recording proof",
                reason: "Briefcase endpoint catalog did not match the configured destination",
            });
        }
        let mutation = Mutation::with_key(IdempotencyKey::parse(request.idempotency_key).map_err(obo_sdk_error)?);
        let proof = self.sdk.obo().exchange_signed(&silicon_iam_client::models::OboExchangeRequest {
            org_id: Some(request.expected_org_id.clone()), subject_token: bearer.to_owned(), audience: request.audience, endpoint_id: RECORDING_ENDPOINT_ID.into(),
            metadata: serde_json::json!({"path":request.path, "name":request.name, "content_type":request.content_type}),
            request: silicon_iam_client::models::OboExchangeRequestBinding {
                method: "POST".into(), body_sha256: request.body_sha256,
            },
        }, &catalog, &mutation).await.map_err(obo_sdk_error)?;
        let expires_at =
            DateTime::from_timestamp(proof.expires_at.unix_timestamp(), 0).ok_or(IdentityError::Contract {
                operation: "recording proof",
                reason: "proof expiry was outside the supported timestamp range",
            })?;
        let now = Utc::now();
        if proof.proof_id.is_nil()
            || !(1..=60).contains(&proof.expires_in)
            || expires_at <= now
            || expires_at > now + chrono::TimeDelta::seconds(65)
            || validate_opaque_token(&proof.access_proof, &["obo_"]).is_err()
        {
            return Err(IdentityError::Contract {
                operation: "recording proof",
                reason: "IAM returned an invalid recording proof",
            });
        }
        let grant = OnBehalfOfGrant::new(proof.access_proof).map_err(|_| IdentityError::Contract {
            operation: "recording proof",
            reason: "IAM returned an invalid recording proof",
        })?;
        Ok(RecordingProof { grant, proof_id: proof.proof_id, expires_at: expires_at.min(identity.expires_at) })
    }
}

fn organizations_from_claims(
    claims: &TokenIntrospection,
    app_id: &str,
    now: DateTime<Utc>,
) -> Result<Vec<OrganizationAccess>, IdentityError> {
    if let Some(id) = claims.org_id.clone() {
        identity_from_claims(claims, app_id, &id, None, now)?;
        return Ok(vec![OrganizationAccess { id, name: None }]);
    }
    validate_common_claims(claims, app_id, now)?;
    let authorizations = claims.authorizations.as_ref().ok_or(IdentityError::Contract {
        operation: "token introspection",
        reason: "an unscoped token had no organization authorizations",
    })?;
    let mut result = Vec::with_capacity(authorizations.len());
    for authorization in authorizations {
        let id = authorization.org_id.clone();
        let mut selected = claims.clone();
        selected.org_id = Some(id.clone());
        selected.membership_id = Some(authorization.membership_id);
        selected.authorization_epoch = Some(authorization.authorization_epoch);
        selected.authorization = Some(authorization.clone());
        selected.authorizations = None;
        identity_from_claims(&selected, app_id, &id, None, now)?;
        result.push(OrganizationAccess { id, name: None });
    }
    result.sort_by(|left, right| left.id.cmp(&right.id));
    result.dedup_by(|left, right| left.id == right.id);
    Ok(result)
}

fn validate_common_claims(claims: &TokenIntrospection, app_id: &str, now: DateTime<Utc>) -> Result<(), IdentityError> {
    if !claims.active {
        return Err(IdentityError::Unauthenticated);
    }
    if claims.client_id.as_deref() != Some(app_id) || claims.audience.as_deref() != Some(app_id) {
        return Err(IdentityError::Unauthenticated);
    }
    let expires_at = claims.expires_at.ok_or(IdentityError::Contract {
        operation: "token introspection",
        reason: "an active token had no expires_at",
    })?;
    if expires_at <= now.timestamp() {
        return Err(IdentityError::Unauthenticated);
    }
    Ok(())
}

fn identity_from_claims(
    claims: &TokenIntrospection,
    app_id: &str,
    expected_org: &str,
    public_id: Option<String>,
    now: DateTime<Utc>,
) -> Result<PrincipalIdentity, IdentityError> {
    validate_common_claims(claims, app_id, now)?;
    if claims.org_id.as_deref() != Some(expected_org) {
        return Err(IdentityError::Forbidden);
    }
    let kind = match claims.actor_type.as_ref() {
        Some(TokenIntrospectionActorType::Carbon) => IdentityKind::Carbon,
        Some(TokenIntrospectionActorType::Silicon) => IdentityKind::Silicon,
        _ => {
            return Err(IdentityError::Contract {
                operation: "token introspection",
                reason: "actor_type was not carbon or silicon",
            });
        }
    };
    let expires_at =
        DateTime::from_timestamp(claims.expires_at.expect("validated above"), 0).ok_or(IdentityError::Contract {
            operation: "token introspection",
            reason: "expires_at was outside the supported timestamp range",
        })?;
    let mut identity = PrincipalIdentity {
        principal_id: claims.principal_id.ok_or(IdentityError::Contract {
            operation: "token introspection",
            reason: "an active token had no principal_id",
        })?,
        public_id,
        tags: None,
        kind,
        org_id: expected_org.to_owned(),
        membership_id: claims.membership_id.ok_or(IdentityError::Contract {
            operation: "token introspection",
            reason: "an org-bound token had no membership_id",
        })?,
        authorization_epoch: claims.authorization_epoch.ok_or(IdentityError::Contract {
            operation: "token introspection",
            reason: "an active token had no authorization_epoch",
        })?,
        expires_at,
    };
    let invalid = || IdentityError::Contract {
        operation: "token introspection",
        reason: "authorization snapshot was absent or did not match the live token",
    };
    let authorization = claims.authorization.as_ref().ok_or_else(invalid)?;
    let kind_matches = matches!(
        (&authorization.actor_type, identity.kind),
        (Some(ApplicationAuthorizationActorType::Carbon), IdentityKind::Carbon)
            | (Some(ApplicationAuthorizationActorType::Silicon), IdentityKind::Silicon)
    );
    let authorization_public_id = authorization.public_id.as_deref().filter(|id| !id.trim().is_empty());
    let scopes: HashSet<&str> = claims.scope.as_deref().ok_or_else(invalid)?.split_whitespace().collect();
    let authorization_scopes: HashSet<&str> = authorization.scopes.iter().map(String::as_str).collect();
    if authorization.principal_id != identity.principal_id
        || !kind_matches
        || authorization.org_id != expected_org
        || authorization.membership_id != identity.membership_id
        || authorization.authorization_epoch != identity.authorization_epoch
        || authorization.audience != app_id
        || authorization_public_id.is_none()
        || identity.public_id.as_deref().is_some_and(|id| Some(id) != authorization_public_id)
        || scopes != authorization_scopes
        || authorization.scopes.len() != authorization_scopes.len()
        || (authorization.tags.is_some() && !scopes.contains("memberships.read"))
        || (authorization.org_role.is_some() && !scopes.contains("self.membership.read"))
        || authorization.tags.as_ref().is_some_and(|tags| tags.iter().any(|tag| tag.name.trim().is_empty()))
    {
        return Err(invalid());
    }
    identity.public_id = Some(authorization_public_id.expect("validated above").to_owned());
    identity.tags = authorization.tags.as_ref().map(|tags| tags.iter().map(|tag| tag.name.clone()).collect());
    Ok(identity)
}

fn actor_ref_kind(kind: &ActorRefType, operation: &'static str) -> Result<IdentityKind, IdentityError> {
    match kind {
        ActorRefType::Carbon => Ok(IdentityKind::Carbon),
        ActorRefType::Silicon => Ok(IdentityKind::Silicon),
        _ => Err(IdentityError::Contract { operation, reason: "actor type was not carbon or silicon" }),
    }
}

fn validate_negotiation(negotiated: &silicon_iam_client::models::ApiVersionNegotiation) -> Result<(), IdentityError> {
    if negotiated.service.as_str() != Some("silicon-iam") {
        return Err(IdentityError::Contract {
            operation: "version negotiation",
            reason: "the endpoint did not identify as silicon-iam",
        });
    }
    if negotiated.selected_api_version != IAM_API_VERSION
        || !negotiated.supported_api_versions.iter().any(|version| version == IAM_API_VERSION)
    {
        return Err(IdentityError::Contract {
            operation: "version negotiation",
            reason: "IAM did not select its canonical v1 contract",
        });
    }
    if negotiated.supported_api_versions.len() > 16
        || negotiated.supported_api_versions.iter().any(|version| api_version_number(version).is_none())
        || negotiated
            .supported_api_versions
            .windows(2)
            .any(|pair| api_version_number(&pair[0]) <= api_version_number(&pair[1]))
    {
        return Err(IdentityError::Contract {
            operation: "version negotiation",
            reason: "supported_api_versions was malformed",
        });
    }
    Ok(())
}

fn api_version_number(value: &str) -> Option<u32> {
    let digits = value.strip_prefix('v')?;
    if digits.is_empty()
        || digits.len() > 9
        || digits.starts_with('0')
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    digits.parse().ok()
}

fn validate_app_credential(app_id: &str, secret: &str) -> Result<(), IdentityError> {
    if app_id.trim().is_empty() || app_id.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(IdentityError::InvalidInput {
            field: "app_id",
            reason: "must be non-empty and contain no control characters",
        });
    }
    if secret.trim().is_empty() || secret.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(IdentityError::InvalidInput {
            field: "app_secret",
            reason: "must be non-empty and contain no control characters",
        });
    }
    Ok(())
}

fn validate_exchange_request(request: &ExchangeRequest) -> Result<(), IdentityError> {
    validate_opaque_token(&request.short_lived_token, &["oac_"]).map_err(|_| IdentityError::InvalidInput {
        field: "short_lived_token",
        reason: "expected IAM's oac_ short-lived-token form",
    })?;
    if let Some(org) = &request.required_org_id {
        validate_org_id(org)?;
    }
    validate_idempotency_key(&request.idempotency_key)
}

fn validate_refresh_request(request: &RefreshRequest) -> Result<(), IdentityError> {
    validate_opaque_token(&request.refresh_token, &["ort_"]).map_err(|_| IdentityError::InvalidInput {
        field: "refresh_token",
        reason: "expected IAM's ort_ refresh-token form",
    })?;
    validate_org_id(&request.required_org_id)?;
    validate_idempotency_key(&request.idempotency_key)
}

fn validate_idempotency_key(value: &str) -> Result<(), IdentityError> {
    IdempotencyKey::parse(value.to_owned()).map(|_| ()).map_err(|_| IdentityError::InvalidInput {
        field: "idempotency_key",
        reason: "must be 16 to 255 visible ASCII characters",
    })
}

fn validate_org_id(org: &str) -> Result<(), IdentityError> {
    if org.trim().is_empty()
        || org
            .chars()
            .any(|character| character.is_control() || character.is_whitespace() || matches!(character, '/' | '\\'))
    {
        return Err(IdentityError::InvalidInput {
            field: "org_id",
            reason: "must not be empty or contain whitespace or path separators",
        });
    }
    Ok(())
}

fn validate_recording_proof_request(request: &RecordingProofRequest) -> Result<(), IdentityError> {
    validate_org_id(&request.expected_org_id)?;
    validate_idempotency_key(&request.idempotency_key)?;
    let valid_app = request.audience.split_once('>').is_some_and(|(org, app)| {
        org == request.expected_org_id
            && !app.is_empty()
            && app.len() <= 100
            && app.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    });
    if !valid_app {
        return Err(IdentityError::InvalidInput {
            field: "audience",
            reason: "expected a configured application in the session organization",
        });
    }
    if request.expected_actor_id.is_empty()
        || request.expected_actor_id.len() > 255
        || request.expected_actor_id.chars().any(char::is_control)
    {
        return Err(IdentityError::InvalidInput {
            field: "expected_actor_id",
            reason: "expected a bounded session initiator identifier",
        });
    }
    if request.path.len() > 4096
        || request.path.chars().any(char::is_control)
        || request.path.split('/').any(|part| matches!(part, "." | ".."))
        || request.name.is_empty()
        || request.name.len() > 255
        || matches!(request.name.as_str(), "." | "..")
        || request.name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\'))
        || request.content_type.is_empty()
        || request.content_type.len() > 255
        || !request.content_type.is_ascii()
        || request.content_type.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(IdentityError::InvalidInput {
            field: "metadata",
            reason: "invalid recording path, name, or content type",
        });
    }
    if request.body_sha256.len() != 64
        || !request.body_sha256.bytes().all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(IdentityError::InvalidInput {
            field: "body_sha256",
            reason: "expected a lowercase SHA-256 digest of the exact upload bytes",
        });
    }
    Ok(())
}

fn obo_sdk_error(error: silicon_iam_client::Error) -> IdentityError {
    use silicon_iam_client::Error;
    match error {
        Error::Api(error) if error.status == 401 || (error.status == 400 && error.code == "invalid_subject_token") => {
            IdentityError::Unauthenticated
        }
        Error::Api(error) if matches!(error.status, 403 | 404) => IdentityError::Forbidden,
        Error::Api(error) if error.status >= 500 => {
            IdentityError::Upstream { kind: UpstreamFailure::Unavailable, request_id: None, retry_after: None }
        }
        Error::Api(error) => IdentityError::Rejected {
            status: error.status,
            code: safe_upstream_identifier(&error.code).unwrap_or_else(|| "obo_request_rejected".into()),
            request_id: None,
        },
        Error::RateLimited { retry_after, .. } => IdentityError::Upstream {
            kind: UpstreamFailure::RateLimited,
            request_id: None,
            retry_after: Some(retry_after),
        },
        Error::Transport(_) | Error::UnstructuredResponse { .. } => {
            IdentityError::Upstream { kind: UpstreamFailure::Transport, request_id: None, retry_after: None }
        }
        _ => IdentityError::Contract {
            operation: "recording proof",
            reason: "IAM SDK rejected the OBO request or response contract",
        },
    }
}

fn validate_oat(token: &str) -> Result<(), IdentityError> {
    validate_opaque_token(token, &["oat_"]).map_err(|_| IdentityError::Unauthenticated)
}

fn validate_direct_iam_token(token: &str) -> Result<IdentityKind, IdentityError> {
    validate_opaque_token(token, &["cat_", "sat_"]).map_err(|_| IdentityError::Unauthenticated)?;
    if token.starts_with("cat_") { Ok(IdentityKind::Carbon) } else { Ok(IdentityKind::Silicon) }
}

fn validate_opaque_token(token: &str, prefixes: &[&str]) -> Result<(), ()> {
    let prefix = prefixes.iter().copied().find(|prefix| token.starts_with(prefix)).ok_or(())?;
    let value = token.strip_prefix(prefix).ok_or(())?;
    if value.len() == 43 && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')) {
        Ok(())
    } else {
        Err(())
    }
}

fn versioned_url(base: &Url, segments: &[&str]) -> Result<Url, IdentityError> {
    let mut url = base.clone();
    let mut path = url
        .path_segments_mut()
        .map_err(|_| IdentityError::InvalidInput { field: "iam_url", reason: "URL cannot carry path segments" })?;
    path.pop_if_empty().extend(["api", IAM_API_VERSION]).extend(segments);
    drop(path);
    Ok(url)
}

fn unversioned_url(base: &Url, segments: &[&str]) -> Result<Url, IdentityError> {
    let mut url = base.clone();
    let mut path = url
        .path_segments_mut()
        .map_err(|_| IdentityError::InvalidInput { field: "iam_url", reason: "URL cannot carry path segments" })?;
    path.pop_if_empty().extend(segments);
    drop(path);
    Ok(url)
}

fn form(fields: &[(&str, &str)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.extend_pairs(fields.iter().copied());
    serializer.finish()
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Deserialize)]
struct ErrorBody {
    code: String,
    #[serde(default)]
    request_id: Option<String>,
}

async fn decode_response<T: serde::de::DeserializeOwned>(
    operation: &'static str,
    response: Result<reqwest::Response, reqwest::Error>,
) -> Result<T, IdentityError> {
    let mut response = response.map_err(|_| IdentityError::Upstream {
        kind: UpstreamFailure::Transport,
        request_id: None,
        retry_after: None,
    })?;
    let status = response.status();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs);
    if response.content_length().is_some_and(|length| length > MAX_RESPONSE_BYTES as u64) {
        return Err(IdentityError::Upstream {
            kind: UpstreamFailure::InvalidResponse,
            request_id: None,
            retry_after: None,
        });
    }
    let mut body =
        Vec::with_capacity(response.content_length().unwrap_or_default().min(MAX_RESPONSE_BYTES as u64) as usize);
    while let Some(chunk) = response.chunk().await.map_err(|_| IdentityError::Upstream {
        kind: UpstreamFailure::Transport,
        request_id: None,
        retry_after: None,
    })? {
        if chunk.len() > MAX_RESPONSE_BYTES.saturating_sub(body.len()) {
            return Err(IdentityError::Upstream {
                kind: UpstreamFailure::InvalidResponse,
                request_id: None,
                retry_after: None,
            });
        }
        body.extend_from_slice(&chunk);
    }
    if status.is_success() {
        return serde_json::from_slice(&body)
            .map_err(|_| IdentityError::Contract { operation, reason: "IAM returned an unexpected success body" });
    }

    let envelope = serde_json::from_slice::<ErrorEnvelope>(&body).ok();
    let request_id =
        envelope.as_ref().and_then(|value| value.error.request_id.as_deref()).and_then(safe_upstream_identifier);
    let code = envelope
        .as_ref()
        .and_then(|value| safe_upstream_identifier(&value.error.code))
        .unwrap_or_else(|| "unrecognized_error".to_owned());
    match status.as_u16() {
        // OAuth defines an expired, revoked, or consumed grant as HTTP 400.
        // Surface it as authentication loss, so clients can request a new SLT.
        400 if code == "invalid_grant" => Err(IdentityError::Unauthenticated),
        401 => Err(IdentityError::Unauthenticated),
        403 | 404 => Err(IdentityError::Forbidden),
        429 => Err(IdentityError::Upstream {
            kind: UpstreamFailure::RateLimited,
            request_id,
            retry_after: Some(retry_after.unwrap_or(Duration::from_secs(1))),
        }),
        500..=599 => Err(IdentityError::Upstream { kind: UpstreamFailure::Unavailable, request_id, retry_after }),
        status => Err(IdentityError::Rejected { status, code, request_id }),
    }
}

fn safe_upstream_identifier(value: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
    .then(|| value.to_owned())
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    digest: [u8; 32],
    org: Option<String>,
}

impl CacheKey {
    fn new(token: &str, org: Option<&str>) -> Self {
        Self { digest: Sha256::digest(token.as_bytes()).into(), org: org.map(str::to_owned) }
    }
}

impl fmt::Debug for CacheKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("CacheKey").field("token", &"sha256:[REDACTED]").field("org", &self.org).finish()
    }
}

/// Deterministic fake for backend unit tests. Input credentials used as lookup
/// keys are hashed at insertion to avoid retaining raw credentials. Configured
/// exchange results necessarily contain the credentials a test expects back.
#[derive(Clone, Default)]
pub struct FakeIdentityProvider {
    state: Arc<RwLock<FakeState>>,
}

#[derive(Default)]
struct FakeState {
    identities: HashMap<CacheKey, PrincipalIdentity>,
    organizations: HashMap<[u8; 32], Result<Vec<OrganizationAccess>, IdentityError>>,
    exchanges: HashMap<([u8; 32], String), Result<ExchangedAuth, IdentityError>>,
    refreshes: HashMap<([u8; 32], String), Result<ExchangedAuth, IdentityError>>,
}

impl fmt::Debug for FakeIdentityProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        formatter
            .debug_struct("FakeIdentityProvider")
            .field("identities", &state.identities.len())
            .field("organizations", &state.organizations.len())
            .field("exchanges", &state.exchanges.len())
            .field("refreshes", &state.refreshes.len())
            .finish()
    }
}

impl FakeIdentityProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allow_identity(&self, bearer: &str, identity: PrincipalIdentity) {
        let key = CacheKey::new(bearer, Some(&identity.org_id));
        self.write().identities.insert(key, identity);
    }

    pub fn deny_identity(&self, bearer: &str, org_id: &str) {
        self.write().identities.remove(&CacheKey::new(bearer, Some(org_id)));
    }

    pub fn allow_orgs(&self, bearer: &str, organizations: Vec<OrganizationAccess>) {
        self.write().organizations.insert(token_digest(bearer), Ok(organizations));
    }

    pub fn fail_orgs(&self, bearer: &str, error: IdentityError) {
        self.write().organizations.insert(token_digest(bearer), Err(error));
    }

    pub fn allow_exchange(&self, short_lived_token: &str, required_org_id: &str, exchanged: ExchangedAuth) {
        self.write().exchanges.insert((token_digest(short_lived_token), required_org_id.to_owned()), Ok(exchanged));
    }

    pub fn fail_exchange(&self, short_lived_token: &str, required_org_id: &str, error: IdentityError) {
        self.write().exchanges.insert((token_digest(short_lived_token), required_org_id.to_owned()), Err(error));
    }

    pub fn allow_refresh(&self, refresh_token: &str, required_org_id: &str, exchanged: ExchangedAuth) {
        self.write().refreshes.insert((token_digest(refresh_token), required_org_id.to_owned()), Ok(exchanged));
    }

    pub fn fail_refresh(&self, refresh_token: &str, required_org_id: &str, error: IdentityError) {
        self.write().refreshes.insert((token_digest(refresh_token), required_org_id.to_owned()), Err(error));
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, FakeState> {
        self.state.write().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl IdentityProvider for FakeIdentityProvider {
    async fn identify(&self, bearer: &str, org: &str) -> Result<PrincipalIdentity, IdentityError> {
        self.state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .identities
            .get(&CacheKey::new(bearer, Some(org)))
            .cloned()
            .ok_or(IdentityError::Unauthenticated)
    }

    async fn orgs(&self, bearer: &str) -> Result<Vec<OrganizationAccess>, IdentityError> {
        self.state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .organizations
            .get(&token_digest(bearer))
            .cloned()
            .unwrap_or(Err(IdentityError::Unauthenticated))
    }

    async fn exchange_short_lived_token(&self, request: ExchangeRequest) -> Result<ExchangedAuth, IdentityError> {
        self.state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .exchanges
            .get(&(token_digest(&request.short_lived_token), request.required_org_id.unwrap_or_default()))
            .cloned()
            .unwrap_or(Err(IdentityError::Unauthenticated))
    }

    async fn refresh(&self, request: RefreshRequest) -> Result<ExchangedAuth, IdentityError> {
        self.state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .refreshes
            .get(&(token_digest(&request.refresh_token), request.required_org_id))
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
    use chrono::TimeDelta;
    use serde_json::json;
    use silicon_iam_client::models::ApiVersionNegotiation;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    const APP: &str = "tos>browser";
    const ORG: &str = "tos";

    fn oat(character: char) -> String {
        format!("oat_{}", character.to_string().repeat(43))
    }

    fn oac(character: char) -> String {
        format!("oac_{}", character.to_string().repeat(43))
    }

    fn claims() -> TokenIntrospection {
        TokenIntrospection {
            active: true,
            principal_id: Some(Uuid::from_u128(1)),
            actor_type: Some(TokenIntrospectionActorType::Silicon),
            client_id: Some(APP.to_owned()),
            org_id: Some(ORG.to_owned()),
            membership_id: Some(Uuid::from_u128(2)),
            session_id: Some(Uuid::from_u128(3)),
            scope: Some("browser memberships.read".to_owned()),
            audience: Some(APP.to_owned()),
            issued_at: Some(Utc::now().timestamp() - 1),
            expires_at: Some(Utc::now().timestamp() + 1_800),
            authorization_epoch: Some(7),
            authorization: Some(silicon_iam_client::models::ApplicationAuthorization {
                principal_id: Uuid::from_u128(1),
                actor_type: Some(ApplicationAuthorizationActorType::Silicon),
                public_id: Some("silicon-1".into()),
                organization_id: Uuid::from_u128(4),
                org_id: ORG.into(),
                membership_id: Uuid::from_u128(2),
                membership_version: 1,
                authorization_epoch: 7,
                audience: APP.into(),
                testing_environment_id: None,
                scopes: vec!["browser".into(), "memberships.read".into()],
                org_role: None,
                tags: Some(vec![silicon_iam_client::models::AuthorizationTag {
                    id: Uuid::from_u128(5),
                    name: "growth".into(),
                }]),
            }),
            authorizations: None,
        }
    }

    fn identity() -> PrincipalIdentity {
        identity_from_claims(&claims(), APP, ORG, Some("silicon-1".into()), Utc::now()).unwrap()
    }

    fn exchanged() -> ExchangedAuth {
        ExchangedAuth {
            access_token: oat('A'),
            refresh_token: format!("ort_{}", "B".repeat(43)),
            identity: identity(),
            scope: "browser".into(),
        }
    }

    #[test]
    fn active_claims_are_bound_to_app_org_actor_and_membership() {
        let identity = identity();
        assert_eq!(identity.principal_id, Uuid::from_u128(1));
        assert_eq!(identity.membership_id, Uuid::from_u128(2));
        assert_eq!(identity.public_id.as_deref(), Some("silicon-1"));

        let mut wrong_app = claims();
        wrong_app.audience = Some("another>app".into());
        assert_eq!(identity_from_claims(&wrong_app, APP, ORG, None, Utc::now()), Err(IdentityError::Unauthenticated));

        let mut wrong_org = claims();
        wrong_org.org_id = Some("elsewhere".into());
        assert_eq!(identity_from_claims(&wrong_org, APP, ORG, None, Utc::now()), Err(IdentityError::Forbidden));

        let mut application_actor = claims();
        application_actor.actor_type = Some(TokenIntrospectionActorType::Application);
        assert!(matches!(
            identity_from_claims(&application_actor, APP, ORG, None, Utc::now()),
            Err(IdentityError::Contract { .. })
        ));
    }

    #[test]
    fn authorization_snapshot_is_required_and_bound_to_every_identity_dimension() {
        for mutation in 0..10 {
            let mut inspected = claims();
            let snapshot = inspected.authorization.as_mut().unwrap();
            match mutation {
                0 => snapshot.principal_id = Uuid::new_v4(),
                1 => snapshot.actor_type = Some(ApplicationAuthorizationActorType::Carbon),
                2 => snapshot.org_id = "another-org".into(),
                3 => snapshot.membership_id = Uuid::new_v4(),
                4 => snapshot.authorization_epoch += 1,
                5 => snapshot.audience = "another>app".into(),
                6 => snapshot.public_id = Some(String::new()),
                7 => snapshot.scopes.push("roles.read".into()),
                8 => snapshot.org_role = Some("owner".into()),
                _ => inspected.authorization = None,
            }
            assert!(
                matches!(
                    identity_from_claims(&inspected, APP, ORG, None, Utc::now()),
                    Err(IdentityError::Contract { .. })
                ),
                "accepted mismatch {mutation}"
            );
        }
        assert!(identity_from_claims(&claims(), APP, ORG, Some("other-public-id".into()), Utc::now()).is_err());
    }

    #[test]
    fn public_identity_bootstraps_from_introspection_and_undisclosed_tags_grant_nothing() {
        let mut inspected = claims();
        let identity = identity_from_claims(&inspected, APP, ORG, None, Utc::now()).unwrap();
        assert_eq!(identity.public_id.as_deref(), Some("silicon-1"));
        assert_eq!(identity.tags, Some(vec!["growth".into()]));
        inspected.authorization.as_mut().unwrap().tags = None;
        assert_eq!(identity_from_claims(&inspected, APP, ORG, None, Utc::now()).unwrap().tags, None);
        inspected.scope = Some("browser".into());
        inspected.authorization.as_mut().unwrap().scopes = vec!["browser".into()];
        assert!(identity_from_claims(&inspected, APP, ORG, None, Utc::now()).is_ok());
        inspected.authorization.as_mut().unwrap().tags = Some(Vec::new());
        assert!(identity_from_claims(&inspected, APP, ORG, None, Utc::now()).is_err());
    }

    #[test]
    fn inactive_expired_or_incomplete_claims_fail_closed() {
        let mut inactive = claims();
        inactive.active = false;
        assert_eq!(identity_from_claims(&inactive, APP, ORG, None, Utc::now()), Err(IdentityError::Unauthenticated));

        let mut expired = claims();
        expired.expires_at = Some((Utc::now() - TimeDelta::seconds(1)).timestamp());
        assert_eq!(identity_from_claims(&expired, APP, ORG, None, Utc::now()), Err(IdentityError::Unauthenticated));

        let mut no_membership = claims();
        no_membership.membership_id = None;
        assert!(matches!(
            identity_from_claims(&no_membership, APP, ORG, None, Utc::now()),
            Err(IdentityError::Contract { .. })
        ));
    }

    #[test]
    fn org_bound_oat_yields_one_org_and_unbound_oat_reports_capability() {
        assert_eq!(
            organizations_from_claims(&claims(), APP, Utc::now()).unwrap(),
            vec![OrganizationAccess { id: ORG.into(), name: None }]
        );

        let mut unbound = claims();
        unbound.org_id = None;
        unbound.membership_id = None;
        unbound.authorizations = Some(Vec::new());
        assert_eq!(organizations_from_claims(&unbound, APP, Utc::now()).unwrap(), Vec::<OrganizationAccess>::new());
    }

    #[test]
    fn token_validation_accepts_only_exact_iam_opaque_forms() {
        assert!(validate_oat(&oat('A')).is_ok());
        assert!(validate_oat(&format!("oat_{}", "A".repeat(42))).is_err());
        assert!(validate_oat(&format!("cat_{}", "A".repeat(43))).is_err());
        assert!(
            validate_exchange_request(&ExchangeRequest {
                short_lived_token: oac('C'),
                required_org_id: Some(ORG.into()),
                idempotency_key: "0123456789abcdef".into(),
            })
            .is_ok()
        );
        assert!(
            validate_refresh_request(&RefreshRequest {
                refresh_token: format!("ort_{}", "D".repeat(43)),
                required_org_id: ORG.into(),
                idempotency_key: "fedcba9876543210".into(),
            })
            .is_ok()
        );
        assert!(
            validate_refresh_request(&RefreshRequest {
                refresh_token: oat('D'),
                required_org_id: ORG.into(),
                idempotency_key: "fedcba9876543210".into(),
            })
            .is_err()
        );
    }

    fn recording_request() -> RecordingProofRequest {
        RecordingProofRequest {
            expected_org_id: ORG.into(),
            expected_actor_id: "silicon-1".into(),
            audience: "tos>briefcase".into(),
            path: String::new(),
            name: "session-recording.webm".into(),
            content_type: "video/webm".into(),
            body_sha256: silicon_iam_client::api::obo::body_sha256(b"exact recording bytes"),
            idempotency_key: "recording-proof-attempt-0001".into(),
        }
    }

    #[test]
    fn recording_proof_inputs_reject_cross_org_audiences_and_unsafe_metadata() {
        assert!(validate_recording_proof_request(&recording_request()).is_ok());
        for mutation in 0..7 {
            let mut request = recording_request();
            match mutation {
                0 => request.audience = "other>briefcase".into(),
                1 => request.body_sha256 = "A".repeat(64),
                2 => request.name = "../recording".into(),
                3 => request.path = "private/../another".into(),
                4 => request.content_type = "video/webm\r\nx-header: bad".into(),
                5 => request.expected_actor_id.clear(),
                _ => request.idempotency_key.clear(),
            }
            assert!(validate_recording_proof_request(&request).is_err());
        }
    }

    #[tokio::test]
    async fn recording_proof_requires_the_live_initiator_and_delegation_scopes() {
        for wrong_actor in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                read_request_head(&mut stream).await;
                let mut claims = claims();
                if wrong_actor {
                    claims.scope = Some("browser memberships.read roles.read obo.issue".into());
                    claims.authorization.as_mut().unwrap().scopes =
                        vec!["browser".into(), "memberships.read".into(), "roles.read".into(), "obo.issue".into()];
                }
                write_test_response(&mut stream, "200 OK", &[], &serde_json::to_string(&claims).unwrap()).await;
            });
            let provider =
                SiliconIamIdentityProvider::from_parts(reqwest::Client::new(), base, APP.into(), "app-secret".into())
                    .unwrap();
            let mut request = recording_request();
            if wrong_actor {
                request.expected_actor_id = "different-viewer".into();
            }
            assert_eq!(provider.issue_recording_proof(&oat('A'), request).await, Err(IdentityError::Forbidden));
            server.await.unwrap();
        }
        assert!(matches!(
            FakeIdentityProvider::new().issue_recording_proof(&oat('A'), recording_request()).await,
            Err(IdentityError::CapabilityUnavailable(IdentityCapability::RecordingProof))
        ));
    }

    #[tokio::test]
    async fn recording_proof_sdk_binds_exact_bytes_metadata_actor_app_and_testing_plane() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let environment_key = "T".repeat(32);
        let expected_environment = environment_key.clone();
        let request = recording_request();
        let expected_digest = request.body_sha256.clone();
        let expected_key = request.idempotency_key.clone();
        let raw_proof = format!("obo_{}", "P".repeat(43));
        let returned_proof = raw_proof.clone();
        let expires_at = Utc::now() + chrono::TimeDelta::seconds(50);
        let expected_expiry = expires_at;
        let server = tokio::spawn(async move {
            for index in 0..4 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_request_head(&mut stream).await;
                let lower = request.to_ascii_lowercase();
                assert!(request.contains(&format!("x-testing-environment-key: {expected_environment}\r\n")));
                assert!(lower.contains("silicon-iam-supported-api-versions: v1\r\n"));
                let body = match index {
                    0 => {
                        json!({"service":"silicon-iam","selected_api_version":"v1","supported_api_versions":["v1"],"build":"test","commit":"test"})
                    }
                    1 => {
                        assert!(request.starts_with("POST /api/v1/oauth/introspect "));
                        assert!(lower.contains("authorization: basic "));
                        assert!(lower.contains("x-org-id: tos\r\n"));
                        let mut claims = claims();
                        claims.scope = Some("browser memberships.read roles.read obo.issue".into());
                        let snapshot = claims.authorization.as_mut().unwrap();
                        snapshot.scopes =
                            vec!["browser".into(), "memberships.read".into(), "roles.read".into(), "obo.issue".into()];
                        snapshot.testing_environment_id = Some(Uuid::from_u128(10));
                        serde_json::to_value(claims).unwrap()
                    }
                    2 => {
                        assert!(request.starts_with("GET /api/v1/obo-access/applications/tos%3Ebriefcase/endpoints "));
                        assert!(lower.contains("authorization: basic "));
                        assert!(!lower.contains("x-org-id:"));
                        json!({"application":{"app_id":"tos>briefcase","org_id":ORG},"endpoints":[{
                            "critical":false,"endpoint_id":RECORDING_ENDPOINT_ID,"path":RECORDING_ENDPOINT_PATH,
                            "metadata":{"path":{"type":"string"},"name":{"type":"string"},"content_type":{"type":"string"}}
                        }]})
                    }
                    _ => {
                        assert!(request.starts_with("POST /api/v1/obo-access/exchanges "));
                        assert!(lower.contains("authorization: basic "));
                        assert!(!lower.contains("x-org-id:"));
                        assert!(request.contains(&format!("idempotency-key: {expected_key}\r\n")));
                        assert!(lower.contains("x-obo-timestamp:"));
                        let signature =
                            request.lines().find_map(|line| line.strip_prefix("x-obo-signature: ")).unwrap();
                        assert_eq!(signature.len(), 64);
                        let wire: serde_json::Value =
                            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
                        assert_eq!(
                            wire,
                            json!({"org_id":ORG,"subject_token":oat('A'),"audience":"tos>briefcase","endpoint_id":RECORDING_ENDPOINT_ID,
                            "metadata":{"path":"","name":"session-recording.webm","content_type":"video/webm"},
                            "request":{"method":"POST","body_sha256":expected_digest}})
                        );
                        json!({"access_proof":returned_proof,"proof_id":Uuid::from_u128(11),"expires_in":60,"expires_at":expires_at.to_rfc3339()})
                    }
                };
                write_test_response(&mut stream, "200 OK", &[], &body.to_string()).await;
            }
        });
        let provider = SiliconIamIdentityProvider::connect_with_environment(
            &base,
            APP.into(),
            "app-secret".into(),
            Some(&environment_key),
        )
        .await
        .unwrap();
        let proof = provider.issue_recording_proof(&oat('A'), request).await.unwrap();
        assert_eq!(proof.grant.expose(), raw_proof);
        assert_eq!(proof.proof_id, Uuid::from_u128(11));
        assert_eq!(proof.expires_at.timestamp(), expected_expiry.timestamp());
        assert!(!format!("{proof:?}").contains(&raw_proof));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn late_delivery_replay_recovers_only_a_current_same_principal_refresh_credential() {
        for wrong_principal in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
            let response:OAuthTokenResponse=serde_json::from_value(json!({"access_token":oat('A'),"refresh_token":format!("ort_{}","R".repeat(43)),"token_type":"Bearer","expires_in":1800,"scope":"obo.issue memberships.read roles.read","org_id":ORG,"actor":{"principal_id":Uuid::from_u128(1),"public_id":"silicon-1","type":"silicon"}})).unwrap();
            let server = tokio::spawn(async move {
                for index in 0..3 {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let request = read_request_head(&mut stream).await;
                    let mut current = claims();
                    if index < 2 {
                        current.active = false;
                        current.authorization = None;
                    } else {
                        assert!(request.contains("token_type_hint=refresh_token"));
                        assert!(request.contains("token=ort_"));
                        current.authorization = None;
                        current.scope = Some("obo.issue memberships.read roles.read".into());
                        if wrong_principal {
                            current.principal_id = Some(Uuid::from_u128(99));
                        }
                    }
                    write_test_response(&mut stream, "200 OK", &[], &serde_json::to_string(&current).unwrap()).await;
                }
            });
            let provider =
                SiliconIamIdentityProvider::from_parts(reqwest::Client::new(), base, APP.into(), "secret".into())
                    .unwrap();
            // The ordinary browser login API still rejects the expired OAT.
            assert!(matches!(
                provider.validate_exchanged(response.clone(), Some(ORG), "test").await,
                Err(IdentityError::Unauthenticated)
            ));
            let recovered = provider.validate_delivery_exchange(response, ORG, "test recovery").await;
            if wrong_principal {
                assert!(matches!(recovered, Err(IdentityError::Forbidden)));
            } else {
                let recovered = recovered.unwrap();
                assert!(!recovered.access_active);
                assert_eq!(recovered.auth.identity.principal_id, Uuid::from_u128(1));
                assert!(recovered.auth.identity.tags.is_none());
            }
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn application_revoke_uses_owned_family_and_stable_mutation_key() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let token = format!("ort_{}", "R".repeat(43));
        let expected_token = token.clone();
        let key = Uuid::new_v4().to_string();
        let expected_key = key.clone();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_request_head(&mut stream).await;
            assert!(request.starts_with("POST /api/v1/oauth/revoke "));
            assert!(request.to_lowercase().contains("authorization: basic "));
            assert!(request.contains(&format!("idempotency-key: {expected_key}\r\n")));
            assert!(request.ends_with(&format!("token={expected_token}")));
            write_test_response(&mut stream, "204 No Content", &[], "").await;
        });
        let provider =
            SiliconIamIdentityProvider::from_parts(reqwest::Client::new(), base, APP.into(), "secret".into()).unwrap();
        assert!(matches!(
            provider.revoke_application_token("not-a-token", &key).await,
            Err(IdentityError::InvalidInput { .. })
        ));
        provider.revoke_application_token(&token, &key).await.unwrap();
        server.await.unwrap();
    }

    #[test]
    fn fake_keys_are_digests() {
        let raw = oat('S');
        let key = CacheKey::new(&raw, Some(ORG));
        assert_eq!(key.digest, token_digest(&raw));
        assert!(!format!("{key:?}").contains(&raw));
    }

    #[test]
    fn negotiation_is_strict_about_service_and_version_contract() {
        let valid = ApiVersionNegotiation {
            service: json!("silicon-iam"),
            selected_api_version: "v1".into(),
            supported_api_versions: vec!["v1".into()],
            build: "1.1.0".into(),
            commit: "abc".into(),
        };
        assert!(validate_negotiation(&valid).is_ok());
        let mut impostor = valid.clone();
        impostor.service = json!("not-iam");
        assert!(validate_negotiation(&impostor).is_err());
        let mut future = valid.clone();
        future.supported_api_versions = vec!["v2".into(), "v1".into()];
        assert!(validate_negotiation(&future).is_ok(), "v1 remains compatible when newer APIs are offered");
        for catalog in [
            vec!["v1", "v1"],
            vec!["v1", "v2"],
            vec!["v1", "v0"],
            vec!["v01", "v1"],
            vec!["v1000000000", "v1"],
            vec!["v2"],
        ] {
            let mut malformed = valid.clone();
            malformed.supported_api_versions = catalog.into_iter().map(String::from).collect();
            assert!(validate_negotiation(&malformed).is_err());
        }
    }

    #[tokio::test]
    async fn fake_is_org_scoped_and_never_retains_raw_credentials() {
        let fake = FakeIdentityProvider::new();
        let bearer = oat('F');
        let slt = oac('G');
        let refresh = format!("ort_{}", "H".repeat(43));
        fake.allow_identity(&bearer, identity());
        fake.allow_orgs(&bearer, vec![OrganizationAccess { id: ORG.into(), name: None }]);
        fake.allow_exchange(&slt, ORG, exchanged());
        fake.allow_refresh(&refresh, ORG, exchanged());

        assert_eq!(fake.identify(&bearer, ORG).await.unwrap().public_id.as_deref(), Some("silicon-1"));
        assert_eq!(fake.identify(&bearer, "other").await, Err(IdentityError::Unauthenticated));
        assert_eq!(fake.orgs(&bearer).await.unwrap()[0].id, ORG);
        let result = fake
            .exchange_short_lived_token(ExchangeRequest {
                short_lived_token: slt.clone(),
                required_org_id: Some(ORG.into()),
                idempotency_key: "ignored-by-fake".into(),
            })
            .await
            .unwrap();
        assert_eq!(result.identity.org_id, ORG);
        let refreshed = fake
            .refresh(RefreshRequest {
                refresh_token: refresh.clone(),
                required_org_id: ORG.into(),
                idempotency_key: "ignored-by-fake".into(),
            })
            .await
            .unwrap();
        assert_eq!(refreshed.identity.org_id, ORG);
        assert_eq!(
            fake.refresh(RefreshRequest {
                refresh_token: refresh.clone(),
                required_org_id: "other".into(),
                idempotency_key: "ignored-by-fake".into(),
            })
            .await,
            Err(IdentityError::Unauthenticated)
        );
        let debug = format!("{fake:?}");
        assert!(!debug.contains(&bearer));
        assert!(!debug.contains(&slt));
        assert!(!debug.contains(&refresh));
        assert!(!format!("{result:?}").contains(&result.access_token));
    }

    #[test]
    fn form_encoding_does_not_put_credentials_in_a_url() {
        let encoded = form(&[("app_id", "tos>browser"), ("slt", "oac_a+b/c")]);
        assert_eq!(encoded, "app_id=tos%3Ebrowser&slt=oac_a%2Bb%2Fc");
    }

    #[tokio::test]
    async fn plaintext_remote_iam_endpoint_is_rejected_before_network_io() {
        let error = SiliconIamIdentityProvider::connect("http://iam.example", APP.into(), "app-secret".into())
            .await
            .unwrap_err();
        assert_eq!(
            error,
            IdentityError::InvalidInput { field: "iam_url", reason: "expected HTTPS unless the IAM host is loopback" }
        );
    }

    #[tokio::test]
    async fn every_request_observes_current_tags_and_token_revocation() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            for index in 0..3 {
                let (mut stream, _) = listener.accept().await.unwrap();
                read_request_head(&mut stream).await;
                let mut response = claims();
                if index == 1 {
                    response.authorization.as_mut().unwrap().tags = Some(Vec::new());
                }
                if index == 2 {
                    response.active = false;
                    response.authorization = None;
                }
                write_test_response(&mut stream, "200 OK", &[], &serde_json::to_string(&response).unwrap()).await;
            }
        });
        let provider =
            SiliconIamIdentityProvider::from_parts(reqwest::Client::new(), base, APP.into(), "app-secret".into())
                .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            assert_eq!(provider.identify(&oat('A'), ORG).await.unwrap().tags, Some(vec!["growth".into()]));
            assert_eq!(provider.identify(&oat('A'), ORG).await.unwrap().tags, Some(Vec::new()));
            assert_eq!(provider.identify(&oat('A'), ORG).await, Err(IdentityError::Unauthenticated));
            server.await.unwrap();
        })
        .await
        .expect("every request must introspect current authority");
    }

    #[tokio::test]
    async fn upstream_errors_preserve_authentication_and_retry_semantics_without_body_leaks() {
        let cases = [
            ("400 Bad Request", "invalid_grant", None, IdentityError::Unauthenticated),
            (
                "400 Bad Request",
                "invalid_request",
                None,
                IdentityError::Rejected {
                    status: 400,
                    code: "invalid_request".into(),
                    request_id: Some("request-1".into()),
                },
            ),
            ("401 Unauthorized", "unauthorized", None, IdentityError::Unauthenticated),
            ("403 Forbidden", "forbidden", None, IdentityError::Forbidden),
            ("404 Not Found", "missing", None, IdentityError::Forbidden),
            (
                "429 Too Many Requests",
                "rate_limited",
                None,
                IdentityError::Upstream {
                    kind: UpstreamFailure::RateLimited,
                    request_id: Some("request-1".into()),
                    retry_after: Some(Duration::from_secs(1)),
                },
            ),
            (
                "429 Too Many Requests",
                "rate_limited",
                Some("12"),
                IdentityError::Upstream {
                    kind: UpstreamFailure::RateLimited,
                    request_id: Some("request-1".into()),
                    retry_after: Some(Duration::from_secs(12)),
                },
            ),
            (
                "503 Unavailable",
                "unavailable",
                Some("3"),
                IdentityError::Upstream {
                    kind: UpstreamFailure::Unavailable,
                    request_id: Some("request-1".into()),
                    retry_after: Some(Duration::from_secs(3)),
                },
            ),
        ];
        for (status, code, retry_after, expected) in cases {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                read_request_head(&mut stream).await;
                let body = json!({"error": {"code": code, "request_id": "request-1",
                    "message": "credential-should-never-be-reflected"}})
                .to_string();
                let headers = retry_after.map(|value| vec![("Retry-After", value)]).unwrap_or_default();
                write_test_response(&mut stream, status, &headers, &body).await;
            });
            let actual = decode_response::<serde_json::Value>("test", reqwest::get(url).await).await.unwrap_err();
            assert_eq!(actual, expected);
            assert!(!format!("{actual:?}").contains("credential-should-never-be-reflected"));
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn invalid_environment_keys_are_rejected_without_network_or_secret_disclosure() {
        for key in ["", "secret-invalid-key", &"a".repeat(31), &"a".repeat(33), &"_".repeat(32)] {
            let error = SiliconIamIdentityProvider::connect_with_environment(
                "http://127.0.0.1:1",
                APP.into(),
                "app-secret".into(),
                Some(key),
            )
            .await
            .unwrap_err();
            assert_eq!(
                error,
                IdentityError::InvalidInput {
                    field: "iam_test_environment_key",
                    reason: "expected exactly 32 alphanumeric characters",
                }
            );
            if !key.is_empty() {
                assert!(!format!("{error:?}").contains(key));
            }
        }
    }

    #[tokio::test]
    async fn every_iam_operation_stays_in_the_selected_environment() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let key = "T".repeat(32);
        let expected_key = key.clone();
        let server = tokio::spawn(async move {
            for index in 0..7 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let head = read_request_head(&mut stream).await;
                assert!(head.contains(&format!("x-testing-environment-key: {expected_key}\r\n")));
                if index == 0 {
                    assert!(head.starts_with("GET /api/version "));
                    let body = json!({ "service": "silicon-iam", "selected_api_version": "v1",
                        "supported_api_versions": ["v1"], "build": "test", "commit": "test" })
                    .to_string();
                    write_test_response(&mut stream, "200 OK", &[], &body).await;
                } else {
                    assert!(head.to_ascii_lowercase().contains("authorization: "));
                    write_test_response(&mut stream, "401 Unauthorized", &[], "{}").await;
                }
            }
        });
        let provider =
            SiliconIamIdentityProvider::connect_with_environment(&base, APP.into(), "app-secret".into(), Some(&key))
                .await
                .unwrap();
        assert!(!format!("{provider:?}").contains(&key));
        assert_eq!(provider.identify(&oat('A'), ORG).await, Err(IdentityError::Unauthenticated));
        assert_eq!(provider.orgs(&oat('A')).await, Err(IdentityError::Unauthenticated));
        let direct = format!("cat_{}", "C".repeat(43));
        assert!(matches!(provider.direct_iam_orgs(&direct).await, Err(IdentityError::Unauthenticated)));
        assert!(matches!(provider.direct_iam_identity(&direct, ORG).await, Err(IdentityError::Unauthenticated)));
        assert_eq!(
            provider
                .exchange_short_lived_token(ExchangeRequest {
                    short_lived_token: oac('B'),
                    required_org_id: Some(ORG.into()),
                    idempotency_key: "0123456789abcdef".into(),
                })
                .await,
            Err(IdentityError::Unauthenticated)
        );
        assert_eq!(
            provider
                .refresh(RefreshRequest {
                    refresh_token: format!("ort_{}", "D".repeat(43)),
                    required_org_id: ORG.into(),
                    idempotency_key: "fedcba9876543210".into(),
                })
                .await,
            Err(IdentityError::Unauthenticated)
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn credential_bearing_iam_requests_never_follow_redirects() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut negotiation, _) = listener.accept().await.unwrap();
            let negotiation_head = read_request_head(&mut negotiation).await;
            assert!(negotiation_head.starts_with("GET /api/version "));
            let body = serde_json::json!({
                "service": "silicon-iam",
                "selected_api_version": "v1",
                "supported_api_versions": ["v1"],
                "build": "test",
                "commit": "test"
            })
            .to_string();
            write_test_response(&mut negotiation, "200 OK", &[], &body).await;

            let (mut directory, _) = listener.accept().await.unwrap();
            let directory_head = read_request_head(&mut directory).await;
            assert!(directory_head.starts_with("GET /api/v1/organizations?"));
            assert!(directory_head.to_ascii_lowercase().contains("authorization: bearer cat_"));
            write_test_response(&mut directory, "302 Found", &[("Location", "/credential-sink")], "").await;

            match tokio::time::timeout(Duration::from_millis(250), listener.accept()).await {
                Ok(Ok((mut redirected, _))) => {
                    let _ = read_request_head(&mut redirected).await;
                    write_test_response(&mut redirected, "500 Error", &[], "").await;
                    true
                }
                _ => false,
            }
        });

        let provider = SiliconIamIdentityProvider::connect(&base, APP.into(), "app-secret".into()).await.unwrap();
        let bearer = format!("cat_{}", "A".repeat(43));
        assert!(matches!(provider.direct_iam_orgs(&bearer).await, Err(IdentityError::Rejected { status: 302, .. })));
        assert!(!server.await.unwrap(), "the IAM HTTP client followed a redirect carrying a bearer token");
    }

    async fn read_request_head(stream: &mut TcpStream) -> String {
        let mut request = Vec::new();
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let mut chunk = [0_u8; 1024];
            let read = stream.read(&mut chunk).await.unwrap();
            assert!(read > 0);
            request.extend_from_slice(&chunk[..read]);
            assert!(request.len() <= 64 * 1024);
        }
        let header_end = request.windows(4).position(|window| window == b"\r\n\r\n").unwrap() + 4;
        let content_length = String::from_utf8_lossy(&request[..header_end])
            .lines()
            .find_map(|line| line.split_once(':').filter(|(name, _)| name.eq_ignore_ascii_case("content-length")))
            .map(|(_, value)| value.trim().parse::<usize>().unwrap())
            .unwrap_or(0);
        assert!(header_end + content_length <= 64 * 1024);
        while request.len() < header_end + content_length {
            let mut chunk = [0_u8; 1024];
            let read = stream.read(&mut chunk).await.unwrap();
            assert!(read > 0);
            request.extend_from_slice(&chunk[..read]);
        }
        String::from_utf8(request).unwrap()
    }

    async fn write_test_response(stream: &mut TcpStream, status: &str, headers: &[(&str, &str)], body: &str) {
        let mut response = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
        for (name, value) in headers {
            response.push_str(name);
            response.push_str(": ");
            response.push_str(value);
            response.push_str("\r\n");
        }
        response.push_str("\r\n");
        response.push_str(body);
        stream.write_all(response.as_bytes()).await.unwrap();
        stream.shutdown().await.unwrap();
    }
}
