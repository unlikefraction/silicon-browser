//! Application-only delegation; credentials from this API never represent a user.
use crate::{Client, Mutation, Result};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// ATA credential exchange and recipient verification.
pub struct Ata<'a>(pub(super) &'a Client);
/// Reusable access token and its newly rotated origin-only refresh token.
#[derive(Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    /// Public access-token row identifier.
    pub token_id: Uuid,
    /// Managed verification identifier.
    pub verification_id: Uuid,
    /// Always Bearer.
    pub token_type: String,
    /// Reusable credential for every approved receiver in the graph.
    pub access_token: String,
    /// Newly rotated refresh credential; save atomically and keep private to the origin.
    pub refresh_token: String,
    /// Access-token expiration.
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    /// Access-token lifetime in seconds.
    pub expires_in: i64,
    /// Verification expiration, or none when it never expires.
    #[serde(with = "time::serde::rfc3339::option")]
    pub refresh_expires_at: Option<OffsetDateTime>,
}
/// Uniform ATA verification response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Verification {
    /// Whether the origin proof is valid for this receiver and endpoint.
    pub verified: bool,
    /// UTC expiry formatted numerically as YYYYMMDDHHMMSS, only for valid proofs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_till: Option<i64>,
}
impl Ata<'_> {
    /// Exchanges a refresh credential using the originating app's Basic credentials.
    /// Save the replacement before discarding the old token. Reuse the mutation key on retry.
    ///
    /// # Errors
    /// Fails when authority expired, changed, was revoked, or the token was reused.
    pub async fn refresh(&self, refresh_token: &str, mutation: &Mutation) -> Result<TokenResponse> {
        self.0
            .post(
                &["ata-access", "tokens"],
                &serde_json::json!({"refresh_token":refresh_token}),
                mutation,
            )
            .await
    }
    /// Verifies a proof using this receiving app's Basic credentials.
    ///
    /// # Errors
    /// Fails for recipient authentication or transport errors; invalid proofs return verified false.
    pub async fn verify(&self, origin: &str, proof: &str, endpoint: &str) -> Result<Verification> {
        let result: Verification=self.0.send_json(self.0.route(reqwest::Method::POST,&["ata-access","verify"])?
            .json(&serde_json::json!({"app_id":origin,"app_proof_token":proof,"endpoint":endpoint}))).await?;
        if result.verified != result.valid_till.is_some() {
            return Err(crate::Error::Decode(
                "Invalid ATA verification response".to_owned(),
            ));
        }
        Ok(result)
    }
    /// Discovers a provider's ATA endpoint definitions and dependencies.
    ///
    /// # Errors
    /// Fails when the provider is unavailable to this application.
    pub async fn endpoints(&self, app_id: &str) -> Result<serde_json::Value> {
        self.0
            .get(&["ata-access", "applications", app_id, "endpoints"])
            .await
    }
}
