//! Disposable testing-environment lifecycle and key-authorized self operations.
//!
//! Every mutation has a convenience method that generates a fresh idempotency
//! key and a `_with_key` variant for durable callers. Persist a caller-owned key
//! with the intended mutation before sending it, then reuse the exact same key
//! and request after an uncertain transport outcome.

use reqwest::Method;
use uuid::Uuid;

use crate::{
    client::{Client, IdempotencyKey, json_body},
    error::{Error, Result},
    models::{
        TestingEnvironment, TestingEnvironmentCleaning, TestingEnvironmentKey,
        TestingEnvironmentPage, TestingEnvironmentSelf, TestingEnvironmentWithKey,
    },
    requests::{TestingEnvironmentCreate, TestingEnvironmentIamPairing, TestingEnvironmentUpdate},
};

const TEST_ONLY: &str = "this action is only possible for a test environment";
const PRODUCTION_ONLY: &str =
    "testing-environment management is only possible from the production plane";

impl Client {
    /// Lists this organization's active or recoverable testing environments.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller is not an organization member.
    pub async fn testing_environments(
        &self,
        status: Option<&str>,
    ) -> Result<TestingEnvironmentPage> {
        let mut url = self.testing_environments_url()?;
        {
            let mut query = url.query_pairs_mut();
            if let Some(status) = status {
                query.append_pair("status", status);
            }
        }
        let request = self
            .request(Method::GET, url)
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }

    /// Creates an empty Briefcase environment coupled to the supplied IAM plane.
    ///
    /// The response returns a distinct Briefcase root key. Keep the UUID in
    /// normal metadata and the key in a secret store.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is taken, the deployment already has ten
    /// active environments, or the IAM key cannot be validated.
    pub async fn create_testing_environment(
        &self,
        input: &TestingEnvironmentCreate,
    ) -> Result<TestingEnvironmentWithKey> {
        let idempotency_key = IdempotencyKey::random();
        self.create_testing_environment_with_key(input, &idempotency_key)
            .await
    }

    /// Creates an environment using a caller-owned retry identity.
    ///
    /// Persist `idempotency_key` with `input` before the first attempt. Reuse
    /// both unchanged when the response may have been lost after the server
    /// committed the environment and generated its root key.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is taken, the deployment already has ten
    /// active environments, or the IAM credential cannot be validated.
    pub async fn create_testing_environment_with_key(
        &self,
        input: &TestingEnvironmentCreate,
        idempotency_key: &IdempotencyKey,
    ) -> Result<TestingEnvironmentWithKey> {
        let body = json_body(input)?;
        let request = self
            .request(Method::POST, self.testing_environments_url()?)
            .header("content-type", "application/json")
            .header("idempotency-key", idempotency_key.as_str())
            .body(body)
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }

    /// Reads one testing environment without disclosing its key.
    ///
    /// # Errors
    ///
    /// Returns an error when it does not exist or the caller cannot see it.
    pub async fn testing_environment(&self, id: Uuid) -> Result<TestingEnvironment> {
        let url = self.testing_environment_url(id, &[])?;
        self.receive_json(
            self.request(Method::GET, url)
                .timeout(self.request_timeout()),
        )
        .await
    }

    /// Renames or re-describes a live testing environment.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller cannot administer it or `version` is stale.
    pub async fn update_testing_environment(
        &self,
        id: Uuid,
        version: i64,
        update: &TestingEnvironmentUpdate,
    ) -> Result<TestingEnvironment> {
        let idempotency_key = IdempotencyKey::random();
        self.update_testing_environment_with_key(id, version, update, &idempotency_key)
            .await
    }

