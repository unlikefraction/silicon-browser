//! An organization's SSO configuration.
//!
//! The browser-facing authorization and callback routes are absent on
//! purpose: they are redirects a browser follows, not calls an API client
//! makes.

use crate::{Client, Mutation, Result, models};

/// SSO configuration for one organization.
pub struct Sso<'a>(pub(super) &'a Client);

impl Sso<'_> {
    /// The organization's current SSO configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller lacks `sso.manage`.
    pub async fn get(&self, org_id: &str) -> Result<models::SsoConfiguration> {
        self.0.get(&["organizations", org_id, "sso"]).await
    }

    /// Creates a short-lived link to the provider's setup portal.
    ///
    /// # Errors
    ///
    /// Returns an error when SSO is not enabled for the organization, or the
    /// caller lacks `sso.manage`.
    pub async fn setup_link(
        &self,
        org_id: &str,
        mutation: &Mutation,
    ) -> Result<models::SsoSetupLink> {
        self.0
            .post(
                &["organizations", org_id, "sso", "setup-link"],
                &serde_json::json!({}),
                mutation,
            )
            .await
    }

    /// Checks the configuration end to end without changing anything.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller lacks `sso.manage`.
    pub async fn test(&self, org_id: &str, mutation: &Mutation) -> Result<models::TestResult> {
        self.0
            .post(
                &["organizations", org_id, "sso", "test"],
                &serde_json::json!({}),
                mutation,
            )
            .await
    }

    /// Disables SSO for the organization.
    ///
    /// Requires a step-up assertion. Members who joined through SSO keep their
    /// memberships; only the join method changes.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale or the step-up is missing.
    pub async fn disable(&self, org_id: &str, version: i64, mutation: &Mutation) -> Result<()> {
        self.0
            .delete(&["organizations", org_id, "sso"], Some(version), mutation)
            .await
    }
}
