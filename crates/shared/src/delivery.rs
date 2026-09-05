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

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryAuthorizationRequest {
    pub short_lived_token: String,
}
impl std::fmt::Debug for DeliveryAuthorizationRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeliveryAuthorizationRequest").field("short_lived_token", &"[REDACTED]").finish()
    }
}
impl crate::Validate for DeliveryAuthorizationRequest {
    fn validate(&self) -> Result<(), crate::ValidationError> {
        crate::AuthExchangeRequest { short_lived_token: self.short_lived_token.clone(), org_id: "delivery".into() }
            .validate()
    }
}
