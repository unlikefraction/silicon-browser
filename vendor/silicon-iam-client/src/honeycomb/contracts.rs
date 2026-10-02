//! Typed management contracts for publication and shared environments.
use super::ManagementClient;
use crate::{Error, Mutation, Result, models};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{Method, RequestBuilder, header::HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

/// Proof of the actor behind a management operation. Application credentials are
/// production credentials; the optional key identifies one attached environment.
pub enum ManagementAuthority<'a> {
    /// A current user token issued to the configured management application.
    Actor(&'a SecretString),
    /// The current root key of one environment, for import or key rotation.
    /// This does not grant access to private production applications.
    Environment(&'a crate::EnvironmentKey),
    /// Current production application credentials, optionally with attachment proof.
    Application {
        /// Immutable qualified application ID.
        app_id: &'a str,
        /// Current production secret; never a test secret.
        app_secret: &'a SecretString,
        /// The key of the one environment being attached or administered.
        environment_key: Option<&'a crate::EnvironmentKey>,
    },
}
impl std::fmt::Debug for ManagementAuthority<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagementAuthority(<redacted>)")
    }
}
fn sensitive(value: &str) -> Result<HeaderValue> {
    let mut value = HeaderValue::from_str(value)
        .map_err(|_| Error::Invalid("invalid management authority header".into()))?;
    value.set_sensitive(true);
    Ok(value)
}
impl ManagementAuthority<'_> {
    fn apply(&self, request: RequestBuilder) -> Result<RequestBuilder> {
        match self {
            Self::Actor(actor) => {
                if !actor.expose_secret().starts_with("oat_") {
                    return Err(Error::Invalid(
                        "a current management application user token is required".into(),
                    ));
                }
                Ok(request.header("x-honeycomb-actor-token", sensitive(actor.expose_secret())?))
            }
            Self::Environment(key) => {
                Ok(request.header("x-honeycomb-testing-key", sensitive(key.expose())?))
            }
            Self::Application {
                app_id,
                app_secret,
                environment_key,
            } => {
                if app_id.contains(':') || !app_secret.expose_secret().starts_with("ask_") {
                    return Err(Error::Invalid(
                        "production application credentials required".into(),
                    ));
                }
                let basic = format!(
                    "Basic {}",
                    STANDARD.encode(format!("{app_id}:{}", app_secret.expose_secret()))
                );
                let mut request =
                    request.header("x-honeycomb-application-authorization", sensitive(&basic)?);
                if let Some(key) = environment_key {
                    request = request.header("x-honeycomb-testing-key", sensitive(key.expose())?);
                }
                Ok(request)
            }
        }
    }
}
macro_rules! publication_mutation {
    ($method:ident,$input:ident,$output:ident,$suffix:literal,$doc:literal) => {
        #[doc=$doc]
        /// # Errors
        /// Returns current authority, exact-plan, revision or validation errors.
        pub async fn $method(
            &self,
            actor: &SecretString,
            input: &models::$input,
            mutation: &Mutation,
        ) -> Result<models::$output> {
            self.contract(
                Method::POST,
                &["honeycomb", "applications", &input.app_id, $suffix],
                Some(&ManagementAuthority::Actor(actor)),
                input,
                mutation,
            )
            .await
        }
    };
}
impl ManagementClient {
    async fn contract<T: Serialize, R: DeserializeOwned>(
        &self,
        method: Method,
        path: &[&str],
        authority: Option<&ManagementAuthority<'_>>,
        input: &T,
        mutation: &Mutation,
    ) -> Result<R> {
        let request = mutation.apply(self.0.route(method, path)?).json(input);
        let request = match authority {
            Some(authority) => authority.apply(request)?,
            None => request,
        };
        self.0.send_json(request).await
    }
    publication_mutation!(
        publication_plan,
        HoneycombPublicationPlan,
        HoneycombPublicationPlanRecord,
        "publication-plans",
        "Creates an immutable plan for one desired public configuration."
    );
    publication_mutation!(
        publication_decision,
        HoneycombPublicationDecision,
        HoneycombPublicationDecisionRecord,
        "publication-decisions",
        "Records a decision for an exact plan gate with current reviewer authority."
    );
    publication_mutation!(
        publication_activate,
        HoneycombPublicationActivation,
        HoneycombReceipt,
        "publication-activations",
        "Activates exactly the reviewed configuration after rechecking every gate."
    );

