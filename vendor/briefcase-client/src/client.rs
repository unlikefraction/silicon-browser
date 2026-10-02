//! The client itself: one deployment, one organization, no stored state.

use std::time::Duration;

use reqwest::{Method, RequestBuilder, Response, StatusCode};
use secrecy::ExposeSecret as _;
use serde::{Serialize, de::DeserializeOwned};
use url::Url;
use uuid::Uuid;

use crate::{
    config::{Config, Credential},
    contract::{API_VERSION, ServiceVersion},
    error::{ApiError, Error, Result, WireErrorEnvelope, transport},
    models::ServiceStatus,
    update::UpdateStatus,
};

/// A key that makes a retried mutation return the first answer again.
///
/// Briefcase requires one on creations, session exchange, testing-environment
/// mutations, and upload finalization. The client generates one per call unless
/// a caller supplies its own, which is what a caller wants when the retry
/// happens in their own process rather than here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Builds a key from a caller-owned value.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Configuration`] unless the value is 8 to 255 visible
    /// ASCII bytes, matching the service contract.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if !(8..=255).contains(&value.len()) || !value.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(Error::Configuration(
                "an idempotency key must be 8 to 255 visible ASCII bytes".into(),
            ));
        }
        Ok(Self(value))
    }

    /// Generates a fresh key.
    #[must_use]
    pub fn random() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Returns the key as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for IdempotencyKey {
    fn default() -> Self {
        Self::random()
    }
}

/// A connected Briefcase client.
///
/// Cloning is cheap and shares one connection pool, so a client is meant to be
/// built once and passed around.
#[derive(Clone, Debug)]
pub struct Client {
    http: reqwest::Client,
    config: Config,
}

impl Client {
    /// Connects, and refuses a deployment this build does not match.
    ///
    /// Reads `GET /api/version` first and compares every operation's revision
    /// against the one this build was written against, so an incompatible
    /// pairing fails here rather than in the middle of a later call.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Incompatible`] when the deployment serves a different
    /// contract, and a transport error when it cannot be reached.
    pub async fn connect(config: Config) -> Result<Self> {
        let client = Self::new_unchecked(config)?;
        client.version().await?.check_compatibility()?;
        Ok(client)
    }

