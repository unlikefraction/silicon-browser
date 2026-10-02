//! The per-request inputs Silicon IAM requires of a mutation.

use secrecy::{ExposeSecret as _, SecretString};

/// A caller-generated key that makes one mutation safe to repeat.
///
/// Ordinary mutating routes in Silicon IAM require one. Application identity
/// key issuance is an exception: it creates a fresh key without replay storage.
/// For replayable mutations, the service binds the key to
/// the caller, the route and the exact request body, then replays the original
/// response for a repeat of the same request -- which is only useful if a retry
/// presents the *same* key. Generate one per logical operation and reuse it for
/// every attempt at that operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Generates a fresh key.
    ///
    /// Use once per logical operation, then hold it across retries of that
    /// operation.
    #[must_use]
    pub fn generate() -> Self {
        Self(uuid::Uuid::now_v7().simple().to_string())
    }

    /// Adopts a caller-supplied key.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Invalid`] unless the key is 16 to 255 visible
    /// ASCII characters, which is the service's accepted form.
    pub fn parse(key: impl Into<String>) -> crate::Result<Self> {
        let key = key.into();
        let valid =
            (16..=255).contains(&key.len()) && key.bytes().all(|byte| byte.is_ascii_graphic());
        if valid {
            Ok(Self(key))
        } else {
            Err(crate::Error::Invalid(
                "an idempotency key is 16 to 255 visible ASCII characters".to_owned(),
            ))
        }
    }

    /// The key as it goes on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for IdempotencyKey {
    fn default() -> Self {
        Self::generate()
    }
}

/// The inputs a mutating call carries besides its body.
///
/// A mutation always needs an idempotency key, and some routes additionally
/// need a step-up assertion. Optimistic concurrency is not here on purpose:
/// where a route requires the current version, the method takes it as an
/// ordinary argument, so it cannot be forgotten.
#[derive(Clone, Debug, Default)]
pub struct Mutation {
    key: IdempotencyKey,
    step_up: Option<SecretString>,
}

impl Mutation {
    /// A mutation with a freshly generated idempotency key.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A mutation reusing a specific key, which is how a retry is expressed.
    #[must_use]
    pub fn with_key(key: IdempotencyKey) -> Self {
        Self { key, step_up: None }
    }

    /// Attaches a step-up assertion obtained from the step-up flow.
    ///
    /// Required by the routes that change authority or reveal a credential;
    /// the service answers `step_up_required` when one is missing.
    #[must_use]
    pub fn step_up(mut self, token: impl Into<String>) -> Self {
        self.step_up = Some(SecretString::from(token.into()));
        self
    }

    /// The key this mutation will present.
    #[must_use]
    pub fn key(&self) -> &IdempotencyKey {
        &self.key
    }

    pub(crate) fn apply(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let request = request.header("idempotency-key", self.key.as_str());
        match &self.step_up {
            Some(token) => request.header("x-step-up-token", token.expose_secret()),
            None => request,
        }
    }
}

/// Where to continue a listing, and how much of it to take.
///
/// Silicon IAM paginates by opaque cursor rather than offset, so a page is
/// continued by handing back the `next_cursor` it returned.
#[derive(Clone, Debug, Default)]
pub struct Paging {
    cursor: Option<String>,
    limit: Option<u16>,
}

impl Paging {
    /// The first page, at the service's default size.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Continues from a cursor a previous page returned.
    #[must_use]
    pub fn after(mut self, cursor: impl Into<String>) -> Self {
        self.cursor = Some(cursor.into());
        self
    }

    /// Requests a specific page size. The service bounds what it accepts.
    #[must_use]
    pub const fn limit(mut self, limit: u16) -> Self {
        self.limit = Some(limit);
        self
    }

    pub(crate) fn query(&self) -> Vec<(&'static str, String)> {
        let mut query = Vec::with_capacity(2);
        if let Some(cursor) = &self.cursor {
            query.push(("cursor", cursor.clone()));
        }
        if let Some(limit) = self.limit {
            query.push(("limit", limit.to_string()));
        }
        query
    }
}

#[cfg(test)]
mod tests {
    use super::{IdempotencyKey, Mutation, Paging};

    #[test]
    fn a_generated_key_is_accepted_by_the_services_own_rule() {
        let key = IdempotencyKey::generate();
        assert!(IdempotencyKey::parse(key.as_str()).is_ok());
    }

    #[test]
    fn generated_keys_do_not_repeat() {
        let first = IdempotencyKey::generate();
        let second = IdempotencyKey::generate();
        assert_ne!(first, second);
    }

    #[test]
    fn a_key_outside_the_accepted_form_is_refused_here() {
        assert!(IdempotencyKey::parse("too-short").is_err());
        assert!(IdempotencyKey::parse("x".repeat(256)).is_err());
        assert!(IdempotencyKey::parse("has\nnewline\nin\nit\n\n\n\n\n\n").is_err());
        assert!(IdempotencyKey::parse("x".repeat(16)).is_ok());
        assert!(IdempotencyKey::parse("x".repeat(255)).is_ok());
    }

    #[test]
    fn a_retry_can_reuse_the_original_key() {
        let key = IdempotencyKey::generate();
        let first = Mutation::with_key(key.clone());
        let retry = Mutation::with_key(key.clone());
        assert_eq!(first.key(), retry.key());
        // A fresh mutation is a different operation, not a retry.
        assert_ne!(Mutation::new().key(), &key);
    }

    #[test]
    fn paging_sends_only_what_was_asked_for() {
        assert!(Paging::new().query().is_empty());
        assert_eq!(
            Paging::new().limit(10).query(),
            vec![("limit", "10".to_owned())]
        );
        assert_eq!(
            Paging::new().after("abc").limit(5).query(),
            vec![("cursor", "abc".to_owned()), ("limit", "5".to_owned())]
        );
    }
}