    /// Verifies production credentials without creating or selecting a test environment.
    /// # Errors
    /// Returns an authentication error for revoked or invalid app credentials.
    pub async fn application_identity(
        &self,
        app_id: &str,
        app_secret: &SecretString,
    ) -> Result<models::HoneycombApplicationIdentity> {
        let request = self
            .0
            .route(Method::GET, &["honeycomb", "application-identity"])?;
        self.0
            .send_json(
                ManagementAuthority::Application {
                    app_id,
                    app_secret,
                    environment_key: None,
                }
                .apply(request)?,
            )
            .await
    }
    /// Lists only environments owned by or already linked to the authenticated app.
    /// # Errors
    /// Returns current app authentication or pagination validation errors.
    pub async fn application_testing_environments(
        &self,
        app_id: &str,
        app_secret: &SecretString,
        cursor: Option<Uuid>,
        limit: Option<u16>,
        status: Option<&str>,
    ) -> Result<models::HoneycombApplicationTestingEnvironments> {
        let mut query = Vec::new();
        if let Some(cursor) = cursor {
            query.push(("cursor", cursor.to_string()));
        }
        if let Some(limit) = limit {
            query.push(("limit", limit.to_string()));
        }
        if let Some(status) = status {
            query.push(("status", status.to_owned()));
        }
        let request =
            self.0
                .route_with(Method::GET, &["honeycomb", "testing-environments"], &query)?;
        self.0
            .send_json(
                ManagementAuthority::Application {
                    app_id,
                    app_secret,
                    environment_key: None,
                }
                .apply(request)?,
            )
            .await
    }

