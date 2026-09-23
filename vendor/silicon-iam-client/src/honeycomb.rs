//! Dedicated Honeycomb service integration. Keep this client on the trusted
//! backend: it cannot be constructed from an ordinary IAM application secret.
//! Requests are stateless; retain operation IDs, bodies and mutation keys for retries.
mod contracts;
pub use contracts::ManagementAuthority;

use crate::{Client, Credential, Error, Mutation, Result, models};
use reqwest::Method;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use uuid::Uuid;

/// Server-only client with a separate provisioned service credential.
#[derive(Clone)]
pub struct ManagementClient(Client);
impl std::fmt::Debug for ManagementClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagementClient(<redacted>)")
    }
}
macro_rules! application_mutation {
    ($method:ident,$schema:ident,$http:ident,$suffix:literal,$doc:literal) => {
        #[doc=$doc]
        /// # Errors
        /// Returns IAM's structured authority, step-up, revision or validation error.
        pub async fn $method(
            &self,
            app_id: &str,
            actor: &SecretString,
            input: &models::$schema,
            mutation: &Mutation,
        ) -> Result<models::HoneycombReceipt> {
            self.mutate(
                Method::$http,
                &["honeycomb", "applications", app_id, $suffix],
                Some(actor),
                input,
                mutation,
            )
            .await
        }
    };
}
impl ManagementClient {
    /// Constructs a production control-plane client with a random `hck_` credential.
    /// No testing header or automatic updater is installed.
    /// # Errors
    /// Rejects ordinary app/user credentials and unsafe base URLs.
    pub fn new(base_url: &str, credential: SecretString) -> Result<Self> {
        let valid = credential
            .expose_secret()
            .strip_prefix("hck_")
            .is_some_and(|secret| {
                secret.len() == 43
                    && secret
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            });
        if !valid {
            return Err(Error::Invalid(
                "a dedicated Honeycomb hck_ service credential is required".into(),
            ));
        }
        Ok(Self(
            Client::builder(base_url)?
                .credential(Credential::Bearer(credential))
                .telemetry(false)
                .build()?,
        ))
    }
    async fn mutate<T: Serialize>(
        &self,
        method: Method,
        path: &[&str],
        actor: Option<&SecretString>,
        input: &T,
        mutation: &Mutation,
    ) -> Result<models::HoneycombReceipt> {
        let mut request = mutation.apply(self.0.route(method, path)?).json(input);
        if let Some(actor) = actor {
            if !actor.expose_secret().starts_with("oat_") {
                return Err(Error::Invalid(
                    "the actor must be an IAM token issued to Honeycomb".into(),
                ));
            }
            let mut value = reqwest::header::HeaderValue::from_str(actor.expose_secret())
                .map_err(|_| Error::Invalid("invalid actor token".into()))?;
            value.set_sensitive(true);
            request = request.header("x-honeycomb-actor-token", value);
        }
        self.0.send_json(request).await
    }
    application_mutation!(
        configure_application,
        HoneycombConfiguration,
        PUT,
        "configuration",
        "Submits an accepted app configuration, independently of its release version."
    );
    application_mutation!(
        rotate_secret,
        HoneycombSecretRotation,
        POST,
        "secret-rotations",
        "Rotates a credential once. Requires `application.client_secret.rotate` step-up."
    );
    application_mutation!(
        decide_scopes,
        HoneycombScopeDecision,
        POST,
        "scope-decisions",
        "Records an exact provider or IAM scope decision using the live reviewer's token."
    );
    application_mutation!(
        approve_webhook,
        HoneycombWebhookApproval,
        POST,
        "webhook-approvals",
        "Activates a pending destination with `application.webhook.approve` step-up."
    );

    application_mutation!(
        rotate_webhook_secret,
        HoneycombWebhookSecretRotation,
        POST,
        "webhook-secret-rotations",
        "Rotates webhook signing material with `application.webhook_secret.rotate` step-up."
    );