    /// Builds a client without checking the contract.
    ///
    /// Use this when the caller has already checked compatibility, or during a
    /// deliberate rollout where the mismatch is known and accepted.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Configuration`] when the HTTP client cannot be built.
    pub fn new_unchecked(config: Config) -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            // A redirect must never move a bearer, refresh token, test root,
            // or idempotency-bound body outside the configured deployment.
            .redirect(reqwest::redirect::Policy::none())
            // OBO proofs and staging capabilities are single-attempt calls;
            // even implicit protocol retries must remain caller-controlled.
            .retry(reqwest::retry::never())
            .user_agent(config.user_agent.clone())
            .build()
            .map_err(|error| {
                Error::Configuration(format!("HTTP client could not be built: {error}"))
            })?;
        Ok(Self { http, config })
    }

    /// Returns the configuration this client was built with.
    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// Runtime dependency updates are retired; always returns `Disabled`.
    #[must_use]
    pub fn update_status(&self) -> UpdateStatus {
        UpdateStatus::Disabled
    }

    /// Returns the selected organization, or an empty string for sign-in-only clients.
    #[must_use]
    pub fn organization(&self) -> &str {
        &self.config.organization
    }

    /// Reads what the deployment serves, without authenticating.
    ///
    /// # Errors
    ///
    /// Returns an error when the deployment cannot be reached, or answers
    /// `406` because it serves no API major this build speaks.
    pub async fn version(&self) -> Result<ServiceVersion> {
        let url = self.origin_url(&["api", "version"])?;
        let request = self
            .http
            .get(url)
            .header("briefcase-supported-api-versions", API_VERSION)
            .timeout(self.config.request_timeout);
        let request = self.apply_environment(request);
        let response = self.receive(request).await?;
        let selected_header = response
            .headers()
            .get("briefcase-api-version")
            .ok_or_else(|| {
                Error::Protocol(
                    "the version response omitted the Briefcase-API-Version header".into(),
                )
            })?
            .to_str()
            .map_err(|_| {
                Error::Protocol(
                    "the version response carried an invalid Briefcase-API-Version header".into(),
                )
            })?
            .to_owned();
        let body = response.bytes().await.map_err(transport)?;
        let version: ServiceVersion = serde_json::from_slice(&body).map_err(|error| {
            Error::Protocol(format!(
                "the version response did not match the contract: {error}"
            ))
        })?;
        if selected_header != version.selected_api_version {
            return Err(Error::Protocol(format!(
                "the version response selected {} in its header but {} in its body",
                selected_header, version.selected_api_version
            )));
        }
        Ok(version)
    }

    /// Reports whether the deployment is alive.
    ///
    /// # Errors
    ///
    /// Returns an error when the deployment cannot be reached.
    pub async fn health(&self) -> Result<ServiceStatus> {
        let url = self.origin_url(&["healthz"])?;
        let request = self.http.get(url).timeout(self.config.request_timeout);
        self.receive_json(self.apply_environment(request)).await
    }

    /// Reports whether the deployment can serve requests.
    ///
    /// # Errors
    ///
    /// Returns an error when the deployment cannot be reached, or answers that
    /// a dependency of its own is unavailable.
    pub async fn ready(&self) -> Result<ServiceStatus> {
        let url = self.origin_url(&["readyz"])?;
        let request = self.http.get(url).timeout(self.config.request_timeout);
        self.receive_json(self.apply_environment(request)).await
    }

    // ---- request plumbing ------------------------------------------------

    /// Builds a URL under the versioned API base from already-safe segments.
    pub(crate) fn api_url(&self, segments: &[&str]) -> Result<Url> {
        if self.config.organization.is_empty()
            && !matches!(
                segments,
                ["iam" | "testing-environment" | "telemetry"]
                    | ["auth", "slt" | "refresh" | "status"]
                    | ["public", _, ..]
                    | ["testing-environment", "cleanings"]
            )
        {
            return Err(Error::Configuration(
                "choose an authorised organisation before using workspace APIs".into(),
            ));
        }
        let mut url = self.config.api_base.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|()| Error::Configuration("base URL cannot have a path".into()))?;
            // The trailing empty segment of ".../api/v1/" would otherwise
            // survive and produce a double slash.
            path.pop_if_empty();
            for segment in segments {
                path.push(segment);
            }
        }
        Ok(url)
    }

    /// Builds a URL at the deployment origin, outside the versioned base.
    pub(crate) fn origin_url(&self, segments: &[&str]) -> Result<Url> {
        let mut url = self.config.origin.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|()| Error::Configuration("origin URL is invalid".into()))?;
            path.pop_if_empty();
            for segment in segments {
                path.push(segment);
            }
        }
        Ok(url)
    }

    /// Starts an authenticated request scoped to this client's organization.
    pub(crate) fn request(&self, method: Method, url: Url) -> RequestBuilder {
        let builder = self
            .http
            .request(method, url)
            .header("x-org-id", &self.config.organization);
        let builder = self.apply_environment(builder);
        match &self.config.credential {
            Credential::Bearer(token) => builder.bearer_auth(token.expose_secret()),
            Credential::None => builder,
        }
    }

    /// Adds only the testing-plane selector, never an IAM bearer credential.
    pub(crate) fn anonymous_request(&self, method: Method, url: Url) -> RequestBuilder {
        self.apply_environment(self.http.request(method, url))
    }

    /// Adds the testing-plane selector to a manually constructed request.
    pub(crate) fn apply_environment(&self, mut request: RequestBuilder) -> RequestBuilder {
        request = request
            .header(
                "x-briefcase-telemetry",
                if self.config.telemetry { "on" } else { "off" },
            )
            .header(
                "x-briefcase-source",
                match self.config.telemetry_source {
                    crate::telemetry::Source::Cli => "cli",
                    crate::telemetry::Source::Daemon => "daemon",
                    crate::telemetry::Source::Web => "web",
                    _ => "sdk",
                },
            );
        if let Some(environment) = self.config.environment() {
            request = request.header("x-briefcase-app-secret", environment.expose());
        }
        request
    }

    pub(crate) const fn request_timeout(&self) -> Duration {
        self.config.request_timeout
    }

    pub(crate) const fn transfer_timeout(&self) -> Duration {
        self.config.transfer_timeout
    }

    /// Sends a request and reads a JSON answer.
    pub(crate) async fn receive_json<T>(&self, request: RequestBuilder) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let response = self.receive(request).await?;
        Self::decode_json(response).await
    }

    async fn decode_json<T>(response: Response) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let body = response.bytes().await.map_err(transport)?;
        serde_json::from_slice(&body).map_err(|error| {
            Error::Protocol(format!("the response did not match the contract: {error}"))
        })
    }

    /// Sends a request that answers with no content.
    pub(crate) async fn receive_empty(&self, request: RequestBuilder) -> Result<()> {
        self.receive(request).await.map(drop)
    }

    /// Sends a request and returns the raw successful response.
    pub(crate) async fn receive(&self, request: RequestBuilder) -> Result<Response> {
        let response = request.send().await.map_err(transport)?;
        if response.status().is_success() {
            return Ok(response);
        }
        Err(Error::Api(api_error(response).await))
    }
}