    /// Reads the immutable plan and its reused approval evidence.
    /// # Errors
    /// Returns service authentication or not-found errors.
    pub async fn read_publication_plan(
        &self,
        plan: Uuid,
    ) -> Result<models::HoneycombPublicationPlanRecord> {
        self.0
            .get(&["honeycomb", "publication-plans", &plan.to_string()])
            .await
    }
    /// Checks current actor eligibility for one exact plan gate.
    /// # Errors
    /// Returns current authentication or plan validation errors.
    pub async fn reviewer_eligibility(
        &self,
        plan: Uuid,
        provider: &str,
        actor: &SecretString,
    ) -> Result<models::HoneycombReviewerEligibility> {
        let request = self.0.route_with(
            Method::GET,
            &[
                "honeycomb",
                "publication-plans",
                &plan.to_string(),
                "reviewer-eligibility",
            ],
            &[("provider", provider.to_owned())],
        )?;
        self.0
            .send_json(ManagementAuthority::Actor(actor).apply(request)?)
            .await
    }
    /// Lists only recipients eligible for the selected publication plan and gate.
    /// # Errors
    /// Returns service authentication or exact-plan errors.
    pub async fn notification_recipients(
        &self,
        plan: Uuid,
        provider: &str,
        after: Option<&str>,
        limit: Option<u16>,
    ) -> Result<models::HoneycombNotificationRecipients> {
        let mut query = vec![("provider", provider.to_owned())];
        if let Some(after) = after {
            query.push(("after", after.to_string()));
        }
        if let Some(limit) = limit {
            query.push(("limit", limit.to_string()));
        }
        self.0
            .get_with(
                &[
                    "honeycomb",
                    "publication-plans",
                    &plan.to_string(),
                    "notification-recipients",
                ],
                &query,
            )
            .await
    }
    /// Lists only current owner/admin recipients for existing organization notices.
    /// # Errors
    /// Returns service authentication or organization validation errors.
    pub async fn organization_recipients(
        &self,
        org: &str,
        after: Option<&str>,
        limit: Option<u16>,
    ) -> Result<models::HoneycombOrganizationRecipients> {
        let mut query = Vec::new();
        if let Some(after) = after {
            query.push(("after", after.to_string()));
        }
        if let Some(limit) = limit {
            query.push(("limit", limit.to_string()));
        }
        self.0
            .get_with(
                &["honeycomb", "organizations", org, "notification-recipients"],
                &query,
            )
            .await
    }
    /// Sends a lifecycle instruction backed by a user or production app identity.
    /// # Errors
    /// Returns ownership, attachment, key-version, generation or revision errors.
    pub async fn testing_instruction_as(
        &self,
        authority: &ManagementAuthority<'_>,
        input: &models::HoneycombTestingInstruction,
        mutation: &Mutation,
    ) -> Result<models::HoneycombReceipt> {
        self.contract(
            Method::POST,
            &[
                "honeycomb",
                "testing-environments",
                &input.environment_id.to_string(),
                "operations",
            ],
            Some(authority),
            input,
            mutation,
        )
        .await
    }
    /// Transfers existing environment identity, ownership, links and its protected key.
    /// # Errors
    /// Returns service authority, revision, lifecycle or expired-secret-replay errors.
    pub async fn adoption_export(
        &self,
        environment: Uuid,
        input: &models::HoneycombAdoptionExport,
        mutation: &Mutation,
    ) -> Result<models::HoneycombReceipt> {
        self.contract(
            Method::POST,
            &[
                "honeycomb",
                "testing-environments",
                &environment.to_string(),
                "adoption-export",
            ],
            None,
            input,
            mutation,
        )
        .await
    }
    /// Retires only the exact listed applications from one environment.
    /// # Errors
    /// Returns service-maintenance authority, version or exact-app-set errors.
    pub async fn retain_testing_applications(
        &self,
        input: &models::HoneycombRetention,
        mutation: &Mutation,
    ) -> Result<models::HoneycombReceipt> {
        self.contract(
            Method::POST,
            &[
                "honeycomb",
                "testing-environments",
                &input.environment_id.to_string(),
                "retention",
            ],
            None,
            input,
            mutation,
        )
        .await
    }
    /// Reads the accepted isolated app record with explicit lifecycle versions.
    /// # Errors
    /// Returns service, environment state or version errors.
    pub async fn testing_application(
        &self,
        environment: Uuid,
        app: &str,
        version: &models::HoneycombTestingAppVersion,
    ) -> Result<models::HoneycombRecord> {
        self.0
            .get_with(
                &[
                    "honeycomb",
                    "testing-environments",
                    &environment.to_string(),
                    "applications",
                    app,
                ],
                &[
                    ("generation", version.generation.to_string()),
                    ("key_version", version.key_version.to_string()),
                    (
                        "expected_environment_revision",
                        version.expected_environment_revision.to_string(),
                    ),
                ],
            )
            .await
    }
    /// Configures or creates an isolated private app and leaves it awaiting activation.
    /// # Errors
    /// Returns current actor, ownership, isolation or revision errors.
    pub async fn configure_testing_application(
        &self,
        app: &str,
        authority: &ManagementAuthority<'_>,
        input: &models::HoneycombTestingAppMutation,
        mutation: &Mutation,
    ) -> Result<models::HoneycombReceipt> {
        self.contract(
            Method::PUT,
            &[
                "honeycomb",
                "testing-environments",
                &input.environment_id.to_string(),
                "applications",
                app,
                "configuration",
            ],
            Some(authority),
            input,
            mutation,
        )
        .await
    }
    /// Recovers only this immutable production application's linked test credential.
    /// # Errors
    /// Rejects foreign app identities, stale versions, retired imports and expired replay.
    pub async fn recover_testing_application_credential(
        &self,
        app: &str,
        app_secret: &SecretString,
        environment_key: Option<&crate::EnvironmentKey>,
        input: &models::HoneycombTestingCredentialRecovery,
        mutation: &Mutation,
    ) -> Result<models::HoneycombReceipt> {
        let authority = ManagementAuthority::Application {
            app_id: app,
            app_secret,
            environment_key,
        };
        self.contract(
            Method::POST,
            &[
                "honeycomb",
                "testing-environments",
                &input.environment_id.to_string(),
                "applications",
                app,
                "credential-recovery",
            ],
            Some(&authority),
            input,
            mutation,
        )
        .await
    }
    /// Rotates only the isolated app credential with durable ten-minute secret replay.
    /// # Errors
    /// Returns current actor, ownership, isolation or revision errors.
    pub async fn rotate_testing_application_secret(
        &self,
        app: &str,
        authority: &ManagementAuthority<'_>,
        input: &models::HoneycombTestingAppMutation,
        mutation: &Mutation,
    ) -> Result<models::HoneycombReceipt> {
        self.contract(
            Method::POST,
            &[
                "honeycomb",
                "testing-environments",
                &input.environment_id.to_string(),
                "applications",
                app,
                "secret-rotations",
            ],
            Some(authority),
            input,
            mutation,
        )
        .await
    }
}