    /// Updates an environment using a caller-owned retry identity.
    ///
    /// Retry only with the same `version`, patch, and `idempotency_key`; this
    /// allows a lost successful response to replay instead of being mistaken
    /// for an optimistic-concurrency conflict.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller cannot administer it or `version` is stale.
    pub async fn update_testing_environment_with_key(
        &self,
        id: Uuid,
        version: i64,
        update: &TestingEnvironmentUpdate,
        idempotency_key: &IdempotencyKey,
    ) -> Result<TestingEnvironment> {
        let request = self
            .request(Method::PATCH, self.testing_environment_url(id, &[])?)
            .header("content-type", "application/merge-patch+json")
            .header("if-match", format!("\"{version}\""))
            .header("idempotency-key", idempotency_key.as_str())
            .body(json_body(update)?)
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }

    /// Retires an environment for its two-day recovery window.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller is not its creator or an administrator.
    pub async fn delete_testing_environment(&self, id: Uuid) -> Result<TestingEnvironment> {
        let idempotency_key = IdempotencyKey::random();
        self.delete_testing_environment_with_key(id, &idempotency_key)
            .await
    }

    /// Retires an environment using a caller-owned retry identity.
    ///
    /// Reuse the same environment UUID and `idempotency_key` after an uncertain
    /// result so a committed retirement is replayed rather than repeated.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller is not its creator or an administrator.
    pub async fn delete_testing_environment_with_key(
        &self,
        id: Uuid,
        idempotency_key: &IdempotencyKey,
    ) -> Result<TestingEnvironment> {
        let request = self
            .request(Method::DELETE, self.testing_environment_url(id, &[])?)
            .header("idempotency-key", idempotency_key.as_str())
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }

    /// Restores a retired environment before its purge deadline.
    ///
    /// # Errors
    ///
    /// Returns an error when recovery is no longer possible.
    pub async fn restore_testing_environment(&self, id: Uuid) -> Result<TestingEnvironmentWithKey> {
        let idempotency_key = IdempotencyKey::random();
        self.restore_testing_environment_with_key(id, &idempotency_key)
            .await
    }

    /// Restores an environment using a caller-owned retry identity.
    ///
    /// Persist this key before the attempt: a successful restoration generates
    /// a replacement root key that must be recovered by replaying the same
    /// request after an uncertain outcome.
    ///
    /// # Errors
    ///
    /// Returns an error when recovery is no longer possible.
    pub async fn restore_testing_environment_with_key(
        &self,
        id: Uuid,
        idempotency_key: &IdempotencyKey,
    ) -> Result<TestingEnvironmentWithKey> {
        self.environment_post_with_key(id, &["restorations"], idempotency_key)
            .await
    }

    /// Retrieves the current root key, an audited administrative operation.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller cannot administer the environment.
    pub async fn testing_environment_key(&self, id: Uuid) -> Result<TestingEnvironmentKey> {
        let url = self.testing_environment_url(id, &["key"])?;
        self.receive_json(
            self.request(Method::GET, url)
                .timeout(self.request_timeout()),
        )
        .await
    }

    /// Replaces the complete IAM pairing while preserving Briefcase data and
    /// the current Briefcase root key.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller cannot administer the environment or
    /// IAM refuses any member of the replacement credential set.
    pub async fn replace_testing_environment_iam_pairing(
        &self,
        id: Uuid,
        pairing: &TestingEnvironmentIamPairing,
    ) -> Result<TestingEnvironment> {
        let idempotency_key = IdempotencyKey::random();
        self.replace_testing_environment_iam_pairing_with_key(id, pairing, &idempotency_key)
            .await
    }

    /// Replaces the IAM pairing using a caller-owned retry identity.
    ///
    /// Persist the key with all four credentials before sending the request.
    /// A retry must use the exact same UUID, root key, Application ID, secret,
    /// and idempotency key.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller cannot administer the environment or
    /// IAM refuses any member of the replacement credential set.
    pub async fn replace_testing_environment_iam_pairing_with_key(
        &self,
        id: Uuid,
        pairing: &TestingEnvironmentIamPairing,
        idempotency_key: &IdempotencyKey,
    ) -> Result<TestingEnvironment> {
        let request = self
            .request(
                Method::POST,
                self.testing_environment_url(id, &["iam-pairings"])?,
            )
            .header("content-type", "application/json")
            .header("idempotency-key", idempotency_key.as_str())
            .body(json_body(pairing)?)
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }

