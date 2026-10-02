//! The client itself: configuration, and the one place a request is sent.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use reqwest::{Method, StatusCode, header::HeaderMap};
use serde::{Serialize, de::DeserializeOwned};
use url::Url;

use crate::{
    credentials::{Credential, EnvironmentKey},
    error::{ApiError, Envelope, Error, Result},
    models,
    request::Mutation,
    update::UpdateStatus,
};

/// The API version this client speaks.
pub const API_VERSION: &str = "v1";

const SUPPORTED_VERSIONS_HEADER: &str = "silicon-iam-supported-api-versions";
const SELECTED_VERSION_HEADER: &str = "silicon-iam-api-version";
const ENVIRONMENT_KEY_HEADER: &str = "x-testing-environment-key";
const MAX_RESPONSE_BODY_BYTES: usize = 4 * 1024 * 1024;

/// A configured Silicon IAM client.
///
/// The client stores no IAM session state, never caches an API response, and
/// never refreshes a credential on your behalf -- a token that expires
/// produces an error, and renewing it is the caller's decision. Dependency
/// versions are controlled by the consuming project. This client never updates
/// its own code or the project lockfile. It can be shared across tasks and threads.
#[derive(Clone, Debug)]
pub struct Client {
    http: reqwest::Client,
    base_url: Url,
    credential: Credential,
    environment: Option<EnvironmentKey>,
    testing_application: Option<Credential>,
    telemetry: Option<crate::telemetry::Telemetry>,
    telemetry_enabled: bool,
}

/// Assembles a [`Client`].
#[derive(Debug)]
pub struct ClientBuilder {
    base_url: Url,
    credential: Credential,
    environment: Option<EnvironmentKey>,
    testing_application: Option<Credential>,
    timeout: Duration,
    user_agent: Option<String>,
    telemetry_enabled: bool,
}

impl Client {
    /// Starts building a client for a service base URL.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invalid`] when the URL cannot be parsed or is not
    /// `http`/`https`.
    pub fn builder(base_url: &str) -> Result<ClientBuilder> {
        ClientBuilder::new(base_url)
    }

    /// An anonymous client, which is all the signup and login routes need.
    ///
    /// # Errors
    ///
    /// Returns an error when the URL is unusable or the HTTP stack cannot be
    /// initialized.
    pub fn new(base_url: &str) -> Result<Self> {
        ClientBuilder::new(base_url)?.build()
    }

    /// The service this client talks to.
    #[must_use]
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// The credential every request from this client presents.
    #[must_use]
    pub fn credential(&self) -> &Credential {
        &self.credential
    }

    /// The testing environment this client executes inside, if any.
    #[must_use]
    pub fn environment(&self) -> Option<&EnvironmentKey> {
        self.environment.as_ref()
    }

    /// Runtime dependency updates are disabled, including for older builder settings.
    #[must_use]
    pub fn update_status(&self) -> UpdateStatus {
        UpdateStatus::Disabled
    }

    /// The same client, presenting a different credential.
    ///
    /// Cheap: the underlying connection pool is shared. Use it to move from
    /// the anonymous client that performed a login to an authenticated one.
    #[must_use]
    pub fn with_credential(&self, credential: Credential) -> Self {
        Self {
            credential,
            ..self.clone()
        }
    }

