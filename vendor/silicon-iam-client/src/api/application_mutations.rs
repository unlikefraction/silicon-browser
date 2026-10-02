//! IAM mutations made with a registered application's user access token.
//!
//! A write permission never implies permission to read existing resource data.
//! Results preserve the server's sparse receipts and independently authorized
//! fields as JSON objects; omitted values remain omitted, including nested
//! Silicon details. Direct IAM integrations can continue using the typed
//! `organizations()`, `members()`, `tags()`, and other first-party methods.
//!
//! Empty-response deletes, invitation verification-code delivery, completed
//! credential rotation, and SSO operations keep their existing typed methods.
//! Reuse the same [`Mutation`] when retrying an operation.

use crate::{Client, Mutation, Result, models};
use uuid::Uuid;

/// Scope-projected mutation results for an application's consented IAM access.
pub struct ApplicationMutations<'a>(pub(super) &'a Client);

impl ApplicationMutations<'_> {
    /// Creates an organization and preserves its scoped receipt.
    ///
    /// Requires `organizations.create` plus the represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn create_organization(
        &self,
        input: &models::OrganizationCreate,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
        self.0.post(&["organizations"], input, mutation).await
    }

    /// Updates organization metadata without requiring read access.
    ///
    /// Requires `organization.profile.update` for profile fields and
    /// `organization.sso.manage` for join-method changes, plus the represented
    /// user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn update_organization(
        &self,
        org_id: &str,
        version: i64,
        patch: &models::OrganizationPatch,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
        self.0
            .patch(&["organizations", org_id], version, patch, mutation)
            .await
    }

    /// Updates only the member fields covered by the application permissions.
    ///
    /// Requires each changed field's scope: `organization.silicon_access.update`,
    /// `organization.trust.update`, or `organization.silicons.update`, plus the
    /// represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn update_member(
        &self,
        org_id: &str,
        membership_id: &str,
        version: i64,
        patch: &models::MembershipDirectoryPatch,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
        self.0
            .patch(
                &["organizations", org_id, "members", membership_id],
                version,
                patch,
                mutation,
            )
            .await
    }

    /// Promotes an eligible Carbon and returns permitted authorization fields.
    ///
    /// Requires `organization.admins.promote` plus the represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn promote_admin(
        &self,
        org_id: &str,
        membership_id: &str,
        version: i64,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
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

    /// Demotes an administrator and returns permitted authorization fields.
    ///
    /// Requires `organization.admins.demote` plus the represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn demote_admin(
        &self,
        org_id: &str,
        membership_id: &str,
        version: i64,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
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

    /// Replaces explicit capabilities and preserves omitted authorization fields.
    ///
    /// Requires `organization.capabilities.update` plus the represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn replace_capabilities(
        &self,
        org_id: &str,
        membership_id: &str,
        version: i64,
        capabilities: &models::OrganizationCapabilitiesReplace,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
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

    /// Creates an invitation without requiring access to invitation details.
    ///
    /// Requires `organization.invitations.create` plus the represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn create_invitation(
        &self,
        org_id: &str,
        input: &models::CarbonInviteCreate,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
        self.0
            .post(
                &["organizations", org_id, "carbon-invites"],
                input,
                mutation,
            )
            .await
    }

    /// Completes verified invitation admission and returns a scoped membership receipt.
    ///
    /// Requires `organizations.join` plus the represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn join_organization(
        &self,
        org_id: &str,
        acceptance: &models::InvitationAcceptance,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
        self.0
            .post(&["organizations", org_id, "join"], acceptance, mutation)
            .await
    }

    /// Creates a Silicon, preserving its generated identifiers and one-time credential.
    ///
    /// Requires `organization.silicons.create` plus the represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn create_silicon(
        &self,
        org_id: &str,
        input: &models::SiliconCreate,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
        self.0
            .post(&["organizations", org_id, "silicons"], input, mutation)
            .await
    }

    /// Updates a Silicon and returns only independently readable fields.
    ///
    /// Requires `organization.silicons.update` plus the represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn update_silicon(
        &self,
        org_id: &str,
        silicon_id: &str,
        version: i64,
        patch: &models::SiliconPatch,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
        self.0
            .patch(
                &["organizations", org_id, "silicons", silicon_id],
                version,
                patch,
                mutation,
            )
            .await
    }

    /// Requests credential rotation without requiring governance read access.
    ///
    /// Requires `organization.silicons.credentials.rotate` plus the represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn request_token_rotation(
        &self,
        org_id: &str,
        silicon_id: &str,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
        self.0
            .post(
                &[
                    "organizations",
                    org_id,
                    "silicons",
                    silicon_id,
                    "token-rotation-requests",
                ],
                &serde_json::json!({}),
                mutation,
            )
            .await
    }

    /// Creates a tag and preserves its generated identifier and version.
    ///
    /// Requires `organization.tags.create` plus the represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn create_tag(
        &self,
        org_id: &str,
        input: &models::TagCreate,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
        self.0
            .post(&["organizations", org_id, "tags"], input, mutation)
            .await
    }

    /// Updates a tag without requiring tag-catalog read access.
    ///
    /// Requires `organization.tags.update` plus the represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn update_tag(
        &self,
        org_id: &str,
        tag_id: Uuid,
        version: i64,
        patch: &models::TagPatch,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
        self.0
            .patch(
                &["organizations", org_id, "tags", &tag_id.to_string()],
                version,
                patch,
                mutation,
            )
            .await
    }

    /// Updates default trust; a write-only caller may receive an empty object.
    ///
    /// Requires `organization.trust.update` plus the represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn replace_default_trust(
        &self,
        org_id: &str,
        version: i64,
        value: &models::TrustValue,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
        self.0
            .put(
                &["organizations", org_id, "trust", "default"],
                version,
                value,
                mutation,
            )
            .await
    }

    /// Creates a trust rule and preserves its scoped mutation receipt.
    ///
    /// Requires `organization.trust.update` plus the represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn create_trust_rule(
        &self,
        org_id: &str,
        input: &models::TrustRuleCreate,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
        self.0
            .post(
                &["organizations", org_id, "trust", "rules"],
                input,
                mutation,
            )
            .await
    }

    /// Updates a trust rule without requiring trust read access.
    ///
    /// Requires `organization.trust.update` plus the represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn update_trust_rule(
        &self,
        org_id: &str,
        rule_id: Uuid,
        version: i64,
        patch: &models::TrustRulePatch,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
        self.0
            .patch(
                &[
                    "organizations",
                    org_id,
                    "trust",
                    "rules",
                    &rule_id.to_string(),
                ],
                version,
                patch,
                mutation,
            )
            .await
    }

    /// Records an eligible approval decision and returns its scoped receipt.
    ///
    /// Requires `organization.change_requests.decide` plus the represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn decide_change_request(
        &self,
        org_id: &str,
        request_id: Uuid,
        version: i64,
        decision: &models::ApprovalDecisionCreate,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
        self.0
            .post_versioned(
                &[
                    "organizations",
                    org_id,
                    "approval-requests",
                    &request_id.to_string(),
                    "decisions",
                ],
                version,
                decision,
                mutation,
            )
            .await
    }

    /// Submits a job-role change request without requiring governance read access.
    ///
    /// Requires `organization.job_role_changes.request` plus the represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn request_job_role_change(
        &self,
        org_id: &str,
        request: &models::RoleChangeRequestCreate,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
        self.0
            .post(
                &["organizations", org_id, "role-change-requests"],
                request,
                mutation,
            )
            .await
    }

    /// Submits a member tag-change request and preserves its scoped receipt.
    ///
    /// Requires `organization.tag_changes.request` plus the represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn request_tag_change(
        &self,
        org_id: &str,
        membership_id: &str,
        request: &models::TagChangeRequestCreate,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
        self.0
            .post(
                &[
                    "organizations",
                    org_id,
                    "members",
                    membership_id,
                    "tag-change-requests",
                ],
                request,
                mutation,
            )
            .await
    }

    /// Replaces a member job role and returns only authorized member fields.
    ///
    /// Requires `organization.job_roles.update` plus the represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn replace_job_role(
        &self,
        org_id: &str,
        membership_id: &str,
        version: i64,
        job_role: &models::DirectJobRoleReplace,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
        self.0
            .put(
                &[
                    "organizations",
                    org_id,
                    "members",
                    membership_id,
                    "job-role",
                ],
                version,
                job_role,
                mutation,
            )
            .await
    }

    /// Replaces member tags and returns only authorized member fields.
    ///
    /// Requires `organization.member_tags.update` plus the represented user's existing authority.
    /// Existing step-up and version preconditions still apply.
    ///
    /// # Errors
    /// Fails if authorization, validation, step-up, or concurrency checks fail.
    pub async fn replace_tags(
        &self,
        org_id: &str,
        membership_id: &str,
        version: i64,
        tags: &models::DirectTagSetReplace,
        mutation: &Mutation,
    ) -> Result<models::ApplicationMutationObject> {
        self.0
            .put(
                &["organizations", org_id, "members", membership_id, "tags"],
                version,
                tags,
                mutation,
            )
            .await
    }
}
