//! Testing environments: disposable replicas of the whole service.
//!
//! Individual environment lifecycle methods also accept the production
//! application credential when that application created the environment.
//! App clients create/list via `applications()`; imported dependencies cannot
//! manage another application's control record.
//!
//! An environment is this same API against a separate database, starting
//! empty. Manage them here, then move a client onto one with
//! [`Client::with_environment`] and use every
//! other method exactly as you would against production.
//!
//! Two of these routes are authorized by the environment key alone rather than
//! by an IAM credential, and are named `current_*` because they act on
//! whichever environment the client is currently inside.

use uuid::Uuid;

use crate::{Client, Mutation, Paging, Result, models};

/// Testing environment lifecycle.
pub struct Environments<'a>(pub(super) &'a Client);

impl Environments<'_> {
    /// The organization's environments.
    ///
    /// `status` accepts `active`, `deleted`, or `all`; deleted environments
    /// are hidden by default.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller is not a member, or the deployment has
    /// no testing database.
    pub async fn list(
        &self,
        org_id: &str,
        status: Option<&str>,
        paging: &Paging,
    ) -> Result<models::TestingEnvironmentPage> {
        let mut query = paging.query();
        if let Some(status) = status {
            query.push(("status", status.to_owned()));
        }
        self.0
            .get_with(&["organizations", org_id, "testing-environments"], &query)
            .await
    }

    /// Creates an empty environment, returning its key.
    ///
    /// Any active member may create one and becomes its creator, keeping
    /// administrative authority over it. The key stays retrievable, so losing
    /// this response is not losing the environment.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is taken or the organization is at its
    /// environment limit.
    pub async fn create(
        &self,
        org_id: &str,
        input: &models::TestingEnvironmentCreate,
        mutation: &Mutation,
    ) -> Result<models::TestingEnvironmentWithKey> {
        self.0
            .post(
                &["organizations", org_id, "testing-environments"],
                input,
                mutation,
            )
            .await
    }

    /// One environment.
    ///
    /// # Errors
    ///
    /// Returns an error when it does not exist in this organization.
    pub async fn get(
        &self,
        org_id: &str,
        environment_id: Uuid,
    ) -> Result<models::TestingEnvironment> {
        self.0
            .get(&[
                "organizations",
                org_id,
                "testing-environments",
                &environment_id.to_string(),
            ])
            .await
    }

    /// Renames or re-describes an environment.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale or the caller does not
    /// administer the environment.
    pub async fn update(
        &self,
        org_id: &str,
        environment_id: Uuid,
        version: i64,
        patch: &models::TestingEnvironmentPatch,
        mutation: &Mutation,
    ) -> Result<models::TestingEnvironment> {
        let built = mutation
            .apply(self.0.route(
                reqwest::Method::PATCH,
                &[
                    "organizations",
                    org_id,
                    "testing-environments",
                    &environment_id.to_string(),
                ],
            )?)
            .header(reqwest::header::IF_MATCH, format!("\"{version}\""))
            .json(patch);
        self.0.send_json(built).await
    }

    /// Retires an environment, keeping it recoverable until `purge_after`.
    ///
    /// Nothing is erased yet, and the key stops working immediately.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller does not administer the environment.
    pub async fn delete(
        &self,
        org_id: &str,
        environment_id: Uuid,
        mutation: &Mutation,
    ) -> Result<models::TestingEnvironment> {
        let built = mutation.apply(self.0.route(
            reqwest::Method::DELETE,
            &[
                "organizations",
                org_id,
                "testing-environments",
                &environment_id.to_string(),
            ],
        )?);
        self.0.send_json(built).await
    }

    /// Brings a retired environment back before its deadline passes.
    ///
    /// # Errors
    ///
    /// Returns an error when the deadline has passed, or another environment
    /// has taken the name in the meantime.
    pub async fn restore(
        &self,
        org_id: &str,
        environment_id: Uuid,
        mutation: &Mutation,
    ) -> Result<models::TestingEnvironment> {
        self.0
            .post(
                &[
                    "organizations",
                    org_id,
                    "testing-environments",
                    &environment_id.to_string(),
                    "restorations",
                ],
                &serde_json::json!({}),
                mutation,
            )
            .await
    }

    /// Reads the environment key back.
    ///
    /// Restricted to the environment's administrators, and audited on every
    /// read.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller does not administer the environment.
    pub async fn key(
        &self,
        org_id: &str,
        environment_id: Uuid,
    ) -> Result<models::TestingEnvironmentKey> {
        self.0
            .get(&[
                "organizations",
                org_id,
                "testing-environments",
                &environment_id.to_string(),
                "key",
            ])
            .await
    }

    /// Issues a new key, invalidating the previous one at once.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller does not administer the environment.
    pub async fn rotate_key(
        &self,
        org_id: &str,
        environment_id: Uuid,
        mutation: &Mutation,
    ) -> Result<models::TestingEnvironmentWithKey> {
        self.0
            .post(
                &[
                    "organizations",
                    org_id,
                    "testing-environments",
                    &environment_id.to_string(),
                    "key-rotations",
                ],
                &serde_json::json!({}),
                mutation,
            )
            .await
    }

    /// Erases everything inside an environment, keeping the environment.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller does not administer the environment.
    pub async fn clean(
        &self,
        org_id: &str,
        environment_id: Uuid,
        mutation: &Mutation,
    ) -> Result<models::TestingEnvironmentCleaning> {
        self.0
            .post(
                &[
                    "organizations",
                    org_id,
                    "testing-environments",
                    &environment_id.to_string(),
                    "cleanings",
                ],
                &serde_json::json!({}),
                mutation,
            )
            .await
    }

    /// Describes the environment this client is currently inside.
    ///
    /// Authorized by the environment key alone.
    ///
    /// # Errors
    ///
    /// Returns an error when the client carries no environment key.
    pub async fn current(&self) -> Result<models::TestingEnvironmentSelf> {
        self.0.get(&["testing-environment"]).await
    }

    /// Erases everything inside the environment this client is inside.
    ///
    /// Authorized by the environment key alone: the key is the environment's
    /// root authority, and the data is disposable by construction.
    ///
    /// # Errors
    ///
    /// Returns an error when the client carries no environment key.
    pub async fn clean_current(
        &self,
        mutation: &Mutation,
    ) -> Result<models::TestingEnvironmentCleaning> {
        self.0
            .post(
                &["testing-environment", "cleanings"],
                &serde_json::json!({}),
                mutation,
            )
            .await
    }
}
