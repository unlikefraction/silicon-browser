//! Silicons: their identity, their credential, and their webhook.

use uuid::Uuid;

use crate::{Client, Mutation, Paging, Result, models};

/// Silicon identities inside one organization.
pub struct Silicons<'a>(pub(super) &'a Client);

impl Silicons<'_> {
    /// Reads the authenticated Silicon and its authoritative owning organization.
    /// # Errors
    /// Requires an authenticated Silicon session.
    pub async fn me(&self) -> Result<models::Silicon> {
        self.0.get(&["me"]).await
    }

    /// The organization's Silicons.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller is not a member.
    pub async fn list(
        &self,
        org_id: &str,
        tag_id: Option<Uuid>,
        paging: &Paging,
    ) -> Result<models::SiliconPage> {
        let mut query = paging.query();
        if let Some(tag_id) = tag_id {
            query.push(("tag_id", tag_id.to_string()));
        }
        self.0
            .get_with(&["organizations", org_id, "silicons"], &query)
            .await
    }

    /// Creates a Silicon, returning its one-time credential.
    ///
    /// The credential is shown once. Store it before doing anything else; it
    /// cannot be read back, only rotated.
    ///
    /// # Errors
    ///
    /// Returns an error when the handle is taken or the caller lacks
    /// `silicons.create`.
    pub async fn create(
        &self,
        org_id: &str,
        input: &models::SiliconCreate,
        mutation: &Mutation,
    ) -> Result<models::SiliconCreated> {
        self.0
            .post(&["organizations", org_id, "silicons"], input, mutation)
            .await
    }

    /// One Silicon by its global ID.
    ///
    /// # Errors
    ///
    /// Returns an error when the Silicon does not exist here.
    pub async fn get(&self, org_id: &str, silicon_id: &str) -> Result<models::Silicon> {
        self.0
            .get(&["organizations", org_id, "silicons", silicon_id])
            .await
    }

    /// Updates a Silicon's directory configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale or the caller lacks
    /// `silicons.update_directory`.
    pub async fn update(
        &self,
        org_id: &str,
        silicon_id: &str,
        version: i64,
        patch: &models::SiliconPatch,
        mutation: &Mutation,
    ) -> Result<models::Silicon> {
        self.0
            .patch(
                &["organizations", org_id, "silicons", silicon_id],
                version,
                patch,
                mutation,
            )
            .await
    }

    /// Removes a Silicon.
    ///
    /// `reassign_reports_to` is required when other Silicons report to this
    /// one.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale or reports would be orphaned.
    pub async fn remove(
        &self,
        org_id: &str,
        silicon_id: &str,
        version: i64,
        reassign_reports_to: Option<&str>,
        mutation: &Mutation,
    ) -> Result<()> {
        let query = reassign_reports_to
            .map(|target| vec![("reassign_reports_to", target.to_string())])
            .unwrap_or_default();
        self.0
            .delete_with(
                &["organizations", org_id, "silicons", silicon_id],
                Some(version),
                &query,
                mutation,
            )
            .await
    }

    /// Requests rotation of a Silicon's long-lived credential.
    ///
    /// Raises an approval request; the new credential appears only when the
    /// rotation is completed.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller lacks `silicons.rotate_token`.
    pub async fn request_token_rotation(
        &self,
        org_id: &str,
        silicon_id: &str,
        mutation: &Mutation,
    ) -> Result<models::ApprovalRequest> {
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

    /// Completes an approved rotation, returning the new credential once.
    ///
    /// # Errors
    ///
    /// Returns an error when the request is not approved, or already spent.
    pub async fn complete_token_rotation(
        &self,
        org_id: &str,
        silicon_id: &str,
        request_id: Uuid,
        mutation: &Mutation,
    ) -> Result<models::SiliconTokenRotated> {
        self.0
            .post(
                &[
                    "organizations",
                    org_id,
                    "silicons",
                    silicon_id,
                    "token-rotation-requests",
                    &request_id.to_string(),
                    "complete",
                ],
                &serde_json::json!({}),
                mutation,
            )
            .await
    }

    /// The Silicon's webhook endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error when no endpoint is configured.
    pub async fn webhook(&self, org_id: &str, silicon_id: &str) -> Result<models::SiliconWebhook> {
        self.0
            .get(&["organizations", org_id, "silicons", silicon_id, "webhook"])
            .await
    }

    /// Configures or replaces the webhook endpoint, rotating its signing key.
    ///
    /// `version` is required when an active endpoint already exists, and must
    /// be `None` for the first configuration -- there is no representation to
    /// match against yet.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale or absent where required, or a
    /// Carbon caller has not stepped up.
    pub async fn replace_webhook(
        &self,
        org_id: &str,
        silicon_id: &str,
        version: Option<i64>,
        input: &models::SiliconWebhookReplace,
        mutation: &Mutation,
    ) -> Result<models::SiliconWebhookConfigured> {
        self.0
            .put_optional_version(
                &["organizations", org_id, "silicons", silicon_id, "webhook"],
                version,
                input,
                mutation,
            )
            .await
    }

    /// Removes the webhook endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale or no endpoint exists.
    pub async fn delete_webhook(
        &self,
        org_id: &str,
        silicon_id: &str,
        version: i64,
        mutation: &Mutation,
    ) -> Result<()> {
        self.0
            .delete(
                &["organizations", org_id, "silicons", silicon_id, "webhook"],
                Some(version),
                mutation,
            )
            .await
    }

    /// What the Silicon's webhook is subscribed to.
    ///
    /// # Errors
    ///
    /// Returns an error when no subscription exists.
    pub async fn subscription(
        &self,
        org_id: &str,
        silicon_id: &str,
    ) -> Result<models::SiliconWebhookSubscription> {
        self.0
            .get(&[
                "organizations",
                org_id,
                "silicons",
                silicon_id,
                "webhook",
                "subscription",
            ])
            .await
    }

    /// Replaces the subscription: mode, topics, and tag filters.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale or a topic is unrecognized.
    pub async fn replace_subscription(
        &self,
        org_id: &str,
        silicon_id: &str,
        version: Option<i64>,
        input: &models::SiliconWebhookSubscriptionReplace,
        mutation: &Mutation,
    ) -> Result<models::SiliconWebhookSubscription> {
        self.0
            .put_optional_version(
                &[
                    "organizations",
                    org_id,
                    "silicons",
                    silicon_id,
                    "webhook",
                    "subscription",
                ],
                version,
                input,
                mutation,
            )
            .await
    }

    /// Removes the subscription, leaving the endpoint configured.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale.
    pub async fn delete_subscription(
        &self,
        org_id: &str,
        silicon_id: &str,
        version: i64,
        mutation: &Mutation,
    ) -> Result<()> {
        self.0
            .delete(
                &[
                    "organizations",
                    org_id,
                    "silicons",
                    silicon_id,
                    "webhook",
                    "subscription",
                ],
                Some(version),
                mutation,
            )
            .await
    }

    /// Deliveries that exhausted their retries.
    ///
    /// # Errors
    ///
    /// Returns an error when no endpoint is configured.
    pub async fn dead_letters(
        &self,
        org_id: &str,
        silicon_id: &str,
        paging: &Paging,
    ) -> Result<models::WebhookDeadLetterPage> {
        self.0
            .get_with(
                &[
                    "organizations",
                    org_id,
                    "silicons",
                    silicon_id,
                    "webhook",
                    "dead-letters",
                ],
                &paging.query(),
            )
            .await
    }

    /// Re-queues dead-lettered deliveries.
    ///
    /// # Errors
    ///
    /// Returns an error when a named delivery is not dead-lettered here.
    pub async fn replay_dead_letters(
        &self,
        org_id: &str,
        silicon_id: &str,
        request: &models::WebhookReplayRequest,
        mutation: &Mutation,
    ) -> Result<models::WebhookReplayResponse> {
        self.0
            .post(
                &[
                    "organizations",
                    org_id,
                    "silicons",
                    silicon_id,
                    "webhook",
                    "dead-letters",
                    "replays",
                ],
                request,
                mutation,
            )
            .await
    }
}