    /// Selects testing using an application credential, without environment root authority.
    /// The selector is retained when switching to an OAuth bearer for scoped reads.
    ///
    /// # Errors
    /// Rejects malformed application credentials locally.
    pub fn with_testing_application(&self, app_id: &str, secret: &str) -> Result<Self> {
        if !(1..=80).contains(&app_id.len())
            || !app_id
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_lowercase)
            || !app_id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
            || secret.len() != 47
            || !secret.starts_with("ask_")
            || !secret[4..]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(Error::Invalid(
                "invalid testing application credential".into(),
            ));
        }
        Ok(Self {
            environment: None,
            testing_application: Some(Credential::application(app_id, secret)),
            ..self.clone()
        })
    }

    /// The same client, executing inside a testing environment.
    ///
    /// Every subsequent request runs against that environment's data instead
    /// of production, over the identical routes -- which is the whole point of
    /// an environment, and why this is a property of the client rather than a
    /// separate set of methods.
    #[must_use]
    pub fn with_environment(&self, environment: EnvironmentKey) -> Self {
        Self {
            environment: Some(environment),
            testing_application: None,
            ..self.clone()
        }
    }

    /// The same client, back on production data.
    #[must_use]
    pub fn without_environment(&self) -> Self {
        Self {
            environment: None,
            testing_application: None,
            ..self.clone()
        }
    }

    /// Builds a request against a versioned API route.
    ///
    /// Routes are named as path segments rather than a formatted string, so a
    /// value carrying a slash or a `..` becomes one escaped segment instead of
    /// changing which route is called.
    pub(crate) fn route(
        &self,
        method: Method,
        segments: &[&str],
    ) -> Result<reqwest::RequestBuilder> {
        Ok(self.prepare(method, self.versioned_url(segments)?))
    }

    fn versioned_url(&self, segments: &[&str]) -> Result<Url> {
        let mut url = self.base_url.clone();
        {
            let Ok(mut path) = url.path_segments_mut() else {
                return Err(Error::Invalid(
                    "the base URL cannot carry a path".to_owned(),
                ));
            };
            path.pop_if_empty()
                .extend(["api", API_VERSION])
                .extend(segments);
        }
        // Brackets delimit the organization in public membership identifiers.
        // Encode them explicitly for HTTP proxies as Url preserves them in paths.
        let path = url.path().replace('[', "%5B").replace(']', "%5D");
        url.set_path(&path);
        Ok(url)
    }

    /// Builds a request against a route that sits outside `/api/v1`.
    pub(crate) fn unversioned(
        &self,
        method: Method,
        segments: &[&str],
    ) -> Result<reqwest::RequestBuilder> {
        let mut url = self.base_url.clone();
        {
            let Ok(mut path) = url.path_segments_mut() else {
                return Err(Error::Invalid(
                    "the base URL cannot carry a path".to_owned(),
                ));
            };
            path.pop_if_empty().extend(segments);
        }
        Ok(self.prepare(method, url))
    }

    fn prepare(&self, method: Method, url: Url) -> reqwest::RequestBuilder {
        let mut request = self
            .http
            .request(method, url)
            .header(SUPPORTED_VERSIONS_HEADER, API_VERSION)
            .header("x-request-id", uuid::Uuid::now_v7().to_string())
            .header(
                "x-iam-telemetry",
                if self.telemetry_enabled { "on" } else { "off" },
            );
        request = self.credential.apply(request);
        if let Some(Credential::Application { app_id, secret }) = &self.testing_application {
            use base64::Engine as _;
            use secrecy::ExposeSecret as _;
            let encoded = base64::engine::general_purpose::STANDARD.encode(format!(
                "{}:{}",
                app_id,
                secret.expose_secret()
            ));
            let value = format!("Basic {encoded}");
            request = match http::HeaderValue::from_str(&value) {
                Ok(mut header) => {
                    header.set_sensitive(true);
                    request.header("x-testing-application", header)
                }
                Err(_) => request.header("x-testing-application", value),
            };
        }
        if let Some(environment) = &self.environment {
            request = request.header(ENVIRONMENT_KEY_HEADER, environment.expose());
        }
        request
    }

    pub(crate) async fn get<R: DeserializeOwned>(&self, segments: &[&str]) -> Result<R> {
        self.send_json(self.route(Method::GET, segments)?).await
    }

    pub(crate) fn route_with(
        &self,
        method: Method,
        segments: &[&str],
        query: &[(&str, String)],
    ) -> Result<reqwest::RequestBuilder> {
        let mut url = self.versioned_url(segments)?;
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (name, value) in query {
                pairs.append_pair(name, value);
            }
        }
        Ok(self.prepare(method, url))
    }
    pub(crate) async fn get_with<R: DeserializeOwned>(
        &self,
        segments: &[&str],
        query: &[(&str, String)],
    ) -> Result<R> {
        self.send_json(self.route_with(Method::GET, segments, query)?)
            .await
    }

    pub(crate) async fn post<B: Serialize, R: DeserializeOwned>(
        &self,
        segments: &[&str],
        body: &B,
        mutation: &Mutation,
    ) -> Result<R> {
        let request = mutation
            .apply(self.route(Method::POST, segments)?)
            .json(body);
        self.send_json(request).await
    }

    pub(crate) async fn post_empty<B: Serialize>(
        &self,
        segments: &[&str],
        body: &B,
        mutation: &Mutation,
    ) -> Result<()> {
        let request = mutation
            .apply(self.route(Method::POST, segments)?)
            .json(body);
        self.send_empty(request).await
    }

    /// A POST whose route also takes an `If-Match` precondition.
    pub(crate) async fn post_versioned<B: Serialize, R: DeserializeOwned>(
        &self,
        segments: &[&str],
        version: i64,
        body: &B,
        mutation: &Mutation,
    ) -> Result<R> {
        let request = mutation
            .apply(self.route(Method::POST, segments)?)
            .header(reqwest::header::IF_MATCH, etag(version))
            .json(body);
        self.send_json(request).await
    }

    /// A partial update. The contract takes these as JSON merge-patch, where
    /// an explicit `null` clears a field and an absent one leaves it alone.
    pub(crate) async fn patch<B: Serialize, R: DeserializeOwned>(
        &self,
        segments: &[&str],
        version: i64,
        body: &B,
        mutation: &Mutation,
    ) -> Result<R> {
        let request = mutation
            .apply(self.route(Method::PATCH, segments)?)
            .header(reqwest::header::IF_MATCH, etag(version))
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/merge-patch+json",
            )
            .body(serde_json::to_vec(body).map_err(|error| {
                Error::Invalid(format!("the patch could not be encoded: {error}"))
            })?);
        self.send_json(request).await
    }

    pub(crate) async fn put<B: Serialize, R: DeserializeOwned>(
        &self,
        segments: &[&str],
        version: i64,
        body: &B,
        mutation: &Mutation,
    ) -> Result<R> {
        let request = mutation
            .apply(self.route(Method::PUT, segments)?)
            .header(reqwest::header::IF_MATCH, etag(version))
            .json(body);
        self.send_json(request).await
    }

    /// A PUT whose `If-Match` is required only when a representation already
    /// exists, which is how the webhook routes are specified: the first
    /// configuration has nothing to match against.
    pub(crate) async fn put_optional_version<B: Serialize, R: DeserializeOwned>(
        &self,
        segments: &[&str],
        version: Option<i64>,
        body: &B,
        mutation: &Mutation,
    ) -> Result<R> {
        let mut request = mutation.apply(self.route(Method::PUT, segments)?);
        if let Some(version) = version {
            request = request.header(reqwest::header::IF_MATCH, etag(version));
        }
        self.send_json(request.json(body)).await
    }

    pub(crate) async fn delete_with(
        &self,
        segments: &[&str],
        version: Option<i64>,
        query: &[(&str, String)],
        mutation: &Mutation,
    ) -> Result<()> {
        let mut url = self.versioned_url(segments)?;
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (name, value) in query {
                pairs.append_pair(name, value);
            }
        }
        let mut request = mutation.apply(self.prepare(Method::DELETE, url));
        if let Some(version) = version {
            request = request.header(reqwest::header::IF_MATCH, etag(version));
        }
        self.send_empty(request).await
    }

    pub(crate) async fn delete(
        &self,
        segments: &[&str],
        version: Option<i64>,
        mutation: &Mutation,
    ) -> Result<()> {
        let mut request = mutation.apply(self.route(Method::DELETE, segments)?);
        if let Some(version) = version {
            request = request.header(reqwest::header::IF_MATCH, etag(version));
        }
        self.send_empty(request).await
    }

    pub(crate) async fn send_json<R: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<R> {
        async {
            let body = self.send(request).await?;
            if body.is_empty() {
                return Err(Error::Decode(
                    "the service returned an empty body where a value was expected".to_owned(),
                ));
            }
            serde_json::from_slice(&body)
                .map_err(|error| Error::Decode(format!("unexpected response shape: {error}")))
        }
        .await
    }

    pub(crate) async fn send_empty(&self, request: reqwest::RequestBuilder) -> Result<()> {
        self.send(request).await.map(|_| ())
    }

    pub(crate) async fn send_negotiation(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<models::ApiVersionNegotiation> {
        async {
            let (headers, body) = self.send_response(request).await?;
            if body.is_empty() {
                return Err(Error::Decode(
                    "the version negotiation returned an empty body".to_owned(),
                ));
            }
            let negotiated = serde_json::from_slice::<models::ApiVersionNegotiation>(&body)
                .map_err(|error| {
                    Error::Decode(format!(
                        "unexpected version-negotiation response shape: {error}"
                    ))
                })?;
            validate_negotiation(&headers, &negotiated)?;
            Ok(negotiated)
        }
        .await
    }

    /// Sends one request and turns anything other than success into an error.
    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Vec<u8>> {
        self.send_response(request).await.map(|(_, body)| body)
    }

    async fn send_response(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<(HeaderMap, Vec<u8>)> {
        let request = request.build().map_err(Error::Transport)?;
        let started = Instant::now();
        let method = request.method().to_string();
        let route = crate::telemetry::route(request.url().path());
        let request_id = request
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let result = self.send_response_inner(request).await;
        if let Some(telemetry) = &self.telemetry {
            let (status, code) = match &result {
                Ok((_, _, status)) => (*status, "success"),
                Err(Error::Api(error)) => (error.status, error.code.as_str()),
                Err(Error::RateLimited { source, .. }) => (429, source.code.as_str()),
                Err(Error::Transport(_)) => (0, "transport_error"),
                Err(_) => (0, "response_error"),
            };
            telemetry.record("request", "request.completed", serde_json::json!({"request_id":request_id, "route":route, "method":method, "status":status, "code":code, "success":result.is_ok(), "testing":self.environment.is_some(), "duration_ms":u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)}));
        }
        result.map(|(headers, body, _)| (headers, body))
    }

    async fn send_response_inner(
        &self,
        request: reqwest::Request,
    ) -> Result<(HeaderMap, Vec<u8>, u16)> {
        let mut response = self.http.execute(request).await.map_err(Error::Transport)?;
        let status = response.status();
        let retry_after = header_seconds(&response, "retry-after");
        let limit = header_u64(&response, "ratelimit-limit");
        let remaining = header_u64(&response, "ratelimit-remaining");
        let headers = response.headers().clone();
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BODY_BYTES as u64)
        {
            return Err(Error::ResponseTooLarge {
                limit: MAX_RESPONSE_BODY_BYTES,
            });
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(Error::Transport)? {
            if body
                .len()
                .checked_add(chunk.len())
                .is_none_or(|length| length > MAX_RESPONSE_BODY_BYTES)
            {
                return Err(Error::ResponseTooLarge {
                    limit: MAX_RESPONSE_BODY_BYTES,
                });
            }
            body.extend_from_slice(&chunk);
        }

        if status.is_success() {
            return Ok((headers, body, status.as_u16()));
        }
        if status.is_redirection() {
            return Err(Error::Decode(format!(
                "the service returned an unexpected redirect ({status}); redirects are not followed"
            )));
        }

        let api = decode_envelope(status, &headers, &body)?;
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::RateLimited {
                // A 429 without Retry-After should not become a busy loop.
                retry_after: retry_after.unwrap_or(Duration::from_secs(1)),
                limit,
                remaining,
                source: Box::new(api),
            });
        }
        if status == StatusCode::NOT_ACCEPTABLE && api.code == "api_version_not_acceptable" {
            return Err(Error::ApiVersionUnsupported {
                offered: offered_versions(api.details.as_ref()),
            });
        }
        Err(Error::Api(Box::new(api)))
    }
}

