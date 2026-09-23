//! Short-lived application identity keys for inter-application calls.
//!
//! Both operations authenticate with [`Credential::application`](crate::Credential::application).
//! Verification uses the receiving application's credential, not the calling application's
//! secret. Configure [`Client::with_environment`] for isolated testing.

use crate::{Client, Error, Result, models};

/// Issues and checks application identity independently of user permissions and OBO.
pub struct AppVerification<'a>(pub(super) &'a Client);

impl AppVerification<'_> {
    /// Issues a fresh key for the authenticated application.
    ///
    /// An omitted lifetime defaults to 300 seconds; accepted lifetimes are 60–3600
    /// seconds inclusive. Every successful call creates an independent key. Issuance
    /// deliberately has no idempotent replay, so keep the returned key securely and
    /// do not log it. Secret rotation and application disablement revoke prior keys.
    ///
    /// # Errors
    /// Rejects an out-of-range lifetime before sending. Authentication and testing
    /// context errors are returned by the service.
    pub async fn issue(
        &self,
        input: &models::AppAccessKeyIssue,
    ) -> Result<models::AppAccessKeyIssued> {
        if input
            .ttl_seconds
            .is_some_and(|seconds| !(60..=3600).contains(&seconds))
        {
            return Err(Error::Invalid(
                "app access key lifetime must be 60 to 3600 seconds inclusive".to_owned(),
            ));
        }
        self.0
            .send_json(
                self.0
                    .route(reqwest::Method::POST, &["app-verification", "keys"])?
                    .json(input),
            )
            .await
    }

    /// Verifies the calling application's key as the authenticated receiving application.
    ///
    /// A valid result proves application identity only; the receiver still authorizes
    /// the requested action. Unknown, expired, revoked, app-mismatched or
    /// environment-mismatched keys return `valid_key: false` without app details.
    /// Verification does not consume the key.
    ///
    /// # Errors
    /// Invalid receiving application credentials, invalid testing context, or a
    /// failed request return an error rather than an invalid-key result. Malformed
    /// success responses fail closed with [`Error::Decode`].
    pub async fn verify(
        &self,
        input: &models::AppAccessKeyVerify,
    ) -> Result<models::AppAccessKeyVerification> {
        let response: serde_json::Value = self
            .0
            .send_json(
                self.0
                    .route(reqwest::Method::POST, &["app-verification", "verify"])?
                    .json(input),
            )
            .await?;
        if response.get("valid_key") == Some(&serde_json::Value::Bool(false))
            && (response.get("app_id").is_some() || response.get("valid_till").is_some())
        {
            return Err(Error::Decode(
                "IAM returned application details for an invalid identity key".to_owned(),
            ));
        }
        let verified: models::AppAccessKeyVerification =
            serde_json::from_value(response).map_err(|_| {
                Error::Decode(
                    "IAM returned a malformed application identity verification result".to_owned(),
                )
            })?;
        let consistent = if verified.valid_key {
            verified.app_id.as_deref() == Some(input.app_id.as_str())
                && verified.valid_till.is_some()
        } else {
            verified.app_id.is_none() && verified.valid_till.is_none()
        };
        if !consistent {
            return Err(Error::Decode(
                "IAM returned an inconsistent application identity verification result".to_owned(),
            ));
        }
        Ok(verified)
    }
}
