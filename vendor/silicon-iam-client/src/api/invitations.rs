//! Inviting Carbons into an organization, and joining on an invitation.

use uuid::Uuid;

use crate::{Client, Mutation, Paging, Result, models};

/// Invitations, from both ends.
pub struct Invitations<'a>(pub(super) &'a Client);

impl Invitations<'_> {
    /// Existing Silicons visible through one of your active source organizations.
    /// # Errors
    /// Requires members.invite in the destination organization.
    pub async fn silicon_candidates(&self, org: &str) -> Result<serde_json::Value> {
        self.0
            .get(&["organizations", org, "silicon-invitations", "candidates"])
            .await
    }
    /// Invitations issued to existing Silicon accounts.
    /// # Errors
    /// Requires members.invite.
    pub async fn silicon_list(&self, org: &str) -> Result<serde_json::Value> {
        self.0
            .get(&["organizations", org, "silicon-invitations"])
            .await
    }
    /// Invites a visible Silicon from another organization you belong to.
    /// # Errors
    /// Fails for an unrelated/hidden Silicon, existing member or missing authority.
    pub async fn silicon_create(
        &self,
        org: &str,
        silicon_id: &str,
        mutation: &Mutation,
    ) -> Result<serde_json::Value> {
        self.0
            .post(
                &["organizations", org, "silicon-invitations"],
                &serde_json::json!({"silicon_id":silicon_id}),
                mutation,
            )
            .await
    }
    /// Reads invitations addressed to the directly authenticated Silicon.
    /// # Errors
    /// Rejects Carbon and application sessions.
    pub async fn silicon_inbox(&self) -> Result<serde_json::Value> {
        self.0.get(&["me", "silicon-invitations"]).await
    }
    /// Accepts or declines an invitation as its Silicon account; custody is unchanged.
    /// # Errors
    /// Rejects another account, stale invitations or lost inviter authority.
    pub async fn silicon_decide(
        &self,
        id: Uuid,
        accept: bool,
        mutation: &Mutation,
    ) -> Result<serde_json::Value> {
        self.0
            .post(
                &["me", "silicon-invitations", &id.to_string(), "decision"],
                &serde_json::json!({"decision":if accept{"accept"}else{"decline"}}),
                mutation,
            )
            .await
    }
    /// Revokes a pending invitation created in the organization.
    /// # Errors
    /// Requires members.invite; an accepted invitation is not a membership removal.
    pub async fn silicon_revoke(
        &self,
        org: &str,
        id: Uuid,
        mutation: &Mutation,
    ) -> Result<serde_json::Value> {
        self.0
            .post(
                &[
                    "organizations",
                    org,
                    "silicon-invitations",
                    &id.to_string(),
                    "revoke",
                ],
                &serde_json::json!({}),
                mutation,
            )
            .await
    }

    /// Invitations issued by this organization.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller lacks `members.invite`.
    pub async fn list(
        &self,
        org_id: &str,
        status: Option<&str>,
        paging: &Paging,
    ) -> Result<models::InvitePage> {
        let mut query = paging.query();
        if let Some(status) = status {
            query.push(("status", status.to_owned()));
        }
        self.0
            .get_with(&["organizations", org_id, "carbon-invites"], &query)
            .await
    }

    /// Invites a Carbon by Carbon ID or email address.
    ///
    /// The Carbon must already exist: invitations do not create accounts.
    ///
    /// # Errors
    ///
    /// Returns an error when the identity is unknown, already a member, or the
    /// caller lacks `members.invite`.
    pub async fn create(
        &self,
        org_id: &str,
        input: &models::CarbonInviteCreate,
        mutation: &Mutation,
    ) -> Result<models::Invite> {
        self.0
            .post(
                &["organizations", org_id, "carbon-invites"],
                input,
                mutation,
            )
            .await
    }

    /// One invitation.
    ///
    /// # Errors
    ///
    /// Returns an error when the invitation does not exist here.
    pub async fn get(&self, org_id: &str, invite_id: Uuid) -> Result<models::Invite> {
        self.0
            .get(&[
                "organizations",
                org_id,
                "carbon-invites",
                &invite_id.to_string(),
            ])
            .await
    }

    /// Revokes a pending invitation.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale or the invitation was already
    /// accepted.
    pub async fn revoke(
        &self,
        org_id: &str,
        invite_id: Uuid,
        version: i64,
        mutation: &Mutation,
    ) -> Result<()> {
        self.0
            .delete(
                &[
                    "organizations",
                    org_id,
                    "carbon-invites",
                    &invite_id.to_string(),
                ],
                Some(version),
                mutation,
            )
            .await
    }

    /// Sends the invitee a verification code for an email invitation.
    ///
    /// Called by the invitee, not the inviter.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller has no usable invitation here.
    pub async fn send_join_code(
        &self,
        org_id: &str,
        email: &str,
        mutation: &Mutation,
    ) -> Result<models::InvitationEmailCodeResponse> {
        self.0
            .post(
                &["organizations", org_id, "join", "email-verification-code"],
                &models::EmailInput {
                    email: email.to_owned(),
                },
                mutation,
            )
            .await
    }

    /// Accepts an invitation and joins the organization.
    ///
    /// # Errors
    ///
    /// Returns an error when the code is wrong or the invitation expired.
    pub async fn join(
        &self,
        org_id: &str,
        acceptance: &models::InvitationAcceptance,
        mutation: &Mutation,
    ) -> Result<models::Membership> {
        self.0
            .post(&["organizations", org_id, "join"], acceptance, mutation)
            .await
    }
}