fn validate_negotiation(
    headers: &HeaderMap,
    negotiated: &models::ApiVersionNegotiation,
) -> Result<()> {
    let mut selected_values = headers.get_all(SELECTED_VERSION_HEADER).iter();
    let selected_header = selected_values
        .next()
        .ok_or_else(|| {
            Error::Decode(format!(
                "version negotiation omitted the {SELECTED_VERSION_HEADER} response header"
            ))
        })?
        .to_str()
        .map_err(|_| {
            Error::Decode(format!(
                "version negotiation returned a non-text {SELECTED_VERSION_HEADER} header"
            ))
        })?;
    if selected_values.next().is_some() {
        return Err(Error::Decode(format!(
            "version negotiation returned {SELECTED_VERSION_HEADER} more than once"
        )));
    }
    if selected_header != API_VERSION || negotiated.selected_api_version != selected_header {
        return Err(Error::Decode(format!(
            "version negotiation disagreed: this client offered {API_VERSION}, the response header selected {selected_header}, and the body selected {}",
            negotiated.selected_api_version
        )));
    }
    if negotiated.service.as_str() != Some("silicon-iam") {
        return Err(Error::Decode(
            "version negotiation identified an unexpected service".to_owned(),
        ));
    }
    if negotiated.supported_api_versions.is_empty()
        || negotiated.supported_api_versions.len() > 16
        || negotiated
            .supported_api_versions
            .iter()
            .any(|version| !is_valid_api_version(version))
        || negotiated
            .supported_api_versions
            .iter()
            .enumerate()
            .any(|(index, version)| negotiated.supported_api_versions[..index].contains(version))
        || negotiated
            .supported_api_versions
            .windows(2)
            .any(|pair| !api_version_descends(&pair[0], &pair[1]))
        || !negotiated
            .supported_api_versions
            .iter()
            .any(|version| version == selected_header)
    {
        return Err(Error::Decode(
            "version negotiation returned an invalid or inconsistent supported-version catalog"
                .to_owned(),
        ));
    }
    let vary_names_supported_versions = headers
        .get_all(reqwest::header::VARY)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|name| name.trim().eq_ignore_ascii_case(SUPPORTED_VERSIONS_HEADER));
    if !vary_names_supported_versions {
        return Err(Error::Decode(format!(
            "version negotiation did not vary on {SUPPORTED_VERSIONS_HEADER}"
        )));
    }
    Ok(())
}

