//! The endpoints an application calls as itself.
//!
//! All three authenticate with the application's own credential, so build the
//! client with [`Credential::application`](crate::Credential::application).
//! The browser-facing login screen is deliberately absent: it belongs to the
//! browser, not to an API caller.

use crate::{Client, Mutation, Result, models};

/// Token exchange, introspection, and revocation.
pub struct OAuth<'a>(pub(super) &'a Client);

impl OAuth<'_> {
    /// Logs a principal in to this Application with a short-lived token.
    ///
    /// This is the only Application-login entry point. The Application never
    /// receives or submits the principal's OTP or any other authentication
    /// credential; IAM completes that ceremony and hands the Application the
    /// single-use `slt`. In a verified testing environment, `slt` may also be
    /// an existing Carbon ID (`alice`) or Silicon ID (`worker:tos`). This test
    /// shortcut selects the actor's current active organizations and the
    /// Application's approved scopes; production accepts only issued codes.
    ///
    /// # Errors
    ///
    /// Returns an error when the short-lived token is invalid, expired, spent,
    /// or was issued for a different Application.
    pub async fn login(
        &self,
        app_id: &str,
        slt: &str,
        mutation: &Mutation,
    ) -> Result<models::OAuthTokenResponse> {
        self.exchange(&application_login_request(app_id, slt), mutation)
            .await
    }

    /// Rotates an Application refresh token after a successful login.
    ///
    /// Refresh is deliberately separate from [`Self::login`]: a refresh token
    /// can continue an existing session, but it cannot begin an Application
    /// login and is never accepted in the login method.
    ///
    /// # Errors
    ///
    /// Returns an error when the refresh token is invalid, expired, spent, or
    /// belongs to a different Application.
    pub async fn refresh(
        &self,
        app_id: &str,
        refresh_token: &str,
        mutation: &Mutation,
    ) -> Result<models::OAuthTokenResponse> {
        self.exchange(
            &application_refresh_request(app_id, refresh_token),
            mutation,
        )
        .await
    }

    /// Asks the service what a token currently authorizes.
    ///
    /// Authoritative and live: it reflects revocation immediately, which is
    /// why a consumer should introspect rather than trust a cached claim. When
    /// supplied, `org_context` must be one valid organization handle and an
    /// exact match for the token; a mismatch answers with `active: false`.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails or `org_context` is malformed.
    /// An unknown, revoked, or organization-mismatched token is not an error --
    /// it answers with `active: false`.
    pub async fn introspect(
        &self,
        request: &models::TokenIntrospectionRequest,
        org_context: Option<&str>,
    ) -> Result<models::TokenIntrospection> {
        let mut built = self
            .0
            .route(reqwest::Method::POST, &["oauth", "introspect"])?;
        if let Some(org_id) = org_context {
            built = built.header("x-org-id", org_id);
        }
        self.0.send_json(built.form(request)).await
    }

    /// Fetches the live authorization snapshot for one organization.
    ///
    /// Call immediately after login, or to rebuild an empty local projection.
    /// No directory edit or webhook arrival is necessary. An organization-bound
    /// token answers for the organization it is bound to and `org_context`, when
    /// given, must name that one. An unscoped token reaches only selected active organizations, so `org_context` picks which one to answer for
    /// and is required: use [`authorizations`](Self::authorizations) to see them
    /// all. `None` means there is no current organization authority (inactive,
    /// wrong application or organization, refresh token, or an unscoped token
    /// with no organization named). Undisclosed roles or tags remain `None`;
    /// never turn them into extra privileges.
    ///
    /// # Errors
    ///
    /// Returns an error when introspection fails or its response is malformed.
    pub async fn authorization(
        &self,
        access_token: &str,
        org_context: Option<&str>,
    ) -> Result<Option<models::ApplicationAuthorization>> {
        let inspected = self
            .introspect(
                &models::TokenIntrospectionRequest {
                    token: access_token.to_owned(),
                    token_type_hint: Some(
                        models::TokenIntrospectionRequestTokenTypeHint::AccessToken,
                    ),
                },
                org_context,
            )
            .await?;
        if inspected.active
            && access_token.starts_with("oat_")
            && inspected.membership_id.is_some()
            && inspected.authorization.is_none()
        {
            return Err(crate::Error::Decode(
                "IAM returned an active organization-bound access token without its authorization snapshot; deploy an API version supporting authorization snapshots before using this method".to_owned(),
            ));
        }
        Ok(if inspected.active {
            inspected.authorization
        } else {
            None
        })
    }

    /// Fetches one live authorization snapshot per organization the token
    /// reaches.
    ///
    /// An organization-bound token yields exactly one. An unscoped token yields
    /// one per selected organization with active membership, in handle
    /// order, and an empty vector when the subject has no currently selected active memberships -- which is not the same as an inactive token. `None` means the
    /// token is inactive or is not an Application access token.
    ///
    /// # Errors
    ///
    /// Returns an error when introspection fails, its response is malformed, or
    /// the service predates unscoped organization listing.
    pub async fn authorizations(
        &self,
        access_token: &str,
    ) -> Result<Option<Vec<models::ApplicationAuthorization>>> {
        let inspected = self
            .introspect(
                &models::TokenIntrospectionRequest {
                    token: access_token.to_owned(),
                    token_type_hint: Some(
                        models::TokenIntrospectionRequestTokenTypeHint::AccessToken,
                    ),
                },
                None,
            )
            .await?;
        if !inspected.active {
            return Ok(None);
        }
        if let Some(authorization) = inspected.authorization {
            return Ok(Some(vec![authorization]));
        }
        if let Some(authorizations) = inspected.authorizations {
            return Ok(Some(authorizations));
        }
        if access_token.starts_with("oat_") {
            return Err(crate::Error::Decode(
                "IAM returned an active Application access token without any authorization snapshot; deploy an API version supporting selected-organization authorization snapshots before using this method".to_owned(),
            ));
        }
        Ok(None)
    }

    /// Revokes one access token, or the complete family of a refresh token.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails. Revoking an unknown token is
    /// deliberately not an error.
    pub async fn revoke(
        &self,
        request: &models::OAuthRevocationRequest,
        mutation: &Mutation,
    ) -> Result<()> {
        let request = mutation
            .apply(self.0.route(reqwest::Method::POST, &["oauth", "revoke"])?)
            .form(request);
        self.0.send_empty(request).await
    }

    async fn exchange(
        &self,
        request: &models::ApplicationTokenRequest,
        mutation: &Mutation,
    ) -> Result<models::OAuthTokenResponse> {
        let request = mutation
            .apply(
                self.0
                    .route(reqwest::Method::POST, &["app-auth", "tokens"])?,
            )
            .form(request);
        self.0.send_json(request).await
    }
}

fn application_login_request(app_id: &str, slt: &str) -> models::ApplicationTokenRequest {
    models::ApplicationTokenRequest {
        app_id: app_id.to_owned(),
        slt: Some(slt.to_owned()),
        refresh_token: None,
    }
}

fn application_refresh_request(
    app_id: &str,
    refresh_token: &str,
) -> models::ApplicationTokenRequest {
    models::ApplicationTokenRequest {
        app_id: app_id.to_owned(),
        slt: None,
        refresh_token: Some(refresh_token.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::{application_login_request, application_refresh_request};

    #[test]
    fn application_login_can_only_submit_an_slt() {
        let request = application_login_request("checkout", "slt_example");
        assert_eq!(request.app_id, "checkout");
        assert_eq!(request.slt.as_deref(), Some("slt_example"));
        assert_eq!(request.refresh_token, None);
    }

    #[test]
    fn application_refresh_cannot_begin_a_login() {
        let request = application_refresh_request("checkout", "ort_example");
        assert_eq!(request.app_id, "checkout");
        assert_eq!(request.slt, None);
        assert_eq!(request.refresh_token.as_deref(), Some("ort_example"));
    }
}
