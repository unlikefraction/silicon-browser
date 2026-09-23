//! What a request presents to prove who is calling.

use secrecy::{ExposeSecret as _, SecretString};

/// The authority a request carries.
///
/// Silicon IAM accepts three kinds of caller credential, and which one a route
/// wants is part of the contract rather than a preference. Naming them as a
/// closed set means a call cannot silently go out unauthenticated.
#[derive(Clone)]
pub enum Credential {
    /// No credential. Correct for signup, login, token refresh, and the
    /// version and health routes -- and wrong everywhere else.
    Anonymous,

    /// An opaque IAM access token, sent as `Authorization: Bearer`.
    ///
    /// Issued to a Carbon by login, or to a Silicon by its own token route.
    Bearer(SecretString),

    /// An application's own identity: its `app_id` and current secret, sent
    /// as HTTP Basic. Used by the OAuth token, introspection and revocation
    /// routes and by delegated access.
    Application {
        /// Public application identifier.
        app_id: String,
        /// Current versioned application secret.
        secret: SecretString,
    },
}

impl Credential {
    /// A bearer credential from anything string-like.
    pub fn bearer(token: impl Into<String>) -> Self {
        Self::Bearer(SecretString::from(token.into()))
    }

    /// An application credential from its identifier and secret.
    pub fn application(app_id: impl Into<String>, secret: impl Into<String>) -> Self {
        Self::Application {
            app_id: app_id.into(),
            secret: SecretString::from(secret.into()),
        }
    }

    /// Whether this credential identifies anyone at all.
    #[must_use]
    pub const fn is_anonymous(&self) -> bool {
        matches!(self, Self::Anonymous)
    }

    pub(crate) fn apply(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            Self::Anonymous => request,
            Self::Bearer(token) => request.bearer_auth(token.expose_secret()),
            Self::Application { app_id, secret } => {
                request.basic_auth(app_id, Some(secret.expose_secret()))
            }
        }
    }
}

impl std::fmt::Debug for Credential {
    /// Never renders secret material, so a credential cannot reach a log by
    /// way of a derived `Debug` on some enclosing struct.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Anonymous => formatter.write_str("Credential::Anonymous"),
            Self::Bearer(_) => formatter.write_str("Credential::Bearer(<redacted>)"),
            Self::Application { app_id, .. } => formatter
                .debug_struct("Credential::Application")
                .field("app_id", app_id)
                .field("secret", &"<redacted>")
                .finish(),
        }
    }
}

/// A testing environment key: the root authority inside one environment.
///
/// Presenting it moves a request onto that environment's data, against the
/// same routes. It is deliberately a distinct type from [`Credential`],
/// because it does not replace one -- a request inside an environment still
/// authenticates as whoever it is.
#[derive(Clone)]
pub struct EnvironmentKey(SecretString);

impl EnvironmentKey {
    /// Accepts a key in the service's fixed wire form: exactly 32
    /// alphanumeric characters.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Invalid`] for anything else, so a malformed key
    /// fails here rather than as an opaque 401 later.
    pub fn new(key: impl Into<String>) -> crate::Result<Self> {
        let key = key.into();
        if key.len() != 32 || !key.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(crate::Error::Invalid(
                "a testing environment key is exactly 32 alphanumeric characters".to_owned(),
            ));
        }
        Ok(Self(SecretString::from(key)))
    }

    pub(crate) fn expose(&self) -> &str {
        self.0.expose_secret()
    }
}

impl std::fmt::Debug for EnvironmentKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("EnvironmentKey(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::{Credential, EnvironmentKey};

    #[test]
    fn debug_never_renders_secret_material() {
        let bearer = format!("{:?}", Credential::bearer("cat_supersecretvalue"));
        assert!(!bearer.contains("supersecret"), "{bearer}");

        let application = format!("{:?}", Credential::application("billing", "ask_secret"));
        assert!(application.contains("billing"), "{application}");
        assert!(!application.contains("ask_secret"), "{application}");

        let Ok(key) = EnvironmentKey::new("a".repeat(32)) else {
            panic!("a 32-character alphanumeric key is valid");
        };
        assert_eq!(format!("{key:?}"), "EnvironmentKey(<redacted>)");
    }

    #[test]
    fn an_environment_key_must_match_the_wire_form() {
        assert!(EnvironmentKey::new("a".repeat(32)).is_ok());
        assert!(EnvironmentKey::new("a".repeat(31)).is_err());
        assert!(EnvironmentKey::new("a".repeat(33)).is_err());
        assert!(EnvironmentKey::new(format!("{}-", "a".repeat(31))).is_err());
        assert!(EnvironmentKey::new("").is_err());
    }

    #[test]
    fn only_the_anonymous_credential_reports_itself_as_such() {
        assert!(Credential::Anonymous.is_anonymous());
        assert!(!Credential::bearer("cat_x").is_anonymous());
        assert!(!Credential::application("a", "b").is_anonymous());
    }
}
