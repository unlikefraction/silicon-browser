//! Changes that need approval, and the history they leave behind.
//!
//! Sensitive actions use configurable policies. Submit the direct mutation;
//! a 428 `approval_required` response identifies any required approval. Once
//! approved, retry the exact mutation with the original request key.

use uuid::Uuid;

use crate::{Client, Mutation, Paging, Result, models};

/// Approval requests, direct changes, and history.
pub struct Governance<'a>(pub(super) &'a Client);

/// Narrows an approval-request listing.
#[derive(Clone, Debug, Default)]
pub struct ApprovalFilter {
    /// Only requests in one state.
    pub status: Option<String>,
    /// Only requests of one kind.
    pub kind: Option<String>,
    /// Only requests the caller can currently decide.
    pub actionable_by_me: Option<bool>,
}

impl ApprovalFilter {
    /// Only the requests waiting on this caller.
    #[must_use]
    pub fn actionable() -> Self {
        Self {
            actionable_by_me: Some(true),
            ..Self::default()
        }
    }

    fn query(&self) -> Vec<(&'static str, String)> {
        let mut query = Vec::new();
        if let Some(status) = &self.status {
            query.push(("status", status.clone()));
        }
        if let Some(kind) = &self.kind {
            query.push(("kind", kind.clone()));
        }
        if let Some(actionable) = self.actionable_by_me {
            query.push(("actionable_by_me", actionable.to_string()));
        }
        query
    }
}

impl Governance<'_> {
    /// Lists effective sensitive-action rules and whether the caller can configure them.
    /// # Errors
    /// Returns an error when the caller is not an active member.
    pub async fn action_policies(&self, org_id: &str) -> Result<models::ActionPolicyList> {
        self.0
            .get(&["organizations", org_id, "action-policies"])
            .await
    }

    /// Configures a sensitive-action rule using its current version (zero for defaults).
    /// # Errors
    /// Returns an error when authority, selectors or version are invalid.
    pub async fn configure_action_policy(
        &self,
        org_id: &str,
        action: &str,
        version: i64,
        input: &models::ActionPolicyUpdate,
        mutation: &Mutation,
    ) -> Result<models::ActionPolicy> {
        self.0
            .put(
                &["organizations", org_id, "action-policies", action],
                version,
                input,
                mutation,
            )
            .await
    }

    /// Lists sensitive-action requests visible to this member.
    /// # Errors
    /// Returns an error when the caller is not an active member.
    pub async fn action_approvals(&self, org_id: &str) -> Result<models::ActionApprovalList> {
        self.0
            .get(&["organizations", org_id, "action-approvals"])
            .await
    }

    /// Approves or rejects the exact proposed sensitive action.
    /// # Errors
    /// Returns an error for stale, expired, unauthorized or no longer pending requests.
    pub async fn decide_action(
        &self,
        org_id: &str,
        request_id: Uuid,
        version: i64,
        input: &models::ActionApprovalDecision,
        mutation: &Mutation,
    ) -> Result<models::ActionApproval> {
        self.0
            .post_versioned(
                &[
                    "organizations",
                    org_id,
                    "action-approvals",
                    &request_id.to_string(),
                    "decisions",
                ],
                version,
                input,
                mutation,
            )
            .await
    }

    /// Approval requests in an organization.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller is not a member.
    pub async fn list_approvals(
        &self,
        org_id: &str,
        filter: &ApprovalFilter,
        paging: &Paging,
    ) -> Result<models::ApprovalRequestPage> {
        let mut query = paging.query();
        query.extend(filter.query());
        self.0
            .get_with(&["organizations", org_id, "approval-requests"], &query)
            .await
    }

    /// One approval request, with its requirements and decisions so far.
    ///
    /// # Errors
    ///
    /// Returns an error when the request does not exist here.
    pub async fn get_approval(
        &self,
        org_id: &str,
        request_id: Uuid,
    ) -> Result<models::ApprovalRequest> {
        self.0
            .get(&[
                "organizations",
                org_id,
                "approval-requests",
                &request_id.to_string(),
            ])
            .await
    }

    /// Approves or rejects a request.
    ///
    /// Some kinds additionally require a step-up assertion; the service says
    /// so with `step_up_required`.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale, the caller is not an eligible
    /// approver, or a required step-up is missing.
    pub async fn decide(
        &self,
        org_id: &str,
        request_id: Uuid,
        version: i64,
        decision: &models::ApprovalDecisionCreate,
        mutation: &Mutation,
    ) -> Result<models::ApprovalRequest> {
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

    /// Raises a job-role change request.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller may not request the change.
    pub async fn request_role_change(
        &self,
        org_id: &str,
        request: &models::RoleChangeRequestCreate,
        mutation: &Mutation,
    ) -> Result<models::ApprovalRequest> {
        self.0
            .post(
                &["organizations", org_id, "role-change-requests"],
                request,
                mutation,
            )
            .await
    }

    /// Raises a tag change request for one member.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller may not request the change.
    pub async fn request_tag_change(
        &self,
        org_id: &str,
        membership_id: &str,
        request: &models::TagChangeRequestCreate,
        mutation: &Mutation,
    ) -> Result<models::ApprovalRequest> {
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

    /// Sets a member's job role directly, without an approval request.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale or the caller lacks the
    /// authority to apply the change directly.
    pub async fn replace_job_role(
        &self,
        org_id: &str,
        membership_id: &str,
        version: i64,
        job_role: &models::DirectJobRoleReplace,
        mutation: &Mutation,
    ) -> Result<models::Membership> {
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

    /// Replaces a member's complete tag set directly.
    ///
    /// Send every tag the member should end up with; anything omitted is
    /// removed.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale, a tag does not exist, or the
    /// caller lacks `tags.manage`.
    pub async fn replace_tags(
        &self,
        org_id: &str,
        membership_id: &str,
        version: i64,
        tags: &models::DirectTagSetReplace,
        mutation: &Mutation,
    ) -> Result<models::Membership> {
        self.0
            .put(
                &["organizations", org_id, "members", membership_id, "tags"],
                version,
                tags,
                mutation,
            )
            .await
    }

    /// A member's job-role history.
    ///
    /// # Errors
    ///
    /// Returns an error when the membership does not exist here.
    pub async fn job_role_history(
        &self,
        org_id: &str,
        membership_id: &str,
        paging: &Paging,
    ) -> Result<models::RoleHistoryPage> {
        self.0
            .get_with(
                &[
                    "organizations",
                    org_id,
                    "members",
                    membership_id,
                    "job-role-history",
                ],
                &paging.query(),
            )
            .await
    }

    /// A member's tag history.
    ///
    /// # Errors
    ///
    /// Returns an error when the membership does not exist here.
    pub async fn tag_history(
        &self,
        org_id: &str,
        membership_id: &str,
        paging: &Paging,
    ) -> Result<models::TagHistoryPage> {
        self.0
            .get_with(
                &[
                    "organizations",
                    org_id,
                    "members",
                    membership_id,
                    "tag-history",
                ],
                &paging.query(),
            )
            .await
    }
}