/// Serializes a JSON body, keeping the failure local rather than panicking.
pub(crate) fn json_body<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value)
        .map_err(|error| Error::Configuration(format!("request could not be encoded: {error}")))
}

async fn api_error(response: Response) -> ApiError {
    let status = response.status();
    let unsatisfied_range_length = (status == StatusCode::RANGE_NOT_SATISFIABLE)
        .then(|| {
            response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)?
                .to_str()
                .ok()?
                .strip_prefix("bytes */")
                .filter(|length| {
                    !length.is_empty() && length.bytes().all(|byte| byte.is_ascii_digit())
                })?
                .parse::<u64>()
                .ok()
        })
        .flatten();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let body = response.bytes().await.unwrap_or_default();

    serde_json::from_slice::<WireErrorEnvelope>(&body).map_or_else(
        |_| ApiError {
            status: status.as_u16(),
            code: fallback_code(status),
            message: status
                .canonical_reason()
                .unwrap_or("the request failed")
                .to_owned(),
            request_id: None,
            retry_after,
            unsatisfied_range_length,
        },
        |envelope| ApiError {
            status: status.as_u16(),
            code: envelope.error.code,
            message: envelope.error.message,
            request_id: envelope.error.request_id,
            retry_after,
            unsatisfied_range_length,
        },
    )
}

fn fallback_code(status: StatusCode) -> String {
    match status {
        StatusCode::UNAUTHORIZED => "unauthenticated",
        StatusCode::FORBIDDEN => "forbidden",
        StatusCode::NOT_FOUND => "not_found",
        StatusCode::NOT_ACCEPTABLE => "unsupported_api_version",
        StatusCode::CONFLICT => "conflict",
        StatusCode::TOO_MANY_REQUESTS => "rate_limited",
        _ if status.is_server_error() => "service_error",
        _ => "request_failed",
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::{Client, IdempotencyKey};
    use crate::config::Config;
    use crate::update::UpdateStatus;

    #[test]
    fn legacy_update_options_cannot_enable_runtime_updates() {
        for enabled in [false, true] {
            let config = Config::new("https://briefcase.example/api/v1/", "tos")
                .unwrap()
                .with_auto_update(enabled)
                .with_update_manifest("/missing/Cargo.toml");
            let client = Client::new_unchecked(config).unwrap();
            assert_eq!(client.update_status(), UpdateStatus::Disabled);
        }
    }

    fn client() -> Client {
        Client::new_unchecked(
            Config::new("https://briefcase.example/api/v1/", "tos")
                .unwrap()
                .with_token("token"),
        )
        .unwrap()
    }

    #[test]
    fn api_urls_sit_under_the_versioned_base_without_doubling_slashes() {
        let client = client();
        assert_eq!(
            client.api_url(&["entries"]).unwrap().as_str(),
            "https://briefcase.example/api/v1/entries"
        );
        assert_eq!(
            client
                .api_url(&["entries", "01a0", "content"])
                .unwrap()
                .as_str(),
            "https://briefcase.example/api/v1/entries/01a0/content"
        );
    }

    #[test]
    fn origin_urls_sit_outside_the_versioned_base() {
        let client = client();
        assert_eq!(
            client.origin_url(&["api", "version"]).unwrap().as_str(),
            "https://briefcase.example/api/version"
        );
    }

    #[test]
    fn a_path_segment_is_escaped_rather_than_interpreted() {
        // Entry names may contain anything but a slash, and an organization
        // identifier is opaque, so segments are never pasted into a URL.
        let client = client();
        assert_eq!(
            client
                .origin_url(&["org", "tos", "private/cos:tos"])
                .unwrap()
                .as_str(),
            "https://briefcase.example/org/tos/private%2Fcos:tos"
        );
    }

    #[test]
    fn idempotency_keys_are_bounded_and_unique() {
        assert!(IdempotencyKey::new("").is_err());
        assert!(IdempotencyKey::new("1234567").is_err());
        assert!(IdempotencyKey::new("contains space").is_err());
        assert!(IdempotencyKey::new("line\nbreak").is_err());
        assert!(IdempotencyKey::new("k".repeat(256)).is_err());
        assert_ne!(IdempotencyKey::random(), IdempotencyKey::random());
        assert_eq!(
            IdempotencyKey::new("mine-000").unwrap().as_str(),
            "mine-000"
        );
    }
}
