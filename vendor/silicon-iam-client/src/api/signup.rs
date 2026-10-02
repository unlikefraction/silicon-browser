//! Creating a Carbon.
//!
//! Signup binds a verified email and an optional verified phone to one temporary
//! session, then exchanges that session for an account:
//!
//! ```text
//! start -> send email code -> verify -> send phone code -> verify -> complete
//! ```
//!
//! Each send reports whether the identity already belongs to a Carbon; when it
//! does, no code is sent and there is nothing to verify.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Client, Mutation, Result, models};

/// Validates a new Carbon ID before starting either verification ceremony.
///
/// # Errors
///
/// Returns an error unless the ID matches the creation contract's
/// `^c:[a-z1-9_-]{3,30}$` syntax. In particular, digit zero is not allowed.
pub fn validate_carbon_id(value: &str) -> Result<()> {
    let Some(value) = value.strip_prefix("c:") else {
        return Err(crate::Error::Invalid(
            "Carbon ID must start with c:".to_owned(),
        ));
    };
    if (3..=30).contains(&value.len())
        && value
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'1'..=b'9' | b'_' | b'-'))
    {
        Ok(())
    } else {
        Err(crate::Error::Invalid("Carbon ID must be 3-30 lowercase letters, digits 1-9, underscores or hyphens; digit 0 is not allowed".to_owned()))
    }
}

/// The signup flow. None of these routes need a credential.
pub struct Signup<'a>(pub(super) &'a Client);

#[derive(Serialize)]
struct Code<'a> {
    code: &'a str,
}

impl Signup<'_> {
    /// Lists Silicon identities for which this Carbon is the custodian.
    /// # Errors
    /// Requires a direct Carbon IAM session.
    pub async fn silicon_custodies(&self) -> Result<SiliconCustodies> {
        self.0.get(&["me", "silicon-custodies"]).await
    }

    /// Updates the custodian's organization-creation setting for one Silicon.
    /// # Errors
    /// Returns an error when custody is unavailable or the version is stale.
    pub async fn set_silicon_organization_creation(
        &self,
        silicon_id: &str,
        version: i64,
        allowed: bool,
        mutation: &Mutation,
    ) -> Result<serde_json::Value> {
        self.0
            .patch(
                &["me", "silicon-custodies", silicon_id],
                version,
                &serde_json::json!({"can_create_organizations": allowed}),
                mutation,
            )
            .await
    }
    /// Discards the optional phone and invalidates its pending verification code.
    ///
    /// # Errors
    /// Returns an error when the signup session has expired or completed.
    pub async fn skip_phone(&self, session: Uuid, mutation: &Mutation) -> Result<()> {
        self.0
            .delete(
                &["signup", "sessions", &session.to_string(), "phone"],
                None,
                mutation,
            )
            .await
    }

    /// Whether a Carbon ID can still be claimed.
    ///
    /// A positive answer reserves nothing; the claim happens at completion.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails.
    pub async fn carbon_id_available(&self, carbon_id: &str) -> Result<models::Availability> {
        self.0.get(&["carbon-ids", carbon_id, "availability"]).await
    }

    /// Opens a signup session. It lives for 48 hours.
    ///
    /// # Errors
    ///
    /// Returns an error when the request fails.
    pub async fn start(&self, mutation: &Mutation) -> Result<models::AuthSession> {
        self.0
            .post(&["signup", "sessions"], &serde_json::json!({}), mutation)
            .await
    }

    /// Sends a verification code to an email address.
    ///
    /// # Errors
    ///
    /// Returns an error when the address is rejected or delivery fails.
    pub async fn send_email_code(
        &self,
        session: Uuid,
        email: &str,
        mutation: &Mutation,
    ) -> Result<models::CodeDispatchResult> {
        self.0
            .post(
                &["signup", "sessions", &session.to_string(), "email"],
                &models::EmailInput {
                    email: email.to_owned(),
                },
                mutation,
            )
            .await
    }

    /// Verifies the emailed code.
    ///
    /// # Errors
    ///
    /// Returns an error when the code is wrong, expired, or exhausted.
    pub async fn verify_email(&self, session: Uuid, code: &str, mutation: &Mutation) -> Result<()> {
        self.0
            .post_empty(
                &[
                    "signup",
                    "sessions",
                    &session.to_string(),
                    "email",
                    "verify",
                ],
                &Code { code },
                mutation,
            )
            .await
    }

    /// Sends a verification code to a phone number, in E.164 form.
    ///
    /// # Errors
    ///
    /// Returns an error when the number is rejected or delivery fails.
    pub async fn send_phone_code(
        &self,
        session: Uuid,
        phone_number: &str,
        mutation: &Mutation,
    ) -> Result<models::CodeDispatchResult> {
        self.0
            .post(
                &["signup", "sessions", &session.to_string(), "phone"],
                &models::PhoneInput {
                    phone_number: phone_number.to_owned(),
                },
                mutation,
            )
            .await
    }

    /// Verifies the texted code.
    ///
    /// # Errors
    ///
    /// Returns an error when the code is wrong, expired, or exhausted.
    pub async fn verify_phone(&self, session: Uuid, code: &str, mutation: &Mutation) -> Result<()> {
        self.0
            .post_empty(
                &[
                    "signup",
                    "sessions",
                    &session.to_string(),
                    "phone",
                    "verify",
                ],
                &Code { code },
                mutation,
            )
            .await
    }

    /// Creates and signs in the Carbon. Email and any supplied phone must be verified.
    ///
    /// # Errors
    ///
    /// Returns an error when a contact is unverified, or the Carbon ID was
    /// taken between the availability check and here.
    pub async fn complete(
        &self,
        session: Uuid,
        profile: &models::CarbonSignupComplete,
        mutation: &Mutation,
    ) -> Result<models::CarbonSignupResult> {
        if let Some(carbon_id) = &profile.carbon_id {
            validate_carbon_id(carbon_id)?;
        }
        self.0
            .post(
                &["signup", "sessions", &session.to_string(), "complete"],
                profile,
                mutation,
            )
            .await
    }
}

