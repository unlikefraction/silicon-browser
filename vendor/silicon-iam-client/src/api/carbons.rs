//! The signed-in Carbon, and looking other Carbons up.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Client, Mutation, Paging, Result, models};

/// The caller's own account, and Carbon lookup.
pub struct Carbons<'a>(pub(super) &'a Client);

/// Carbon IDs matching a search, at most ten of them.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CarbonSuggestions {
    /// The matches, closest first.
    pub items: Vec<models::CarbonSuggestion>,
}

impl Carbons<'_> {
    /// The signed-in Carbon's own profile, contacts included.
    ///
    /// # Errors
    ///
    /// Returns an error when the credential is not a direct Carbon token.
    pub async fn me(&self) -> Result<models::CarbonSelf> {
        self.0.get(&["me"]).await
    }

    /// Updates the signed-in Carbon's profile.
    ///
    /// A merge-patch: a field set to `None` is left alone, and a field set to
    /// `Some(None)` is cleared.
    ///
    /// # Errors
    ///
    /// Returns an error when `version` is stale, or a field is rejected.
    pub async fn update_me(
        &self,
        version: i64,
        patch: &models::CarbonProfilePatch,
        mutation: &Mutation,
    ) -> Result<models::CarbonSelf> {
        self.0.patch(&["me"], version, patch, mutation).await
    }

    /// The caller's active sessions, newest first.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails.
    pub async fn sessions(&self, paging: &Paging) -> Result<models::SessionPage> {
        self.0.get_with(&["me", "sessions"], &paging.query()).await
    }

    /// Revokes one of the caller's sessions.
    ///
    /// Requires a step-up assertion: ending a session is an authority change.
    ///
    /// # Errors
    ///
    /// Returns an error when the session is unknown or the step-up is missing.
    pub async fn revoke_session(&self, session: Uuid, mutation: &Mutation) -> Result<()> {
        self.0
            .delete(&["me", "sessions", &session.to_string()], None, mutation)
            .await
    }

    /// The caller's login history.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails.
    pub async fn login_history(&self, paging: &Paging) -> Result<models::LoginEventPage> {
        self.0
            .get_with(&["me", "login-history"], &paging.query())
            .await
    }

    /// Suggests Carbon IDs matching a partial handle.
    ///
    /// Returns handles only. `limit` is bounded by the service at ten.
    ///
    /// # Errors
    ///
    /// Returns an error when the query is empty or the request fails.
    pub async fn search(&self, query: &str, limit: Option<u16>) -> Result<CarbonSuggestions> {
        let mut parameters = vec![("q", query.to_owned())];
        if let Some(limit) = limit {
            parameters.push(("limit", limit.to_string()));
        }
        self.0.get_with(&["carbons", "search"], &parameters).await
    }

    /// Resolves an exact verified email address to a Carbon ID.
    ///
    /// Answers `404` when nothing matches, and never returns contact or
    /// profile data.
    ///
    /// # Errors
    ///
    /// Returns an error when nothing matches or the request fails.
    pub async fn resolve_email(&self, email: &str) -> Result<models::CarbonResolution> {
        let request = self
            .0
            .route(reqwest::Method::POST, &["carbons", "resolve", "email"])?
            .json(&models::EmailInput {
                email: email.to_owned(),
            });
        self.0.send_json(request).await
    }

    /// Resolves an exact verified phone number to a Carbon ID.
    ///
    /// # Errors
    ///
    /// Returns an error when nothing matches or the request fails.
    pub async fn resolve_phone(&self, phone_number: &str) -> Result<models::CarbonResolution> {
        let request = self
            .0
            .route(reqwest::Method::POST, &["carbons", "resolve", "phone"])?
            .json(&models::PhoneInput {
                phone_number: phone_number.to_owned(),
            });
        self.0.send_json(request).await
    }
}
