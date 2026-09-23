//! Organizations the caller belongs to, or is creating.

use crate::{Client, Mutation, Paging, Result, models};

/// Organization tenancy.
pub struct Organizations<'a>(pub(super) &'a Client);

impl Organizations<'_> {
    /// Whether an organization handle can still be claimed.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails.
    pub async fn handle_available(&self, org_id: &str) -> Result<models::Availability> {
        self.0
            .get(&["organization-ids", org_id, "availability"])
            .await
    }

    /// Organizations the caller is an active member of.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails.
    pub async fn list(&self, paging: &Paging) -> Result<models::OrganizationPage> {
        self.list_with_status(None, paging).await
    }

    /// Organizations the caller belongs to, optionally filtered by membership
    /// status (`active` or `removed`).
    ///
    /// # Errors
    ///
    /// Returns an error when the status is invalid or the request fails.
    pub async fn list_with_status(
        &self,
        status: Option<&str>,
        paging: &Paging,
    ) -> Result<models::OrganizationPage> {
        let mut query = paging.query();
        if let Some(status) = status {
            query.push(("status", status.to_owned()));
        }
        self.0.get_with(&["organizations"], &query).await
    }

    /// Creates an organization, with the caller as its owner.
    ///
    /// # Errors
    ///
    /// Returns an error when the handle is taken or a field is rejected.
    pub async fn create(
        &self,
        input: &models::OrganizationCreate,
        mutation: &Mutation,
    ) -> Result<models::Organization> {
        self.0.post(&["organizations"], input, mutation).await
    }

    /// One organization by handle.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller is not a member, which is answered as
    /// not-found rather than forbidden.
    pub async fn get(&self, org_id: &str) -> Result<models::Organization> {
        self.0.get(&["organizations", org_id]).await
    }

    /// Updates organization metadata. The handle itself is immutable.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale or the caller lacks
    /// `organization.update`.
    pub async fn update(
        &self,
        org_id: &str,
        version: i64,
        patch: &models::OrganizationPatch,
        mutation: &Mutation,
    ) -> Result<models::Organization> {
        self.0
            .patch(&["organizations", org_id], version, patch, mutation)
            .await
    }

    /// Hands ownership to another member.
    ///
    /// Requires a step-up assertion. The current owner becomes an admin.
    ///
    /// # Errors
    ///
    /// Returns an error when the target cannot own, `version` is stale, or the
    /// step-up is missing.
    pub async fn transfer_ownership(
        &self,
        org_id: &str,
        version: i64,
        transfer: &models::OwnershipTransfer,
        mutation: &Mutation,
    ) -> Result<models::Organization> {
        self.0
            .post_versioned(
                &["organizations", org_id, "ownership-transfers"],
                version,
                transfer,
                mutation,
            )
            .await
    }
}