    /// Erases an environment's contents through the production management plane.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller cannot administer the environment.
    pub async fn clean_testing_environment(&self, id: Uuid) -> Result<TestingEnvironmentCleaning> {
        let idempotency_key = IdempotencyKey::random();
        self.clean_testing_environment_with_key(id, &idempotency_key)
            .await
    }

    /// Cleans an environment using a caller-owned retry identity.
    ///
    /// Reuse the same key and environment UUID after an uncertain result so
    /// the original cleaning result is replayed.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller cannot administer the environment.
    pub async fn clean_testing_environment_with_key(
        &self,
        id: Uuid,
        idempotency_key: &IdempotencyKey,
    ) -> Result<TestingEnvironmentCleaning> {
        self.environment_post_with_key(id, &["cleanings"], idempotency_key)
            .await
    }

    /// Describes the environment selected by this client's root key.
    ///
    /// # Errors
    ///
    /// Fails locally when this client has no testing key.
    pub async fn current_testing_environment(&self) -> Result<TestingEnvironmentSelf> {
        self.require_testing_environment()?;
        let url = self.api_url(&["testing-environment"])?;
        let request = self
            .anonymous_request(Method::GET, url)
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }

    /// Erases the selected environment's contents using its root key alone.
    ///
    /// # Errors
    ///
    /// Fails locally when this client has no testing key.
    pub async fn clean_current_testing_environment(&self) -> Result<TestingEnvironmentCleaning> {
        let idempotency_key = IdempotencyKey::random();
        self.clean_current_testing_environment_with_key(&idempotency_key)
            .await
    }

    /// Cleans the selected environment using a caller-owned retry identity.
    ///
    /// Reuse the exact key after an uncertain response. The testing root key
    /// selects the plane; this separate key identifies the cleaning mutation.
    ///
    /// # Errors
    ///
    /// Fails locally when this client has no testing key.
    pub async fn clean_current_testing_environment_with_key(
        &self,
        idempotency_key: &IdempotencyKey,
    ) -> Result<TestingEnvironmentCleaning> {
        self.require_testing_environment()?;
        let url = self.api_url(&["testing-environment", "cleanings"])?;
        let request = self
            .anonymous_request(Method::POST, url)
            .header("content-type", "application/json")
            .header("idempotency-key", idempotency_key.as_str())
            .body("{}")
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }

    fn require_testing_environment(&self) -> Result<()> {
        if self.config().environment().is_none() {
            return Err(Error::Configuration(TEST_ONLY.to_owned()));
        }
        Ok(())
    }

    fn require_production_environment(&self) -> Result<()> {
        if self.config().environment().is_some() {
            return Err(Error::Configuration(PRODUCTION_ONLY.to_owned()));
        }
        Ok(())
    }

    fn testing_environments_url(&self) -> Result<url::Url> {
        self.require_production_environment()?;
        self.api_url(&["organizations", self.organization(), "testing-environments"])
    }

    fn testing_environment_url(&self, id: Uuid, suffix: &[&str]) -> Result<url::Url> {
        self.require_production_environment()?;
        let id = id.to_string();
        let mut segments = vec![
            "organizations",
            self.organization(),
            "testing-environments",
            id.as_str(),
        ];
        segments.extend_from_slice(suffix);
        self.api_url(&segments)
    }

    async fn environment_post_with_key<T: serde::de::DeserializeOwned>(
        &self,
        id: Uuid,
        suffix: &[&str],
        idempotency_key: &IdempotencyKey,
    ) -> Result<T> {
        let request = self
            .request(Method::POST, self.testing_environment_url(id, suffix)?)
            .header("content-type", "application/json")
            .header("idempotency-key", idempotency_key.as_str())
            .body("{}")
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }
}
