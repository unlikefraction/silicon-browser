//! Organization tags.

use uuid::Uuid;

use crate::{Client, Mutation, Paging, Result, models};

/// Tags: stable organization-scoped groupings that also grant Silicon access.
pub struct Tags<'a>(pub(super) &'a Client);

impl Tags<'_> {
    /// The organization's tags.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails.
    pub async fn list(&self, org_id: &str, paging: &Paging) -> Result<models::TagPage> {
        self.0
            .get_with(&["organizations", org_id, "tags"], &paging.query())
            .await
    }

    /// Creates a tag. Requires `tags.manage`.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is already used by a live tag.
    pub async fn create(
        &self,
        org_id: &str,
        input: &models::TagCreate,
        mutation: &Mutation,
    ) -> Result<models::Tag> {
        self.0
            .post(&["organizations", org_id, "tags"], input, mutation)
            .await
    }

    /// One tag.
    ///
    /// # Errors
    ///
    /// Returns an error when the tag does not exist or was deleted.
    pub async fn get(&self, org_id: &str, tag_id: Uuid) -> Result<models::Tag> {
        self.0
            .get(&["organizations", org_id, "tags", &tag_id.to_string()])
            .await
    }

    /// Renames a tag. The identifier is preserved, so references survive.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale or the new name is taken.
    pub async fn update(
        &self,
        org_id: &str,
        tag_id: Uuid,
        version: i64,
        patch: &models::TagPatch,
        mutation: &Mutation,
    ) -> Result<models::Tag> {
        self.0
            .patch(
                &["organizations", org_id, "tags", &tag_id.to_string()],
                version,
                patch,
                mutation,
            )
            .await
    }

    /// Deletes a tag, and everything it conferred.
    ///
    /// Atomic with its cascade: the tag leaves every member that held it,
    /// tag-scoped trust rules are archived, affected members' authorization
    /// epochs advance, and the name becomes reusable at once.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale or the caller lacks
    /// `tags.manage`.
    pub async fn delete(
        &self,
        org_id: &str,
        tag_id: Uuid,
        version: i64,
        mutation: &Mutation,
    ) -> Result<()> {
        self.0
            .delete(
                &["organizations", org_id, "tags", &tag_id.to_string()],
                Some(version),
                mutation,
            )
            .await
    }

    /// The Carbons and Silicons carrying a tag.
    ///
    /// # Errors
    ///
    /// Returns an error when the tag does not exist or was deleted.
    pub async fn members(
        &self,
        org_id: &str,
        tag_id: Uuid,
        paging: &Paging,
    ) -> Result<models::MembershipPage> {
        self.0
            .get_with(
                &[
                    "organizations",
                    org_id,
                    "tags",
                    &tag_id.to_string(),
                    "members",
                ],
                &paging.query(),
            )
            .await
    }
}
