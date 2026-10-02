//! Scope-projected IAM reads made with an application's user access token.
//!
//! Fields absent from these objects are undisclosed. This surface deliberately
//! preserves omission instead of inventing roles, tags, contact details, or defaults.

use serde_json::Value;

use crate::{Client, Paging, Result, models};

/// IAM read projections for an application's explicitly consented scopes.
pub struct ApplicationReads<'a>(pub(super) &'a Client);

impl ApplicationReads<'_> {
    /// Reads the represented Carbon or Silicon's disclosed identity/profile/contacts.
    ///
    /// # Errors
    /// Fails if the token has no applicable self scope or is no longer active.
    pub async fn me(&self) -> Result<Value> {
        self.0.get(&["me"]).await
    }

    /// Lists only selected active organizations and their approved public details.
    ///
    /// # Errors
    /// Requires `self.organizations.read`.
    pub async fn organizations(&self, paging: &Paging) -> Result<Value> {
        self.0.get_with(&["organizations"], &paging.query()).await
    }

    /// Reads one selected organization's disclosed details.
    ///
    /// # Errors
    /// Requires organization consent and `self.organizations.read`.
    pub async fn organization(&self, org_id: &str) -> Result<Value> {
        self.0.get(&["organizations", org_id]).await
    }

    /// Lists only approved actor types with independently scoped member fields.
    ///
    /// # Errors
    /// Requires `directory.carbons.read` or `directory.silicons.read` and selected membership.
    pub async fn members(&self, org_id: &str, paging: &Paging) -> Result<Value> {
        self.0
            .get_with(&["organizations", org_id, "members"], &paging.query())
            .await
    }

    /// Reads a scoped membership; self and other-member permissions remain independent.
    ///
    /// # Errors
    /// Fails for unselected organizations or missing directory permissions for another actor.
    pub async fn member(&self, org_id: &str, membership_id: &str) -> Result<Value> {
        self.0
            .get(&["organizations", org_id, "members", membership_id])
            .await
    }

    /// Reads only disclosed role and explicit capabilities for a membership.
    ///
    /// # Errors
    /// Requires the applicable self or directory membership/capability scope.
    pub async fn member_authorization(&self, org_id: &str, membership_id: &str) -> Result<Value> {
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

    /// Reads one Silicon's independently scoped identity, profile, tags, and hierarchy.
    ///
    /// # Errors
    /// Requires self scope for the represented Silicon or directory scope for another Silicon.
    pub async fn silicon(&self, org_id: &str, silicon_id: &str) -> Result<Value> {
        self.0
            .get(&["organizations", org_id, "silicons", silicon_id])
            .await
    }

    /// Lists the organization's full tag catalog.
    ///
    /// # Errors
    /// Requires `organization.tags.read` and selected organization membership.
    pub async fn tags(&self, org_id: &str, paging: &Paging) -> Result<Value> {
        self.0
            .get_with(&["organizations", org_id, "tags"], &paging.query())
            .await
    }
    /// Reads the represented actor's directory view from its own trust perspective.
    ///
    /// # Errors
    /// Requires a selected active organization; fields follow the corresponding self scopes.
    pub async fn directory_self(&self, org_id: &str) -> Result<Value> {
        self.0
            .get(&["organizations", org_id, "directory", "self"])
            .await
    }

    /// Reads a directory page with independently scoped fields and actor types.
    ///
    /// # Errors
    /// Requires directory actor permission and selected membership.
    pub async fn directory(&self, org_id: &str, paging: &Paging) -> Result<Value> {
        self.0
            .get_with(
                &["organizations", org_id, "directory", "members"],
                &paging.query(),
            )
            .await
    }

    /// Lists Silicon identities with only their authorized optional fields.
    ///
    /// # Errors
    /// Requires `directory.silicons.read`.
    pub async fn silicons(&self, org_id: &str, paging: &Paging) -> Result<Value> {
        self.0
            .get_with(&["organizations", org_id, "silicons"], &paging.query())
            .await
    }

    /// Evaluates trust involving the represented user, or raw organization trust with its critical scope.
    ///
    /// # Errors
    /// Requires `self.trust.read` for the user's perspective or `organization.trust.read`.
    pub async fn evaluate_trust(
        &self,
        org_id: &str,
        input: &models::TrustEvaluationRequest,
    ) -> Result<Value> {
        let built = self
            .0
            .route(
                reqwest::Method::POST,
                &["organizations", org_id, "trust", "effective"],
            )?
            .json(input);
        self.0.send_json(built).await
    }
}
