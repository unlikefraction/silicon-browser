//! Separately consented, reusable delegated access between applications.
//!
//! An application requests endpoint approval when needed. The represented user
//! reviews the complete dependency graph in IAM, then the application exchanges
//! the authorization code for one access/refresh pair per approved root endpoint.
//! Receivers verify access tokens on every request without consuming them.

use uuid::Uuid;

use crate::{Client, Mutation, Paging, Result, models};

/// OBO authorization, tokens and user-owned grants.
pub struct Obo<'a>(pub(super) &'a Client);

impl Obo<'_> {
    /// Discovers a permitted application's registered endpoints and dependencies.
    ///
    /// # Errors
    /// Returns an error when the target is unavailable in the selected environment.
    pub async fn endpoints(&self, app_id: &str) -> Result<models::OboEndpointCatalog> {
        self.0
            .get(&["obo-access", "applications", app_id, "endpoints"])
            .await
    }

    /// Starts separate endpoint consent using the requesting app's credentials.
    /// This creates a pending request, not endpoint authority. Show its IAM URL.
    ///
    /// # Errors
    /// Fails for an invalid user token, organization, scope or dependency graph.
    pub async fn authorize(
        &self,
        request: &models::OboAuthorizationRequest,
        mutation: &Mutation,
    ) -> Result<models::OboConsentDetail> {
        self.0
            .post(&["obo-access", "authorizations"], request, mutation)
            .await
    }

    /// Reads authorization status using the creating application's credentials.
    ///
    /// # Errors
    /// Fails when the request is unavailable to this application.
    pub async fn authorization(&self, id: Uuid) -> Result<models::OboConsentDetail> {
        self.0
            .get(&["obo-access", "authorizations", &id.to_string()])
            .await
    }

    /// Reads the full consent graph using the represented user's direct IAM session.
    /// Application credentials and third-party app bearers cannot approve consent.
    ///
    /// # Errors
    /// Fails when the request does not belong to the directly authenticated user.
    pub async fn consent(&self, id: Uuid) -> Result<models::OboConsentDetail> {
        self.0
            .get(&["obo-access", "consents", &id.to_string()])
            .await
    }

    /// Approves the exact displayed graph version or declines it as the IAM user.
    /// Approval returns a short-lived code to hand to the requesting application.
    ///
    /// # Errors
    /// Fails for stale consent, changed dependencies or the wrong user.
    pub async fn decide(
        &self,
        id: Uuid,
        request: &models::OboConsentDecision,
        mutation: &Mutation,
    ) -> Result<models::OboConsentDecisionResult> {
        self.0
            .post(
                &["obo-access", "consents", &id.to_string(), "decision"],
                request,
                mutation,
            )
            .await
    }

    /// Redeems a single-use code for one pair per approved root endpoint.
    /// Persist the mutation key and reuse it after an uncertain identical retry.
    ///
    /// # Errors
    /// Fails for expired/used codes, wrong app credentials or revoked authority.
    pub async fn exchange_code(
        &self,
        authorization_id: Uuid,
        authorization_code: &str,
        mutation: &Mutation,
    ) -> Result<models::OboTokenResponse> {
        self.0
            .post(
                &["obo-access", "tokens"],
                &models::OboTokenRequest {
                    grant_id: None,
                    subject_token: None,
                    authorization_id: Some(authorization_id),
                    authorization_code: Some(authorization_code.to_owned()),
                    refresh_token: None,
                },
                mutation,
            )
            .await
    }

    /// Rotates a root refresh token while retaining its exact approved grant.
    /// Only the owning app may refresh. Keep one refresh in flight per family;
    /// reuse the same mutation for retries rather than reusing a rotated token.
    ///
    /// # Errors
    /// Fails for revoked/expired authority or refresh-token reuse.
    pub async fn refresh(
        &self,
        refresh_token: &str,
        mutation: &Mutation,
    ) -> Result<models::OboTokenResponse> {
        self.0
            .post(
                &["obo-access", "tokens"],
                &models::OboTokenRequest {
                    grant_id: None,
                    subject_token: None,
                    authorization_id: None,
                    authorization_code: None,
                    refresh_token: Some(refresh_token.to_owned()),
                },
                mutation,
            )
            .await
    }

    /// Restores credentials for an already approved exact graph using a fresh app login.
    /// A reset of another selected account requires that account to authenticate again.
    ///
    /// # Errors
    /// Fails for revoked grants, changed authority, or a login from another account/app.
    pub async fn recover(
        &self,
        grant_id: Uuid,
        subject_token: &str,
        mutation: &Mutation,
    ) -> Result<models::OboTokenResponse> {
        self.0
            .post(
                &["obo-access", "tokens"],
                &models::OboTokenRequest {
                    grant_id: Some(grant_id),
                    subject_token: Some(subject_token.to_owned()),
                    authorization_id: None,
                    authorization_code: None,
                    refresh_token: None,
                },
                mutation,
            )
            .await
    }

    /// Verifies an access token as its intended receiving application.
    /// This does not consume the token: verification can be repeated. The
    /// receiver still validates payload, metadata and resource permissions.
    ///
    /// # Errors
    /// Fails for a wrong audience/endpoint/path or inactive authority.
    pub async fn verify(
        &self,
        request: &models::OboTokenVerificationRequest,
    ) -> Result<models::OboTokenVerification> {
        let built = self
            .0
            .route(
                reqwest::Method::POST,
                &["obo-access", "token-verifications"],
            )?
            .json(request);
        self.0.send_json(built).await
    }

    /// Compatibility helper for a declared downstream endpoint.
    /// Returns the same reusable graph token. New callers may forward that token
    /// directly; each recipient authenticates itself when verifying its endpoint.
    ///
    /// # Errors
    /// Fails for undeclared edges, cycles, excess depth or revoked lineage.
    pub async fn delegate(
        &self,
        request: &models::OboDelegationRequest,
        mutation: &Mutation,
    ) -> Result<models::OboAccessToken> {
        self.0
            .post(&["obo-access", "delegations"], request, mutation)
            .await
    }

    /// Lists the first page of the directly authenticated user's OBO grants.
    /// The service returns at most 10 grants, most recent first.
    ///
    /// # Errors
    /// Fails unless authenticated with a direct IAM user session.
    pub async fn grants(&self) -> Result<models::OboGrants> {
        self.grants_page(&Paging::new()).await
    }

    /// Lists a page of user-owned grants with an opaque continuation cursor.
    /// The page size defaults to 10 and is bounded to 1 through 10.
    ///
    /// # Errors
    /// Fails unless directly authenticated as an IAM user or if the cursor is invalid.
    pub async fn grants_page(&self, paging: &Paging) -> Result<models::OboGrants> {
        self.0
            .get_with(&["obo-access", "grants"], &paging.query())
            .await
    }

    /// Lists this account's grants for one originating application, filtered before pagination.
    ///
    /// # Errors
    /// Fails unless directly authenticated or when the app ID/cursor is invalid.
    pub async fn grants_for_app(&self, app_id: &str, paging: &Paging) -> Result<models::OboGrants> {
        let mut query = paging.query();
        query.push(("app_id", app_id.to_owned()));
        self.0.get_with(&["obo-access", "grants"], &query).await
    }

    /// Revokes a user-owned grant, its token families and all descendants.
    ///
    /// # Errors
    /// Fails when the grant is unavailable to the directly authenticated user.
    pub async fn revoke(&self, id: Uuid, mutation: &Mutation) -> Result<models::OboGrantRevoked> {
        self.0
            .post(
                &["obo-access", "grants", &id.to_string(), "revoke"],
                &serde_json::json!({}),
                mutation,
            )
            .await
    }
}
