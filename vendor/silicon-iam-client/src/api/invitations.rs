//! Inviting Carbons into an organization, and joining on an invitation.

use uuid::Uuid;

use crate::{Client, Mutation, Paging, Result, models};

/// Invitations, from both ends.
pub struct Invitations<'a>(pub(super) &'a Client);

impl Invitations<'_> {
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