    /// Reconciles the current accepted authentication record without an actor token.
    /// # Errors
    /// Returns IAM's structured service-authentication or not-found error.
    pub async fn application(&self, id: &str) -> Result<models::HoneycombRecord> {
        self.0.get(&["honeycomb", "applications", id]).await
    }
    /// Reads a durable operation status; credentials are never returned here.
    /// # Errors
    /// Returns IAM's structured authentication or not-found error.
    pub async fn operation(&self, id: Uuid) -> Result<models::HoneycombRecord> {
        self.0
            .get(&["honeycomb", "operations", &id.to_string()])
            .await
    }
    /// Reads IAM scope definitions and reviewer requirements.
    /// # Errors
    /// Returns IAM's structured service-authentication error.
    pub async fn scope_catalog(&self) -> Result<models::HoneycombRecord> {
        self.0.get(&["honeycomb", "scope-catalog"]).await
    }
    /// Reads scope eligibility for an organization, optionally filtered to a provider.
    /// # Errors
    /// Returns IAM's structured validation or service-authentication error.
    pub async fn scope_catalog_for(
        &self,
        org_id: &str,
        app_id: Option<&str>,
    ) -> Result<models::HoneycombRecord> {
        let mut query = vec![("org_id", org_id.to_owned())];
        if let Some(app_id) = app_id {
            query.push(("app_id", app_id.to_owned()));
        }
        self.0
            .get_with(&["honeycomb", "scope-catalog"], &query)
            .await
    }
    /// Enumerates secret-free resource IDs; pass `next_after` to continue.
    /// # Errors
    /// Returns IAM's structured validation or service-authentication error.
    pub async fn inventory(
        &self,
        kind: &str,
        after: Option<Uuid>,
    ) -> Result<models::HoneycombRecord> {
        let mut query = vec![("kind", kind.to_owned())];
        if let Some(id) = after {
            query.push(("after", id.to_string()));
        }
        self.0.get_with(&["honeycomb", "inventory"], &query).await
    }
    /// Reads accepted bundle configuration, including deletion state.
    /// # Errors
    /// Returns IAM's structured authentication or not-found error.
    pub async fn bundle(&self, id: &str) -> Result<models::HoneycombRecord> {
        self.0.get(&["honeycomb", "bundles", id]).await
    }
    /// Configures bundle membership after IAM checks ownership and bundle eligibility.
    /// # Errors
    /// Returns IAM's structured permission, revision or eligibility error.
    pub async fn configure_bundle(
        &self,
        id: &str,
        actor: &SecretString,
        input: &models::HoneycombBundleConfiguration,
        mutation: &Mutation,
    ) -> Result<models::HoneycombReceipt> {
        self.mutate(
            Method::PUT,
            &["honeycomb", "bundles", id, "configuration"],
            Some(actor),
            input,
            mutation,
        )
        .await
    }
    /// Sends an IAM-local test instruction. Omit actor only for explicitly authorized maintenance.
    /// # Errors
    /// Returns IAM's structured permission, generation, revision or pending-state error.
    pub async fn testing_instruction(
        &self,
        actor: Option<&SecretString>,
        input: &models::HoneycombTestingInstruction,
        mutation: &Mutation,
    ) -> Result<models::HoneycombReceipt> {
        self.mutate(
            Method::POST,
            &[
                "honeycomb",
                "testing-environments",
                &input.environment_id.to_string(),
                "operations",
            ],
            actor,
            input,
            mutation,
        )
        .await
    }
    /// Reads IAM-local environment progress without depending on test sessions.
    /// # Errors
    /// Returns IAM's structured service-authentication or not-found error.
    pub async fn testing_environment(&self, id: Uuid) -> Result<models::HoneycombRecord> {
        self.0
            .get(&["honeycomb", "testing-environments", &id.to_string()])
            .await
    }
    /// Reads archived management events. Reconcile current records during concurrent writes.
    /// # Errors
    /// Returns IAM's structured service-authentication error.
    pub async fn events(&self, after: Option<Uuid>) -> Result<models::HoneycombRecord> {
        let query = after
            .map(|id| vec![("after", id.to_string())])
            .unwrap_or_default();
        self.0.get_with(&["honeycomb", "events"], &query).await
    }
    /// Queues the same stable event for signed delivery again.
    /// # Errors
    /// Returns IAM's structured service-authentication or not-found error.
    pub async fn replay_event(&self, id: Uuid) -> Result<models::HoneycombRecord> {
        self.0
            .send_json(self.0.route(
                Method::POST,
                &["honeycomb", "events", &id.to_string(), "replay"],
            )?)
            .await
    }
}

/// Verifies the timestamp and HMAC over the complete, unmodified notification body.
/// Deduplicate the authenticated event ID and apply only newer resource revisions.
/// # Errors
/// Rejects malformed, expired, future-dated or incorrectly signed notifications.
pub fn verify_notification(
    key: &SecretString,
    header: &str,
    body: &[u8],
    now_unix_seconds: i64,
    tolerance: std::time::Duration,
) -> Result<()> {
    use hmac::{Hmac, Mac as _};
    let invalid = || Error::Invalid("invalid management notification signature".into());
    let (timestamp, signature) = header
        .strip_prefix("t=")
        .and_then(|value| value.split_once(",v1="))
        .ok_or_else(invalid)?;
    let timestamp: i64 = timestamp.parse().map_err(|_| invalid())?;
    if timestamp > now_unix_seconds.saturating_add(5)
        || now_unix_seconds.saturating_sub(timestamp)
            > i64::try_from(tolerance.as_secs()).unwrap_or(i64::MAX)
    {
        return Err(invalid());
    }
    let supplied = hex::decode(signature).map_err(|_| invalid())?;
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key.expose_secret().as_bytes())
        .map_err(|_| invalid())?;
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    mac.verify_slice(&supplied).map_err(|_| invalid())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_credentials_are_rejected_and_debug_redacts_service_authority() -> Result<()> {
        for prefix in ["ask_", "oat_", "cat_"] {
            assert!(
                ManagementClient::new(
                    "https://iam.test",
                    format!("{prefix}{}", "a".repeat(43)).into()
                )
                .is_err()
            );
        }
        let secret = format!("hck_{}", "a".repeat(43));
        let client = ManagementClient::new("https://iam.test", secret.clone().into())?;
        assert!(!format!("{client:?}").contains(&secret));
        Ok(())
    }
    #[test]
    fn notification_verification_rejects_tampering_and_replay_outside_window() -> Result<()> {
        use hmac::{Hmac, Mac as _};
        let key: SecretString = "independent-notification-key".into();
        let body = br#"{"event_id":"stable-id","revision":2}"#;
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key.expose_secret().as_bytes())
            .map_err(|_| Error::Invalid("key".into()))?;
        mac.update(b"100.");
        mac.update(body);
        let header = format!("t=100,v1={}", hex::encode(mac.finalize().into_bytes()));
        let tolerance = std::time::Duration::from_secs(30);
        verify_notification(&key, &header, body, 110, tolerance)?;
        assert!(verify_notification(&key, &header, b"changed", 110, tolerance).is_err());
        assert!(verify_notification(&key, &header, body, 131, tolerance).is_err());
        assert!(verify_notification(&key, &header, body, 90, tolerance).is_err());
        Ok(())
    }
}
