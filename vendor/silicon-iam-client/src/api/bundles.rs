//! Application bundles retain independent applications and credentials.

use crate::{Client, Mutation, Paging, Result, models};

/// Organization-administered application bundle configuration.
pub struct Bundles<'a>(pub(super) &'a Client);

impl Bundles<'_> {
    /// Whether bundle configuration is available to the signed-in Carbon in an organization.
    /// The response describes this operation's availability without disclosing organization policy.
    ///
    /// # Errors
    /// Fails for non-Carbon or delegated credentials, or when the caller is not an active member.
    pub async fn availability(
        &self,
        org_id: &str,
    ) -> Result<models::ApplicationBundleAvailability> {
        self.0
            .get(&["organizations", org_id, "application-bundle-availability"])
            .await
    }

    /// Lists bundles the signed-in Carbon may administer.
    ///
    /// # Errors
    /// Fails when the caller is not authenticated.
    pub async fn list(&self) -> Result<models::ApplicationBundleList> {
        self.list_page(&Paging::new()).await
    }

    /// Lists one page of bundles the signed-in Carbon may administer.
    ///
    /// # Errors
    /// Fails when the caller is not authenticated or pagination is invalid.
    pub async fn list_page(&self, paging: &Paging) -> Result<models::ApplicationBundleList> {
        self.list_filtered(None, paging).await
    }

    /// Lists bundles in one organization, filtering before pagination.
    ///
    /// # Errors
    /// Fails when the organization is unknown or unavailable to the caller, or pagination is invalid.
    pub async fn list_for_organization(
        &self,
        org_id: &str,
        paging: &Paging,
    ) -> Result<models::ApplicationBundleList> {
        self.list_filtered(Some(org_id), paging).await
    }

    async fn list_filtered(
        &self,
        org_id: Option<&str>,
        paging: &Paging,
    ) -> Result<models::ApplicationBundleList> {
        let mut query = paging.query();
        if let Some(org_id) = org_id {
            query.push(("org_id", org_id.to_owned()));
        }
        self.0.get_with(&["application-bundles"], &query).await
    }

    /// Creates a bundle whose members belong to the same organization.
    ///
    /// # Errors
    /// Fails if bundle creation is unavailable, the handle is taken, or a member is invalid.
    pub async fn create(
        &self,
        input: &models::ApplicationBundleCreate,
        mutation: &Mutation,
    ) -> Result<models::ApplicationBundle> {
        self.0.post(&["application-bundles"], input, mutation).await
    }

    /// Reads a bundle using its canonical organization-qualified identifier.
    ///
    /// # Errors
    /// Fails when the bundle is unavailable to this caller.
    pub async fn get(&self, bundle_id: &str) -> Result<models::ApplicationBundle> {
        self.0.get(&["application-bundles", bundle_id]).await
    }

    /// Updates display details or the complete member list.
    ///
    /// # Errors
    /// Fails if the version is stale or the caller cannot administer the bundle.
    pub async fn update(
        &self,
        bundle_id: &str,
        version: i64,
        input: &models::ApplicationBundlePatch,
        mutation: &Mutation,
    ) -> Result<models::ApplicationBundle> {
        self.0
            .patch(
                &["application-bundles", bundle_id],
                version,
                input,
                mutation,
            )
            .await
    }

    /// Deletes a bundle while leaving every member application intact.
    ///
    /// # Errors
    /// Fails if the version is stale or the caller cannot administer the bundle.
    pub async fn delete(&self, bundle_id: &str, version: i64, mutation: &Mutation) -> Result<()> {
        self.0
            .delete(&["application-bundles", bundle_id], Some(version), mutation)
            .await
    }
}
