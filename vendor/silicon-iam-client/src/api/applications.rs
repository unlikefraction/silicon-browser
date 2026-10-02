//! Applications: their public base URLs, credentials, OBO surface, and webhook.

use crate::{Client, Error, Mutation, Paging, Result, models};

/// Application management. Applications are organization-owned, and these
/// routes need a direct Carbon token with owner or admin membership in the
/// owning organization. Webhook inspection and approval also allow IAM
/// reviewers with the `applications.review` platform capability.
pub struct Applications<'a>(pub(super) &'a Client);

impl Applications<'_> {
    /// Applications the caller can administer.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails.
    pub async fn list(
        &self,
        status: Option<&str>,
        paging: &Paging,
    ) -> Result<models::ApplicationPage> {
        self.list_filtered(None, status, paging).await
    }

    /// Lists applications in one organization, filtering before pagination.
    ///
    /// # Errors
    /// Returns an error when the organization is unavailable or a filter is invalid.
    pub async fn list_for_organization(
        &self,
        org_id: &str,
        status: Option<&str>,
        paging: &Paging,
    ) -> Result<models::ApplicationPage> {
        self.list_filtered(Some(org_id), status, paging).await
    }

    async fn list_filtered(
        &self,
        org_id: Option<&str>,
        status: Option<&str>,
        paging: &Paging,
    ) -> Result<models::ApplicationPage> {
        let mut query = paging.query();
        if let Some(org_id) = org_id {
            query.push(("org_id", org_id.to_owned()));
        }
        if let Some(status) = status {
            query.push(("status", status.to_owned()));
        }
        self.0.get_with(&["applications"], &query).await
    }

    /// Registers an application with separate login and webhook scopes.
    /// New critical permissions remain unavailable until their reviewers approve.
    ///
    /// In production, only the submitted webhook destination remains pending
    /// approval by an owning-organization owner/admin or IAM reviewer. A testing environment activates it immediately because
    /// that isolated plane has no platform reviewer. The caller chooses the
    /// webhook signing secret; IAM generates only the returned client secret.
    ///
    /// # Errors
    ///
    /// Returns an error when the identifier is taken or a field is rejected.
    pub async fn create(
        &self,
        input: &models::ApplicationCreate,
        mutation: &Mutation,
    ) -> Result<models::ApplicationCreated> {
        self.0.post(&["applications"], input, mutation).await
    }

    /// Creates or reuses an IAM test environment and imports this application's dependencies.
    /// Authenticate with the production application's own credential. Dependency
    /// credentials are kept by IAM; only this application's test secret is returned.
    ///
    /// # Errors
    /// Fails when the application credential or supplied existing test key is invalid.
    pub async fn create_testing_environment(
        &self,
        input: &models::ApplicationTestingEnvironmentCreate,
        mutation: &Mutation,
    ) -> Result<models::ApplicationTestingEnvironmentCreated> {
        self.0
            .post(&["application", "testing-environments"], input, mutation)
            .await
    }

    /// Lists test environments linked to this production application and its organization.
    /// `can_manage` identifies environments this app created and can manage with
    /// the same credential through `client.environments()`.
    ///
    /// # Errors
    /// Fails when the requesting application cannot authenticate.
    pub async fn testing_environments(
        &self,
        status: Option<&str>,
        paging: &Paging,
    ) -> Result<models::ApplicationTestingEnvironmentPage> {
        let mut query = paging.query();
        if let Some(status) = status {
            query.push(("status", status.to_owned()));
        }
        self.0
            .get_with(&["application", "testing-environments"], &query)
            .await
    }

    /// Authenticates this application's test credential in the selected environment.
    /// Use a test `Credential::Application` together with `Client::with_environment`.
    /// The response contains no secrets and grants no production authority.
    ///
    /// # Errors
    /// Fails for missing/invalid environment keys, production secrets, or another app's secret.
    pub async fn testing_context(&self) -> Result<models::ApplicationTestingContext> {
        self.0.get(&["application", "testing-context"]).await
    }

    /// One application.
    ///
    /// # Errors
    ///
    /// Returns an error when the application does not exist, or the caller
    /// cannot administer it.
    pub async fn get(&self, app_id: &str) -> Result<models::Application> {
        self.0.get(&["applications", app_id]).await
    }

    /// Discovers a public application's accepted backend origin anonymously.
    ///
    /// Private applications require an authorized user or application credential.
    /// Supplied credentials are validated even for public targets. Testing context
    /// confines both the caller and the target to the selected environment.
    ///
    /// # Errors
    /// Returns an error for invalid credentials, unavailable targets or unauthorized
    /// private access, without revealing the private origin.
    pub async fn discover_base_url(&self, app_id: &str) -> Result<models::ApplicationBaseUrl> {
        self.0.get(&["application-directory", app_id]).await
    }

    /// Updates an application's configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale or a field is rejected.
    pub async fn update(
        &self,
        app_id: &str,
        version: i64,
        patch: &models::ApplicationPatch,
        mutation: &Mutation,
    ) -> Result<models::Application> {
        self.0
            .patch(&["applications", app_id], version, patch, mutation)
            .await
    }

    /// Rotates the client secret, returning the new one once.
    ///
    /// Requires a step-up assertion. The previous secret stops working when
    /// the response is committed, so store the new one before reconfiguring
    /// anything that uses it.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale or the step-up is missing.
    pub async fn rotate_secret(
        &self,
        app_id: &str,
        version: i64,
        mutation: &Mutation,
    ) -> Result<models::ApplicationSecretRotated> {
        self.0
            .post_versioned(
                &["applications", app_id, "client-secret-rotations"],
                version,
                &serde_json::json!({}),
                mutation,
            )
            .await
    }

    /// Installs a caller-supplied successor webhook signing secret.
    ///
    /// Requires a verified-channel step-up assertion for
    /// `application.webhook_secret.rotate`. New deliveries use the supplied
    /// secret and returned version; keep older versions until their in-flight
    /// delivery window has closed.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale or the step-up is missing.
    pub async fn rotate_webhook_secret(
        &self,
        app_id: &str,
        version: i64,
        input: &models::ApplicationWebhookSecretRotate,
        mutation: &Mutation,
    ) -> Result<models::ApplicationWebhookSecretRotated> {
        self.0
            .post_versioned(
                &["applications", app_id, "webhook-secret-rotations"],
                version,
                input,
                mutation,
            )
            .await
    }

    /// Imports a production application into the selected testing environment.
    ///
    /// This route exists only in a testing context. The production application
    /// contributes its canonical id, base URL, webhook URL, and OBO surface;
    /// the response never exposes its production webhook signing secret.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invalid`] before sending when this client was not
    /// configured with [`Client::with_environment`].
    pub async fn import_from_production(
        &self,
        app_id: &str,
        mutation: &Mutation,
    ) -> Result<models::TestingApplicationImported> {
        if self.0.environment().is_none() {
            return Err(Error::Invalid(
                "application import is only possible in a testing environment; configure the client with Client::with_environment".to_owned(),
            ));
        }
        self.0
            .post(
                &["testing-environment", "applications", "imports"],
                &models::TestingApplicationImport {
                    app_id: app_id.to_owned(),
                },
                mutation,
            )
            .await
    }

    /// The application's webhook endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error when no endpoint is configured.
    pub async fn webhook(&self, app_id: &str) -> Result<models::ApplicationWebhook> {
        self.0.get(&["applications", app_id, "webhook"]).await
    }

    /// Replaces a webhook endpoint.
    ///
    /// Production proposes it for approval by an owning-organization
    /// owner/admin or IAM reviewer; a testing environment
    /// activates it immediately because that isolated plane has no reviewer.
    ///
    /// A production replacement reuses its signing key unless a successor is
    /// supplied. A testing replacement installs the supplied test-only secret
    /// or generates one, returning it in `webhook_signing_secret`.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale or the URL is rejected.
    pub async fn replace_webhook(
        &self,
        app_id: &str,
        version: i64,
        input: &models::ApplicationWebhookReplace,
        mutation: &Mutation,
    ) -> Result<models::ApplicationWebhook> {
        self.0
            .put(
                &["applications", app_id, "webhook"],
                version,
                input,
                mutation,
            )
            .await
    }

    /// Activates the pending webhook destination of a verified application.
    ///
    /// Requires a direct Carbon session with current owner/admin membership
    /// in the owning organization, or the `applications.review` platform
    /// capability. Supply a verified-channel step-up assertion for
    /// `application.webhook.approve`, bound to the internal Application UUID.
    /// `version` is the application aggregate version returned by [`Self::webhook`].
    /// This changes only the endpoint, not application status or approved scopes.
    ///
    /// # Errors
    ///
    /// Returns an error when permission or step-up is missing, the version is
    /// stale, no endpoint is pending, the application is not verified, or the
    /// destination fails public HTTPS/DNS validation.
    pub async fn approve_webhook(
        &self,
        app_id: &str,
        version: i64,
        mutation: &Mutation,
    ) -> Result<models::ApplicationWebhook> {
        self.0
            .post_versioned(
                &["applications", app_id, "webhook", "approvals"],
                version,
                &serde_json::json!({}),
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
        app_id: &str,
        paging: &Paging,
    ) -> Result<models::WebhookDeadLetterPage> {
        self.0
            .get_with(
                &["applications", app_id, "webhook", "dead-letters"],
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
        app_id: &str,
        request: &models::WebhookReplayRequest,
        mutation: &Mutation,
    ) -> Result<models::WebhookReplayResponse> {
        self.0
            .post(
                &["applications", app_id, "webhook", "dead-letters", "replays"],
                request,
                mutation,
            )
            .await
    }

    /// Logins performed through this application.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller cannot administer the application.
    pub async fn login_history(
        &self,
        app_id: &str,
        paging: &Paging,
    ) -> Result<models::LoginEventPage> {
        self.0
            .get_with(&["applications", app_id, "login-history"], &paging.query())
            .await
    }
}

#[cfg(test)]
mod tests {
    use crate::{Client, Error, Mutation};

    #[test]
    fn webhook_approval_accepts_a_canonical_application_identity() {
        let mut payload = serde_json::json!({
            "application_id": "billing",
            "active_url": null,
            "pending_url": "https://hooks.example.test/iam",
            "status": "pending_review",
            "secret_version": 1,
            "version": 1
        });
        let Ok(webhook) =
            serde_json::from_value::<crate::models::ApplicationWebhook>(payload.clone())
        else {
            panic!("the canonical application identity must decode for webhook approval");
        };
        assert_eq!(webhook.application_id.as_deref(), Some("billing"));
        payload["application_id"] = serde_json::Value::Null;
        let Ok(webhook) = serde_json::from_value::<crate::models::ApplicationWebhook>(payload)
        else {
            panic!("an undisclosed application identity must remain nullable");
        };
        assert_eq!(webhook.application_id, None);
    }

    #[tokio::test]
    async fn production_clients_refuse_the_test_only_import_before_sending() {
        let Ok(client) = Client::new("https://example.test") else {
            panic!("a valid client must build");
        };
        let error = client
            .applications()
            .import_from_production("billing", &Mutation::new())
            .await;
        assert!(
            matches!(error, Err(Error::Invalid(message)) if message.contains("only possible in a testing environment"))
        );
    }
}
