//! Advisory trust: the default, the rules, and what they evaluate to.
//!
//! Trust is reliable metadata a consumer may act on. It is never an
//! authorization decision by itself, and every evaluation says so.

use uuid::Uuid;

use crate::{Client, Mutation, Paging, Result, models};

/// Trust configuration and evaluation.
pub struct Trust<'a>(pub(super) &'a Client);

impl Trust<'_> {
    /// The organization-wide baseline.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails.
    pub async fn default(&self, org_id: &str) -> Result<models::TrustValue> {
        self.0
            .get(&["organizations", org_id, "trust", "default"])
            .await
    }

    /// Replaces the organization-wide baseline.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale or the caller lacks
    /// `trust.manage`.
    pub async fn replace_default(
        &self,
        org_id: &str,
        version: i64,
        value: &models::TrustValue,
        mutation: &Mutation,
    ) -> Result<models::TrustValue> {
        self.0
            .put(
                &["organizations", org_id, "trust", "default"],
                version,
                value,
                mutation,
            )
            .await
    }

    /// The organization's trust rules.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails.
    pub async fn list_rules(&self, org_id: &str, paging: &Paging) -> Result<models::TrustRulePage> {
        self.0
            .get_with(
                &["organizations", org_id, "trust", "rules"],
                &paging.query(),
            )
            .await
    }

    /// Creates a trust rule.
    ///
    /// # Errors
    ///
    /// Returns an error when a selector names something that does not exist.
    pub async fn create_rule(
        &self,
        org_id: &str,
        input: &models::TrustRuleCreate,
        mutation: &Mutation,
    ) -> Result<models::TrustRule> {
        self.0
            .post(
                &["organizations", org_id, "trust", "rules"],
                input,
                mutation,
            )
            .await
    }

    /// One trust rule.
    ///
    /// # Errors
    ///
    /// Returns an error when the rule does not exist or was archived.
    pub async fn get_rule(&self, org_id: &str, rule_id: Uuid) -> Result<models::TrustRule> {
        self.0
            .get(&[
                "organizations",
                org_id,
                "trust",
                "rules",
                &rule_id.to_string(),
            ])
            .await
    }

    /// Changes a rule's trust value. Its selectors are immutable.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale.
    pub async fn update_rule(
        &self,
        org_id: &str,
        rule_id: Uuid,
        version: i64,
        patch: &models::TrustRulePatch,
        mutation: &Mutation,
    ) -> Result<models::TrustRule> {
        self.0
            .patch(
                &[
                    "organizations",
                    org_id,
                    "trust",
                    "rules",
                    &rule_id.to_string(),
                ],
                version,
                patch,
                mutation,
            )
            .await
    }

    /// Archives a trust rule.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale.
    pub async fn delete_rule(
        &self,
        org_id: &str,
        rule_id: Uuid,
        version: i64,
        mutation: &Mutation,
    ) -> Result<()> {
        self.0
            .delete(
                &[
                    "organizations",
                    org_id,
                    "trust",
                    "rules",
                    &rule_id.to_string(),
                ],
                Some(version),
                mutation,
            )
            .await
    }

    /// Explains the trust between one subject and one target Silicon.
    ///
    /// Returns the value, the rules that produced it, and `advisory: true`.
    ///
    /// # Errors
    ///
    /// Returns an error when a selector names something that does not exist.
    pub async fn evaluate(
        &self,
        org_id: &str,
        request: &models::TrustEvaluationRequest,
    ) -> Result<models::TrustEvaluation> {
        let built = self
            .0
            .route(
                reqwest::Method::POST,
                &["organizations", org_id, "trust", "effective"],
            )?
            .json(request);
        self.0.send_json(built).await
    }
}
