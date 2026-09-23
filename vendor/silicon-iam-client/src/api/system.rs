//! Version and readiness of the service itself.

use reqwest::Method;

use crate::{Client, Result, models};

/// Service metadata. None of these routes need a credential.
pub struct System<'a>(pub(super) &'a Client);

impl System<'_> {
    /// Lists contract lifecycle states, compatibility metadata, and retirement policy.
    ///
    /// # Errors
    /// Fails when the service is unavailable or returns an invalid response.
    pub async fn contracts(&self) -> Result<serde_json::Value> {
        self.0.get(&["contracts"]).await
    }

    /// The service's build and API version.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails or the response is unreadable.
    pub async fn version(&self) -> Result<models::VersionInfo> {
        let version = self.0.get::<models::VersionInfo>(&["version"]).await?;
        if version.service.as_str() != Some("silicon-iam")
            || version.api_version.as_str() != Some(crate::API_VERSION)
        {
            return Err(crate::Error::Decode(
                "the version endpoint identified an unexpected service or API version".to_owned(),
            ));
        }
        Ok(version)
    }

    /// Agrees an API version with the service.
    ///
    /// This client advertises the one version it implements. A service that
    /// has moved past it answers [`Error::ApiVersionUnsupported`], which is the
    /// signal to upgrade this crate rather than to retry.
    ///
    /// [`Error::ApiVersionUnsupported`]: crate::Error::ApiVersionUnsupported
    ///
    /// # Errors
    ///
    /// Returns an error when no version is shared, or the request fails.
    pub async fn negotiate(&self) -> Result<models::ApiVersionNegotiation> {
        let request = self.0.unversioned(Method::GET, &["api", "version"])?;
        self.0.send_negotiation(request).await
    }

    /// Whether the process is alive. Never touches the database.
    ///
    /// # Errors
    ///
    /// Returns an error when the process does not answer.
    pub async fn liveness(&self) -> Result<()> {
        let request = self.0.unversioned(Method::GET, &["healthz"])?;
        self.0.send_empty(request).await
    }

    /// Whether the service is ready to serve, database included.
    ///
    /// # Errors
    ///
    /// Returns an error when the service reports itself unready.
    pub async fn readiness(&self) -> Result<()> {
        let request = self.0.unversioned(Method::GET, &["readyz"])?;
        self.0.send_empty(request).await
    }
}
