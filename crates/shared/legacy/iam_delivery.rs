//! Public state of backend-owned recording authorization. No credentials cross this boundary.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryAuthorization {
    pub configured: bool,
    pub enabled: bool,
    pub state: DeliveryAuthorizationState,
    pub actor_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryAuthorizationState {
    Unavailable,
    Pending,
    Active,
    Refreshing,
    NeedsAuth,
    Revoking,
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryAuthorizationConsent {
    pub authorization_id: uuid::Uuid,
    pub authorization_url: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryAuthorizationRequest {
    pub authorization_id: uuid::Uuid,
    pub authorization_code: String,
}
impl DeliveryAuthorizationRequest {
    pub fn validate_testing(&self) -> Result<(), crate::ValidationError> {
        crate::Validate::validate(self)
    }
}
impl std::fmt::Debug for DeliveryAuthorizationRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeliveryAuthorizationRequest")
            .field("authorization_id", &self.authorization_id)
            .field("authorization_code", &"[REDACTED]")
            .finish()
    }
}
impl crate::Validate for DeliveryAuthorizationRequest {
    fn validate(&self) -> Result<(), crate::ValidationError> {
        if self.authorization_id.is_nil()
            || self.authorization_code.is_empty()
            || self.authorization_code.len() > 4096
            || !self.authorization_code.bytes().all(|c| c.is_ascii_graphic())
        {
            return Err(crate::ValidationError::Invalid {
                field: "authorization_code",
                reason: "expected the code from the pending IAM permission request".into(),
            });
        }
        Ok(())
    }
}

/// Separate feature consent. Access and refresh credentials stay on the backend.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingConsent {
    pub authorization_id: String,
    pub consent_url: Option<String>,
    pub state: String,
    pub status: String,
    pub expires_at: String,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingConsentComplete {
    pub code: String,
    pub state: String,
}
impl std::fmt::Debug for RecordingConsentComplete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecordingConsentComplete([REDACTED])")
    }
}
