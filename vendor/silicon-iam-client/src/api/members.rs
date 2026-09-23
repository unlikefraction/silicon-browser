//! Members of an organization, and the directory view of them.
//!
//! Two projections of the same people. The membership routes are the
//! management view, keyed by membership identifier and carrying version and
//! authorization state. The directory routes are the reading view an ordinary
//! member gets, with a field selector for how much of it to fetch.

use uuid::Uuid;

use crate::{Client, Mutation, Paging, Result, models};

/// Organization membership and directory.
pub struct Members<'a>(pub(super) &'a Client);

/// Narrows a member listing.
#[derive(Clone, Debug, Default)]
pub struct MemberFilter {
    /// Only Carbons, or only Silicons.
    pub principal_type: Option<String>,
    /// Only members carrying one tag.
    pub tag_id: Option<Uuid>,
    /// Only members in one lifecycle state.
    pub status: Option<String>,
}

impl MemberFilter {
    fn query(&self) -> Vec<(&'static str, String)> {
        let mut query = Vec::new();
        if let Some(principal_type) = &self.principal_type {
            query.push(("principal_type", principal_type.clone()));
        }
        if let Some(tag_id) = self.tag_id {
            query.push(("tag_id", tag_id.to_string()));
        }
        if let Some(status) = &self.status {
            query.push(("status", status.clone()));
        }
        query
    }
}

impl Members<'_> {
    /// Every visible active member with all permitted details and caller-relative trust.
    ///
    /// The dictionary is keyed by Carbon ID or full Silicon ID, without pagination.
    ///
    /// # Errors
    /// Returns an error if directory access or organization selection is missing.
    pub async fn details(
        &self,
        org_id: &str,
    ) -> Result<std::collections::BTreeMap<String, serde_json::Value>> {
        self.0
            .get(&["organizations", org_id, "directory", "details"])
            .await
    }

    /// The organization's members.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller is not a member of the organization.
    pub async fn list(
        &self,
        org_id: &str,
        filter: &MemberFilter,
        paging: &Paging,
    ) -> Result<models::MembershipPage> {
        let mut query = paging.query();
        query.extend(filter.query());
        self.0
            .get_with(&["organizations", org_id, "members"], &query)
            .await
    }

    /// One member.
    ///
    /// # Errors
    ///
    /// Returns an error when the membership does not exist here.
    pub async fn get(&self, org_id: &str, membership_id: &str) -> Result<models::Membership> {
        self.0
            .get(&["organizations", org_id, "members", membership_id])
            .await
    }

    /// Updates a member's directory metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale or the caller lacks
    /// `members.update_directory`.
    pub async fn update(
        &self,
        org_id: &str,
        membership_id: &str,
        version: i64,
        patch: &models::MembershipDirectoryPatch,
        mutation: &Mutation,
    ) -> Result<models::Membership> {
        self.0
            .patch(
                &["organizations", org_id, "members", membership_id],
                version,
                patch,
                mutation,
            )
            .await
    }

    /// Removes a member.
    ///
    /// `reassign_reports_to` is required when the member has Silicons
    /// reporting to them; the service refuses the removal otherwise rather
    /// than orphan a hierarchy.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale, the member is the sole owner,
    /// or reports would be orphaned.
    pub async fn remove(
        &self,
        org_id: &str,
        membership_id: &str,
        version: i64,
        reassign_reports_to: Option<&str>,
        mutation: &Mutation,
    ) -> Result<()> {
        let segments = ["organizations", org_id, "members", membership_id];
        let query = reassign_reports_to
            .map(|target| vec![("reassign_reports_to", target.to_string())])
            .unwrap_or_default();
        self.0
            .delete_with(&segments, Some(version), &query, mutation)
            .await
    }

    /// A member's effective role and capability grants.
    ///
    /// # Errors
    ///
    /// Returns an error when the membership does not exist here.
    pub async fn authorization(
        &self,
        org_id: &str,
        membership_id: &str,
    ) -> Result<models::MembershipAuthorization> {
        self.0
            .get(&[
                "organizations",
                org_id,
                "members",
                membership_id,
                "authorization",
            ])
            .await
    }

    /// Promotes a member to administrator. Requires a step-up assertion.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale, the step-up is missing, or the
    /// caller lacks `admins.create`.
    pub async fn promote_admin(
        &self,
        org_id: &str,
        membership_id: &str,
        version: i64,
        mutation: &Mutation,
    ) -> Result<models::MembershipAuthorization> {
        self.0
            .post_versioned(
                &[
                    "organizations",
                    org_id,
                    "members",
                    membership_id,
                    "admin-promotions",
                ],
                version,
                &serde_json::json!({}),
                mutation,
            )
            .await
    }

    /// Demotes an administrator. Requires a step-up assertion.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale, the step-up is missing, or the
    /// caller lacks `admins.manage`.
    pub async fn demote_admin(
        &self,
        org_id: &str,
        membership_id: &str,
        version: i64,
        mutation: &Mutation,
    ) -> Result<models::MembershipAuthorization> {
        self.0
            .post_versioned(
                &[
                    "organizations",
                    org_id,
                    "members",
                    membership_id,
                    "admin-demotions",
                ],
                version,
                &serde_json::json!({}),
                mutation,
            )
            .await
    }

    /// Replaces an administrator's capability grants wholesale.
    ///
    /// Requires a step-up assertion. Send the complete set: anything omitted
    /// is revoked.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale, the step-up is missing, or a
    /// capability is outside what the caller may delegate.
    pub async fn replace_capabilities(
        &self,
        org_id: &str,
        membership_id: &str,
        version: i64,
        capabilities: &models::OrganizationCapabilitiesReplace,
        mutation: &Mutation,
    ) -> Result<models::MembershipAuthorization> {
        self.0
            .put(
                &[
                    "organizations",
                    org_id,
                    "members",
                    membership_id,
                    "capabilities",
                ],
                version,
                capabilities,
                mutation,
            )
            .await
    }

    /// The caller's own directory entry in one organization.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller is not a member.
    pub async fn directory_self(
        &self,
        org_id: &str,
        fields: Option<&str>,
    ) -> Result<models::DirectoryMember> {
        self.0
            .get_with(
                &["organizations", org_id, "directory", "self"],
                &directory_query(fields),
            )
            .await
    }

    /// The organization directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller is not a member.
    pub async fn directory(
        &self,
        org_id: &str,
        fields: Option<&str>,
        paging: &Paging,
    ) -> Result<models::DirectoryPage> {
        let mut query = paging.query();
        query.extend(directory_query(fields));
        self.0
            .get_with(&["organizations", org_id, "directory", "members"], &query)
            .await
    }

    /// One directory entry.
    ///
    /// # Errors
    ///
    /// Returns an error when the membership does not exist here.
    pub async fn directory_member(
        &self,
        org_id: &str,
        membership_id: &str,
        fields: Option<&str>,
    ) -> Result<models::DirectoryMember> {
        self.0
            .get_with(
                &[
                    "organizations",
                    org_id,
                    "directory",
                    "members",
                    membership_id,
                ],
                &directory_query(fields),
            )
            .await
    }
}

/// The directory's field selector, sent only when asked for.
fn directory_query(fields: Option<&str>) -> Vec<(&'static str, String)> {
    fields
        .map(|fields| vec![("fields", fields.to_owned())])
        .unwrap_or_default()
}
