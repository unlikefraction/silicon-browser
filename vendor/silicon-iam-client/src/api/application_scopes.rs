//! Declared IAM and external application permissions, and critical-scope reviews.

use uuid::Uuid;

use crate::{Client, Mutation, Result, models};

/// Scope discovery and application-specific approval discussions.
pub struct ApplicationScopes<'a>(pub(super) &'a Client);

impl ApplicationScopes<'_> {
    /// Lists IAM scopes, or the exposed OBO scopes of one audience application.
    ///
    /// # Errors
    /// Fails if the caller cannot authenticate or the audience is unavailable.
    pub async fn catalog(&self, app_id: Option<&str>) -> Result<models::ApplicationScopeCatalog> {
        let query = app_id
            .map(|id| ("app_id", id.to_owned()))
            .into_iter()
            .collect::<Vec<_>>();
        self.0.get_with(&["application-scopes"], &query).await
    }

    /// Lists discussions visible to the requester or target application's administrators.
    ///
    /// # Errors
    /// Fails if the caller cannot authenticate or the status is invalid.
    pub async fn requests(
        &self,
        status: Option<&str>,
    ) -> Result<models::ApplicationScopeRequestList> {
        let query = status
            .map(|value| ("status", value.to_owned()))
            .into_iter()
            .collect::<Vec<_>>();
        self.0
            .get_with(&["application-scope-requests"], &query)
            .await
    }

    /// Requests a revised scope set, grouped into separate audience review discussions.
    /// The previous effective scope set remains active during an upgrade review.
    ///
    /// # Errors
    /// Fails if permission is missing, the version is stale, or scopes are invalid.
    pub async fn request(
        &self,
        app_id: &str,
        version: i64,
        input: &models::ApplicationScopeRequestCreate,
        mutation: &Mutation,
    ) -> Result<models::ApplicationScopeRequestList> {
        self.0
            .post_versioned(
                &["applications", app_id, "scope-requests"],
                version,
                input,
                mutation,
            )
            .await
    }

    /// Reads the request, reviewer authority, and complete discussion.
    ///
    /// # Errors
    /// Fails when the caller is not a participant administrator or IAM reviewer.
    pub async fn get(&self, request_id: Uuid) -> Result<models::ApplicationScopeRequest> {
        self.0
            .get(&["application-scope-requests", &request_id.to_string()])
            .await
    }

    /// Adds a normal text reply to a review discussion.
    ///
    /// # Errors
    /// Fails if the request version is stale or the caller cannot participate.
    pub async fn reply(
        &self,
        request_id: Uuid,
        version: i64,
        input: &models::ApplicationScopeMessageCreate,
        mutation: &Mutation,
    ) -> Result<models::ApplicationScopeRequest> {
        self.0
            .post_versioned(
                &[
                    "application-scope-requests",
                    &request_id.to_string(),
                    "messages",
                ],
                version,
                input,
                mutation,
            )
            .await
    }

    /// Approves or denies this request; a denial requires a nonempty reason.
    ///
    /// # Errors
    /// Fails when reviewer authority is missing, the version is stale, or the decision is invalid.
    pub async fn decide(
        &self,
        request_id: Uuid,
        version: i64,
        input: &models::ApplicationScopeDecision,
        mutation: &Mutation,
    ) -> Result<models::ApplicationScopeRequest> {
        self.0
            .post_versioned(
                &[
                    "application-scope-requests",
                    &request_id.to_string(),
                    "decisions",
                ],
                version,
                input,
                mutation,
            )
            .await
    }
}