/// Silicon custody relationships belonging to the current Carbon.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SiliconCustodies {
    /// Current custody relationships.
    pub items: Vec<SiliconCustody>,
}
/// A Carbon's settings for a Silicon identity.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SiliconCustody {
    /// Globally unique Silicon ID.
    pub silicon_id: String,
    /// Silicon display name.
    pub display_name: String,
    /// Whether this Silicon may create organizations.
    pub can_create_organizations: bool,
    /// Optimistic concurrency version.
    pub version: i64,
}

impl Signup<'_> {
    /// Registers a Silicon pending verified Carbon custody approval.
    /// # Errors
    /// Returns validation, duplicate identity or delivery-queue errors.
    pub async fn silicon(
        &self,
        input: &models::SiliconSignupRequest,
        mutation: &Mutation,
    ) -> Result<models::SiliconSignupCreated> {
        self.0
            .post(&["silicon-signup", "requests"], input, mutation)
            .await
    }
    /// Polls one pending Silicon registration without granting IAM access.
    /// # Errors
    /// Returns an error for an invalid polling capability or unavailable request.
    pub async fn silicon_status(
        &self,
        request: Uuid,
        poll_token: &str,
    ) -> Result<models::SiliconSignupStatus> {
        self.0
            .with_credential(crate::Credential::bearer(poll_token))
            .get(&["silicon-signup", "requests", &request.to_string()])
            .await
    }
    /// Reads a custody request as its prospective verified Carbon custodian.
    /// # Errors
    /// Returns not found unless the caller's verified email matches.
    pub async fn silicon_custody(&self, request: Uuid) -> Result<models::SiliconSignupStatus> {
        self.0
            .get(&[
                "silicon-signup",
                "requests",
                &request.to_string(),
                "custodian",
            ])
            .await
    }
    /// Approves or rejects Silicon custody with the explicit organization-creation setting.
    /// # Errors
    /// Returns an error unless the caller owns the verified target email and the request is pending.
    pub async fn decide_silicon_custody(
        &self,
        request: Uuid,
        approve: bool,
        can_create_organizations: bool,
        mutation: &Mutation,
    ) -> Result<models::SiliconSignupStatus> {
        self.0.post(&["silicon-signup","requests",&request.to_string(),"custodian"],&serde_json::json!({"approve":approve,"can_create_organizations":can_create_organizations}),mutation).await
    }
}

/// Availability of the two configured external signup providers.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SocialProviders {
    /// Providers and their actual deployment availability.
    pub providers: Vec<SocialProvider>,
}
/// One configured external signup provider.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SocialProvider {
    /// Stable provider ID: google or apple.
    pub id: String,
    /// Whether this deployment has usable provider credentials.
    pub enabled: bool,
}
fn check_social_provider(provider: &str) -> Result<()> {
    if matches!(provider, "google" | "apple") {
        Ok(())
    } else {
        Err(crate::Error::Invalid(
            "Provider must be google or apple".to_owned(),
        ))
    }
}
impl Signup<'_> {
    /// Lists real deployment availability for Google and Apple signup.
    /// # Errors
    /// Returns an error if provider discovery is unavailable.
    pub async fn social_providers(&self) -> Result<SocialProviders> {
        self.0.get(&["signup", "social", "providers"]).await
    }
    /// Begins external email verification; keep its polling token confidential.
    /// # Errors
    /// Returns an error for an unsupported or unconfigured provider.
    pub async fn social_start(
        &self,
        provider: &str,
        mutation: &Mutation,
    ) -> Result<models::SocialSignupStart> {
        check_social_provider(provider)?;
        self.0
            .post(
                &["signup", "social", provider, "start"],
                &serde_json::json!({}),
                mutation,
            )
            .await
    }
    /// Polls external verification. Verified responses resume ordinary signup.
    /// # Errors
    /// Returns an error for invalid polling authority or an unavailable request.
    pub async fn social_status(
        &self,
        provider: &str,
        input: &models::SocialSignupStatusInput,
    ) -> Result<models::SocialSignupStatus> {
        check_social_provider(provider)?;
        self.0
            .send_json(
                self.0
                    .route(
                        reqwest::Method::POST,
                        &["signup", "social", provider, "status"],
                    )?
                    .json(input),
            )
            .await
    }
}