fn is_valid_api_version(version: &str) -> bool {
    version.strip_prefix('v').is_some_and(|major| {
        !major.is_empty()
            && major.len() <= 9
            && !major.starts_with('0')
            && major.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn api_version_descends(left: &str, right: &str) -> bool {
    let left = left
        .strip_prefix('v')
        .and_then(|major| major.parse::<u32>().ok());
    let right = right
        .strip_prefix('v')
        .and_then(|major| major.parse::<u32>().ok());
    matches!((left, right), (Some(left), Some(right)) if left > right)
}

impl ClientBuilder {
    /// Starts from a service base URL.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invalid`] when the URL cannot be parsed or does not
    /// use an HTTP scheme.
    pub fn new(base_url: &str) -> Result<Self> {
        let mut base_url = Url::parse(base_url)
            .map_err(|error| Error::Invalid(format!("invalid base URL: {error}")))?;
        if !matches!(base_url.scheme(), "http" | "https") {
            return Err(Error::Invalid(
                "the base URL must be http or https".to_owned(),
            ));
        }
        if base_url.host_str().is_none()
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.port() == Some(0)
            || base_url.query().is_some()
            || base_url.fragment().is_some()
        {
            return Err(Error::Invalid(
                "the base URL must contain a host and no credentials, zero port, query, or fragment"
                    .to_owned(),
            ));
        }
        let is_loopback = match base_url.host() {
            Some(url::Host::Domain("localhost")) => true,
            Some(url::Host::Ipv4(address)) => address.is_loopback(),
            Some(url::Host::Ipv6(address)) => address.is_loopback(),
            _ => false,
        };
        if base_url.scheme() == "http" && !is_loopback {
            return Err(Error::Invalid(
                "the base URL must use HTTPS; HTTP is limited to localhost or a loopback IP address"
                    .to_owned(),
            ));
        }
        // Every route is joined onto this, and `Url::join` discards the last
        // path segment unless the base ends in a slash.
        if !base_url.path().ends_with('/') {
            let path = format!("{}/", base_url.path());
            base_url.set_path(&path);
        }
        Ok(Self {
            base_url,
            credential: Credential::Anonymous,
            environment: None,
            testing_application: None,
            timeout: Duration::from_secs(30),
            user_agent: None,
            telemetry_enabled: true,
        })
    }

    /// Sets the credential every request will present.
    #[must_use]
    pub fn credential(mut self, credential: Credential) -> Self {
        self.credential = credential;
        self
    }

    /// Runs every request inside a testing environment.
    #[must_use]
    pub fn environment(mut self, environment: EnvironmentKey) -> Self {
        self.environment = Some(environment);
        self
    }

    /// Overrides the per-request timeout. Defaults to 30 seconds.
    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Appends a caller identity to the `User-Agent`.
    ///
    /// Worth setting: it is what makes one integration distinguishable from
    /// another in the service's request logs.
    #[must_use]
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    /// Compatibility setting. Runtime dependency updates are always disabled.
    #[must_use]
    pub const fn auto_update(self, _enabled: bool) -> Self {
        self
    }

    /// Opt out of client telemetry and propagate the preference to the IAM backend.
    #[must_use]
    pub fn telemetry(mut self, enabled: bool) -> Self {
        self.telemetry_enabled = enabled;
        self
    }

    /// Compatibility setting. The client never reads or changes this manifest.
    #[must_use]
    pub fn update_manifest(self, _path: impl Into<PathBuf>) -> Self {
        self
    }

    /// Builds the client.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Transport`] when the HTTP stack cannot be built, which
    /// in practice means the TLS backend failed to initialize.
    pub fn build(self) -> Result<Client> {
        let default_agent = concat!("silicon-iam-client/", env!("CARGO_PKG_VERSION"));
        let user_agent = match self.user_agent {
            Some(caller) => format!("{default_agent} {caller}"),
            None => default_agent.to_owned(),
        };
        let http = reqwest::Client::builder()
            .timeout(self.timeout)
            .user_agent(user_agent)
            // An IAM response is never a navigation. Refusing redirects keeps
            // bearer, Basic, environment, and step-up authority on the exact
            // origin the caller configured.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(Error::Transport)?;
        let telemetry_enabled = crate::telemetry::enabled(self.telemetry_enabled);
        let telemetry = crate::telemetry::Telemetry::from_env("rust-client", telemetry_enabled)
            .ok()
            .flatten();
        Ok(Client {
            http,
            telemetry,
            telemetry_enabled,
            base_url: self.base_url,
            credential: self.credential,
            environment: self.environment,
            testing_application: self.testing_application,
        })
    }
}

/// Recovers the service's envelope without inventing IAM errors for a proxy.
///
/// Every documented failure carries the envelope, but a proxy or a crash can
/// still put something else on the wire. Keep that failure distinct from an
/// IAM authorization decision and never display or retain its raw body.
fn decode_envelope(status: StatusCode, headers: &HeaderMap, body: &[u8]) -> Result<ApiError> {
    let request_id = headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .map(|value| value.to_string());
    match serde_json::from_slice::<Envelope>(body) {
        Ok(envelope) => Ok(ApiError {
            status: status.as_u16(),
            code: envelope.error.code,
            message: envelope.error.message,
            details: envelope.error.details,
            request_id: envelope.error.request_id.or(request_id),
        }),
        Err(_) => Err(Error::UnstructuredResponse {
            status: status.as_u16(),
            request_id,
        }),
    }
}

fn offered_versions(details: Option<&serde_json::Value>) -> Vec<String> {
    details
        .and_then(|details| details.get("supported_versions"))
        .and_then(serde_json::Value::as_array)
        .map(|versions| {
            versions
                .iter()
                .filter_map(|version| version.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// The strong entity tag form the contract expects in `If-Match`.
fn etag(version: i64) -> String {
    format!("\"{version}\"")
}

fn header_u64(response: &reqwest::Response, name: &str) -> Option<u64> {
    response
        .headers()
        .get(name)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn header_seconds(response: &reqwest::Response, name: &str) -> Option<Duration> {
    header_u64(response, name).map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use reqwest::StatusCode;

    use crate::{Credential, EnvironmentKey};

    use super::{Client, decode_envelope, offered_versions};
    use crate::update::UpdateStatus;

    #[test]
    fn runtime_updates_cannot_be_enabled() -> crate::Result<()> {
        let client = Client::builder("https://example.test")?
            .auto_update(true)
            .build()?;
        assert_eq!(client.update_status(), UpdateStatus::Disabled);
        Ok(())
    }

    #[test]
    fn a_base_url_without_a_trailing_slash_still_joins_correctly() {
        let Ok(client) = Client::new("https://backend.iam.teamofsilicons.com") else {
            panic!("a plain https URL must be accepted");
        };
        let Ok(request) = client.route(reqwest::Method::GET, &["me"]) else {
            panic!("the route must join onto the base");
        };
        let Ok(built) = request.build() else {
            panic!("the request must build");
        };
        assert_eq!(
            built.url().as_str(),
            "https://backend.iam.teamofsilicons.com/api/v1/me"
        );
    }

    #[test]
    fn a_base_url_with_a_path_prefix_keeps_it() {
        let Ok(client) = Client::new("https://example.test/iam") else {
            panic!("a prefixed URL must be accepted");
        };
        let Ok(request) = client.route(reqwest::Method::GET, &["me"]) else {
            panic!("the route must join onto the base");
        };
        let Ok(built) = request.build() else {
            panic!("the request must build");
        };
        assert_eq!(built.url().as_str(), "https://example.test/iam/api/v1/me");
    }

    #[test]
    fn a_path_parameter_cannot_change_which_route_is_called() {
        let Ok(client) = Client::new("https://example.test") else {
            panic!("a plain https URL must be accepted");
        };
        let Ok(request) = client.route(
            reqwest::Method::GET,
            &["organizations", "../../admin/applications", "tags"],
        ) else {
            panic!("the route must build");
        };
        let Ok(built) = request.build() else {
            panic!("the request must build");
        };
        // The hostile segment is escaped into one segment rather than walking
        // up the path.
        assert!(!built.url().path().contains("/admin/"), "{}", built.url());
        assert!(built.url().path().starts_with("/api/v1/organizations/"));
    }

    #[test]
    fn a_non_http_base_url_is_refused() {
        assert!(Client::new("ftp://example.test").is_err());
        assert!(Client::new("not a url").is_err());
    }

    #[test]
    fn telemetry_opt_out_is_propagated_without_a_recorder() -> crate::Result<()> {
        let client = Client::builder("https://example.test")?
            .telemetry(false)
            .build()?;
        assert!(client.telemetry.is_none());
        let request = client
            .route(reqwest::Method::GET, &["me"])?
            .build()
            .map_err(crate::Error::Transport)?;
        assert_eq!(
            request
                .headers()
                .get("x-iam-telemetry")
                .and_then(|v| v.to_str().ok()),
            Some("off")
        );
        assert!(
            request
                .headers()
                .get("x-request-id")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
        );
        Ok(())
    }

    #[test]
    fn test_selection_and_application_auth_are_independent_headers() {
        let Ok(environment) = EnvironmentKey::new("a".repeat(32)) else {
            panic!("the fixed-size test key must be accepted");
        };
        let Ok(client) = Client::builder("https://example.test").and_then(|builder| {
            builder
                .credential(Credential::application("caller", "ask_secret"))
                .environment(environment)
                .build()
        }) else {
            panic!("a valid client must build");
        };
        let Ok(request) = client.route(
            reqwest::Method::GET,
            &["application-directory", "other>target"],
        ) else {
            panic!("the discovery request must build");
        };
        let Ok(built) = request.build() else {
            panic!("the request must finalize");
        };

        assert_eq!(
            built
                .headers()
                .get("x-testing-environment-key")
                .and_then(|value| value.to_str().ok()),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert!(
            built
                .headers()
                .get(reqwest::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with("Basic "))
        );
        assert_eq!(
            built.url().as_str(),
            "https://example.test/api/v1/application-directory/other%3Etarget"
        );
    }

    #[test]
    fn an_unparsable_error_body_still_reports_the_status() {
        let error = decode_envelope(
            StatusCode::BAD_GATEWAY,
            &reqwest::header::HeaderMap::new(),
            b"<html>upstream died</html>",
        );
        assert!(matches!(
            error,
            Err(crate::Error::UnstructuredResponse { status: 502, .. })
        ));
    }

    #[test]
    fn the_envelope_is_preferred_when_present() {
        let body = br#"{"error":{"code":"etag_mismatch","message":"stale","request_id":"r1"}}"#;
        let Ok(error) = decode_envelope(
            StatusCode::PRECONDITION_FAILED,
            &reqwest::header::HeaderMap::new(),
            body,
        ) else {
            panic!("a structured IAM envelope must be recognized");
        };
        assert!(error.is_version_conflict());
        assert_eq!(error.request_id.as_deref(), Some("r1"));
    }

    #[test]
    fn offered_versions_survive_a_missing_or_odd_detail_shape() {
        assert!(offered_versions(None).is_empty());
        assert!(offered_versions(Some(&serde_json::json!({}))).is_empty());
        assert_eq!(
            offered_versions(Some(&serde_json::json!({"supported_versions":["v2","v3"]}))),
            vec!["v2".to_owned(), "v3".to_owned()]
        );
    }
}
