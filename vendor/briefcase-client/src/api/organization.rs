//! Organization-wide figures and storage.

use reqwest::Method;

use crate::{
    IdempotencyKey,
    client::{Client, json_body},
    error::Result,
    models::{BucketConfiguration, BucketConfigurationStatus, OrganizationUsage},
};

impl Client {
    /// Reports what the organization is consuming, in exact bytes.
    ///
    /// Storage counts every retained version, binned entries included, because
    /// those bytes are still stored; the space returns when the bin is purged
    /// rather than when an entry is binned.
    ///
    /// # Errors
    ///
    /// Returns an error when the deployment cannot be reached.
    pub async fn usage(&self) -> Result<OrganizationUsage> {
        let url = self.api_url(&["usage"])?;
        let request = self
            .request(Method::GET, url)
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }

    /// Points the organization's files at a bucket it owns.
    ///
    /// Briefcase assumes the role and performs a create, read, update, and
    /// delete probe; the bucket becomes active only when every check passes,
    /// and a failed probe leaves the previous configuration in place.
    ///
    /// # Errors
    ///
    /// Returns a forbidden error unless the caller is an organization owner or
    /// an authorized administrator.
    pub async fn configure_storage(
        &self,
        configuration: &BucketConfiguration,
    ) -> Result<BucketConfigurationStatus> {
        self.configure_storage_with_key(configuration, &IdempotencyKey::random())
            .await
    }

    /// Configures storage with a caller-owned retry identity.
    ///
    /// Retain this key together with the exact configuration before sending.
    /// Reuse both after a lost response to recover the same validation outcome.
    /// To retry a completed failed probe after fixing the external bucket or
    /// role policy, use a new key: the old key replays that failed result.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Client::configure_storage`], or a conflict
    /// when the key was previously used for a different configuration.
    pub async fn configure_storage_with_key(
        &self,
        configuration: &BucketConfiguration,
        idempotency_key: &IdempotencyKey,
    ) -> Result<BucketConfigurationStatus> {
        let url = self.api_url(&["storage", "configuration"])?;
        let body = json_body(configuration)?;
        let request = self
            .request(Method::PUT, url)
            .header("content-type", "application/json")
            .header("idempotency-key", idempotency_key.as_str())
            .body(body)
            .timeout(self.transfer_timeout());
        self.receive_json(request).await
    }
}
