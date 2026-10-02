//! Verification for webhook deliveries sent by Silicon IAM.
//!
//! A webhook signature covers the Unix timestamp, a literal `.`, and the
//! exact request body bytes. Verification therefore has to happen before a
//! framework deserializes or otherwise rewrites the body.

use std::{collections::BTreeMap, fmt, time::Duration};

use hmac::{Hmac, Mac as _};
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use sha2::Sha256;
use subtle::ConstantTimeEq as _;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{EnvironmentKey, models};

type HmacSha256 = Hmac<Sha256>;

const EVENT_ID_HEADER: &str = "x-silicon-iam-event-id";
const TIMESTAMP_HEADER: &str = "x-silicon-iam-timestamp";
const KEY_VERSION_HEADER: &str = "x-silicon-iam-key-version";
const SIGNATURE_HEADER: &str = "x-silicon-iam-signature";

/// Maximum body size accepted by the default verifier: one mebibyte.
pub const DEFAULT_MAX_WEBHOOK_BODY_BYTES: usize = 1024 * 1024;

/// Default maximum distance between signing time and verification time.
pub const DEFAULT_WEBHOOK_TOLERANCE: Duration = Duration::from_mins(5);

/// A validated Application webhook signing secret.
///
/// The secret is redacted from [`Debug`](fmt::Debug) output and has no public
/// accessor. Construct this value from the same caller-chosen secret supplied
/// to Application creation or webhook-secret rotation.
#[derive(Clone)]
pub struct WebhookSecret(SecretString);

impl WebhookSecret {
    /// Validates the Application webhook-secret contract before retaining it.
    ///
    /// # Errors
    ///
    /// Returns [`WebhookError::InvalidSecret`] unless the value contains 32 to
    /// 512 non-whitespace ASCII characters.
    pub fn new(secret: impl Into<String>) -> Result<Self, WebhookError> {
        let secret = secret.into();
        if !(32..=512).contains(&secret.len())
            || !secret.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(WebhookError::InvalidSecret);
        }
        Ok(Self(SecretString::from(secret)))
    }

    fn expose(&self) -> &str {
        self.0.expose_secret()
    }
}

impl fmt::Debug for WebhookSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WebhookSecret(<redacted>)")
    }
}

/// Signing secrets indexed by the exact version carried in each delivery.
///
/// Retain old entries for at least the delivery retry window. A URL change
/// which rebinds one secret to a new version should use [`Self::rebind_version`]
/// so both in-flight old deliveries and new deliveries continue to verify.
#[derive(Clone, Default)]
pub struct WebhookSecretKeyring {
    secrets: BTreeMap<i64, WebhookSecret>,
}

impl WebhookSecretKeyring {
    /// Creates a keyring containing its first positive version.
    ///
    /// # Errors
    ///
    /// Returns [`WebhookError::InvalidKeyVersion`] when `version` is not
    /// positive.
    pub fn new(version: i64, secret: WebhookSecret) -> Result<Self, WebhookError> {
        validate_version(version)?;
        Ok(Self {
            secrets: BTreeMap::from([(version, secret)]),
        })
    }

    /// Adds the distinct secret returned by an explicit rotation.
    ///
    /// # Errors
    ///
    /// Returns [`WebhookError::InvalidKeyVersion`] for a non-positive version,
    /// or [`WebhookError::KeyVersionAlreadyExists`] rather than silently
    /// replacing key material already in use.
    pub fn insert(&mut self, version: i64, secret: WebhookSecret) -> Result<(), WebhookError> {
        validate_version(version)?;
        if self.secrets.contains_key(&version) {
            return Err(WebhookError::KeyVersionAlreadyExists(version));
        }
        self.secrets.insert(version, secret);
        Ok(())
    }

    /// Associates an existing secret with a new version while retaining both.
    ///
    /// # Errors
    ///
    /// Returns [`WebhookError::UnknownKeyVersion`] when `existing_version` is
    /// absent, and rejects an invalid or already occupied `new_version`.
    pub fn rebind_version(
        &mut self,
        existing_version: i64,
        new_version: i64,
    ) -> Result<(), WebhookError> {
        validate_version(new_version)?;
        if self.secrets.contains_key(&new_version) {
            return Err(WebhookError::KeyVersionAlreadyExists(new_version));
        }
        let secret = self
            .secrets
            .get(&existing_version)
            .cloned()
            .ok_or(WebhookError::UnknownKeyVersion(existing_version))?;
        self.secrets.insert(new_version, secret);
        Ok(())
    }

    /// Stops accepting deliveries signed under `version`.
    ///
    /// Returns whether that version was present. Only retire a version after
    /// its complete retry window has elapsed.
    pub fn retire(&mut self, version: i64) -> bool {
        self.secrets.remove(&version).is_some()
    }

    /// Reports whether a version is currently accepted.
    #[must_use]
    pub fn contains_version(&self, version: i64) -> bool {
        self.secrets.contains_key(&version)
    }

    fn secret(&self, version: i64) -> Result<&WebhookSecret, WebhookError> {
        self.secrets
            .get(&version)
            .ok_or(WebhookError::UnknownKeyVersion(version))
    }
}

impl fmt::Debug for WebhookSecretKeyring {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebhookSecretKeyring")
            .field("versions", &self.secrets.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// A signature-authenticated and parsed webhook delivery.
///
/// Test deliveries are normalized to the same [`models::WebhookEvent`] shape
/// as production. Their environment root key is retained only in redacted
/// secret storage and can only be checked with
/// [`Self::verify_testing_environment`].
pub struct VerifiedWebhook {
    event: models::WebhookEvent,
    testing_key: Option<SecretString>,
}

impl VerifiedWebhook {
    /// The event identifier authenticated in both the header and body.
    #[must_use]
    pub const fn event_id(&self) -> Uuid {
        self.event.event_id
    }

    /// The normalized, signature-authenticated event.
    #[must_use]
    pub const fn event(&self) -> &models::WebhookEvent {
        &self.event
    }

    /// Whether IAM sent the explicit testing-environment envelope.
    #[must_use]
    pub const fn is_testing(&self) -> bool {
        self.testing_key.is_some()
    }

    /// Constant-time confirmation that this delivery belongs to `expected`.
    ///
    /// Call this before routing, logging, or persisting a test event. The SDK
    /// deliberately does not expose the received root key.
    ///
    /// # Errors
    ///
    /// Returns [`WebhookError::ProductionEvent`] for a production delivery and
    /// [`WebhookError::TestingEnvironmentMismatch`] when the authenticated
    /// envelope names a different testing environment.
    pub fn verify_testing_environment(
        &self,
        expected: &EnvironmentKey,
    ) -> Result<(), WebhookError> {
        let received = self
            .testing_key
            .as_ref()
            .ok_or(WebhookError::ProductionEvent)?;
        if bool::from(
            received
                .expose_secret()
                .as_bytes()
                .ct_eq(expected.expose().as_bytes()),
        ) {
            Ok(())
        } else {
            Err(WebhookError::TestingEnvironmentMismatch)
        }
    }
}

impl fmt::Debug for VerifiedWebhook {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedWebhook")
            .field("event", &self.event)
            .field(
                "testing_key",
                &self.testing_key.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// Exact-byte webhook signature verifier.
pub struct WebhookVerifier {
    keyring: WebhookSecretKeyring,
    tolerance: Duration,
    max_body_bytes: usize,
}

impl WebhookVerifier {
    /// Creates a verifier with a five-minute timestamp tolerance and a 1 MiB
    /// body limit.
    #[must_use]
    pub const fn new(keyring: WebhookSecretKeyring) -> Self {
        Self {
            keyring,
            tolerance: DEFAULT_WEBHOOK_TOLERANCE,
            max_body_bytes: DEFAULT_MAX_WEBHOOK_BODY_BYTES,
        }
    }

    /// Sets the accepted absolute distance from the current Unix timestamp.
    #[must_use]
    pub const fn with_tolerance(mut self, tolerance: Duration) -> Self {
        self.tolerance = tolerance;
        self
    }

    /// Sets the largest signed body accepted before JSON parsing.
    #[must_use]
    pub const fn with_max_body_bytes(mut self, max_body_bytes: usize) -> Self {
        self.max_body_bytes = max_body_bytes;
        self
    }

    /// The accepted signing-key versions, for controlled rotation updates.
    #[must_use]
    pub const fn keyring(&self) -> &WebhookSecretKeyring {
        &self.keyring
    }

    /// Mutably accesses the keyring so a rotation can be installed without
    /// rebuilding the verifier.
    pub const fn keyring_mut(&mut self) -> &mut WebhookSecretKeyring {
        &mut self.keyring
    }

    /// Authenticates the exact body bytes and only then parses the event.
    ///
    /// The four `X-Silicon-IAM-*` security headers must each occur exactly
    /// once. The event ID in the signed body must equal the separately signed
    /// routing header.
    ///
    /// # Errors
    ///
    /// Returns [`WebhookError`] for a missing, duplicate, malformed, stale,
    /// unknown-version, oversized, incorrectly signed, or invalid delivery.
    pub fn verify(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<VerifiedWebhook, WebhookError> {
        self.verify_at(headers, body, OffsetDateTime::now_utc())
    }

    fn verify_at(
        &self,
        headers: &HeaderMap,
        body: &[u8],
        now: OffsetDateTime,
    ) -> Result<VerifiedWebhook, WebhookError> {
        let event_id_header = exactly_one_header(headers, EVENT_ID_HEADER)?;
        let timestamp_header = exactly_one_header(headers, TIMESTAMP_HEADER)?;
        let key_version_header = exactly_one_header(headers, KEY_VERSION_HEADER)?;
        let signature_header = exactly_one_header(headers, SIGNATURE_HEADER)?;

        let event_id = parse_uuid_header(EVENT_ID_HEADER, event_id_header)?;
        let timestamp = parse_positive_i64(TIMESTAMP_HEADER, timestamp_header)?;
        let key_version = parse_positive_i64(KEY_VERSION_HEADER, key_version_header)?;

        if now.unix_timestamp().abs_diff(timestamp) > self.tolerance.as_secs() {
            return Err(WebhookError::TimestampOutsideTolerance);
        }
        if body.len() > self.max_body_bytes {
            return Err(WebhookError::BodyTooLarge {
                actual: body.len(),
                limit: self.max_body_bytes,
            });
        }

        let supplied_signature = decode_signature(signature_header)?;
        let secret = self.keyring.secret(key_version)?;
        let mut mac = <HmacSha256 as hmac::Mac>::new_from_slice(secret.expose().as_bytes())
            .map_err(|_| WebhookError::InvalidSignature)?;
        mac.update(timestamp_header.as_bytes());
        mac.update(b".");
        mac.update(body);
        mac.verify_slice(&supplied_signature)
            .map_err(|_| WebhookError::InvalidSignature)?;

        parse_authenticated_event(body, event_id)
    }
}

impl fmt::Debug for WebhookVerifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebhookVerifier")
            .field("keyring", &self.keyring)
            .field("tolerance", &self.tolerance)
            .field("max_body_bytes", &self.max_body_bytes)
            .finish()
    }
}

/// A webhook could not be configured or authenticated safely.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum WebhookError {
    /// A value was not a valid caller-chosen Application signing secret.
    #[error("the webhook signing secret must contain 32 to 512 non-whitespace ASCII characters")]
    InvalidSecret,
    /// A secret version was zero or negative.
    #[error("webhook signing-key versions must be positive")]
    InvalidKeyVersion,
    /// Inserting a key would silently replace existing material.
    #[error("webhook signing-key version {0} already exists")]
    KeyVersionAlreadyExists(i64),
    /// The delivery requested a version not retained by the receiver.
    #[error("webhook signing-key version {0} is not available")]
    UnknownKeyVersion(i64),
    /// A required security header was absent.
    #[error("required webhook header {0} is missing")]
    MissingHeader(&'static str),
    /// A security header appeared more than once.
    #[error("webhook header {0} must occur exactly once")]
    DuplicateHeader(&'static str),
    /// A security header did not have its canonical wire form.
    #[error("webhook header {0} is malformed")]
    InvalidHeader(&'static str),
    /// The delivery timestamp was too old or too far in the future.
    #[error("the webhook timestamp is outside the accepted tolerance")]
    TimestampOutsideTolerance,
    /// The signed request body exceeded the configured limit.
    #[error("the webhook body is {actual} bytes; the configured limit is {limit}")]
    BodyTooLarge {
        /// Exact received byte count.
        actual: usize,
        /// Configured maximum byte count.
        limit: usize,
    },
    /// The signature was malformed or did not authenticate the exact bytes.
    #[error("the webhook signature is invalid")]
    InvalidSignature,
    /// An authenticated body did not match the webhook event contract.
    #[error("the authenticated webhook body is not a valid event")]
    InvalidPayload,
    /// The authenticated body and event routing header named different IDs.
    #[error("the webhook event ID header does not match the signed body")]
    EventIdMismatch,
    /// Testing-environment confirmation was requested for production traffic.
    #[error("the webhook is a production event, not a testing-environment event")]
    ProductionEvent,
    /// A test delivery carried a different environment root key.
    #[error("the webhook belongs to a different testing environment")]
    TestingEnvironmentMismatch,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TestingEnvelope {
    test: TestingEvent,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TestingEvent {
    testing_key: String,
    metadata: TestingMetadata,
    data: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TestingMetadata {
    #[serde(default)]
    environment_id: Option<Uuid>,
    #[serde(default)]
    generation: Option<i64>,
    spec_version: serde_json::Value,
    event_id: Uuid,
    event_type: String,
    #[serde(with = "time::serde::rfc3339")]
    occurred_at: OffsetDateTime,
    organization_id: RequiredNullableUuid,
    aggregate: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(transparent)]
struct RequiredNullableUuid(Option<Uuid>);

fn validate_version(version: i64) -> Result<(), WebhookError> {
    if version <= 0 {
        return Err(WebhookError::InvalidKeyVersion);
    }
    Ok(())
}

fn exactly_one_header<'a>(
    headers: &'a HeaderMap,
    name: &'static str,
) -> Result<&'a str, WebhookError> {
    let mut values = headers.get_all(name).iter();
    let value = values.next().ok_or(WebhookError::MissingHeader(name))?;
    if values.next().is_some() {
        return Err(WebhookError::DuplicateHeader(name));
    }
    value
        .to_str()
        .map_err(|_| WebhookError::InvalidHeader(name))
}

fn parse_positive_i64(name: &'static str, value: &str) -> Result<i64, WebhookError> {
    let parsed = value
        .parse::<i64>()
        .map_err(|_| WebhookError::InvalidHeader(name))?;
    if parsed <= 0 || parsed.to_string() != value {
        return Err(WebhookError::InvalidHeader(name));
    }
    Ok(parsed)
}

fn parse_uuid_header(name: &'static str, value: &str) -> Result<Uuid, WebhookError> {
    let parsed = Uuid::parse_str(value).map_err(|_| WebhookError::InvalidHeader(name))?;
    if parsed.to_string() != value {
        return Err(WebhookError::InvalidHeader(name));
    }
    Ok(parsed)
}

fn decode_signature(value: &str) -> Result<[u8; 32], WebhookError> {
    let encoded = value
        .strip_prefix("v1=")
        .ok_or(WebhookError::InvalidSignature)?;
    if encoded.len() != 64
        || !encoded
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(WebhookError::InvalidSignature);
    }
    let mut signature = [0_u8; 32];
    for (index, destination) in signature.iter_mut().enumerate() {
        let offset = index * 2;
        let high =
            decode_lower_hex(encoded.as_bytes()[offset]).ok_or(WebhookError::InvalidSignature)?;
        let low = decode_lower_hex(encoded.as_bytes()[offset + 1])
            .ok_or(WebhookError::InvalidSignature)?;
        *destination = (high << 4) | low;
    }
    Ok(signature)
}

const fn decode_lower_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn parse_authenticated_event(
    body: &[u8],
    header_event_id: Uuid,
) -> Result<VerifiedWebhook, WebhookError> {
    let value = serde_json::from_slice::<serde_json::Value>(body)
        .map_err(|_| WebhookError::InvalidPayload)?;
    let object = value.as_object().ok_or(WebhookError::InvalidPayload)?;
    let (event, testing_key) = if object.contains_key("test") {
        if object.len() != 1 {
            return Err(WebhookError::InvalidPayload);
        }
        let envelope = serde_json::from_value::<TestingEnvelope>(value)
            .map_err(|_| WebhookError::InvalidPayload)?;
        if !valid_environment_key(&envelope.test.testing_key) {
            return Err(WebhookError::InvalidPayload);
        }
        let mut metadata = envelope.test.metadata;
        match (metadata.environment_id, metadata.generation) {
            (None, None) => {}
            (Some(id), Some(generation)) if generation > 0 => {
                let aggregate = metadata
                    .aggregate
                    .as_object_mut()
                    .ok_or(WebhookError::InvalidPayload)?;
                for (key, value) in [
                    ("environment_id", serde_json::json!(id)),
                    ("generation", serde_json::json!(generation)),
                ] {
                    if aggregate
                        .get(key)
                        .is_some_and(|existing| existing != &value)
                    {
                        return Err(WebhookError::InvalidPayload);
                    }
                    aggregate.insert(key.to_owned(), value);
                }
            }
            _ => return Err(WebhookError::InvalidPayload),
        }
        let event = models::WebhookEvent {
            spec_version: metadata.spec_version,
            event_id: metadata.event_id,
            event_type: metadata.event_type,
            occurred_at: metadata.occurred_at,
            organization_id: metadata.organization_id.0,
            aggregate: metadata.aggregate,
            data: envelope.test.data,
        };
        (event, Some(SecretString::from(envelope.test.testing_key)))
    } else {
        let event = serde_json::from_value::<models::WebhookEvent>(value)
            .map_err(|_| WebhookError::InvalidPayload)?;
        (event, None)
    };

    validate_event(&event)?;
    if event.event_id != header_event_id {
        return Err(WebhookError::EventIdMismatch);
    }
    Ok(VerifiedWebhook { event, testing_key })
}

fn validate_event(event: &models::WebhookEvent) -> Result<(), WebhookError> {
    if event.spec_version.as_str() != Some("1.0")
        || !valid_event_type(&event.event_type)
        || !event.data.is_object()
    {
        return Err(WebhookError::InvalidPayload);
    }
    let aggregate = event
        .aggregate
        .as_object()
        .ok_or(WebhookError::InvalidPayload)?;
    let valid_aggregate = aggregate
        .get("type")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|aggregate_type| !aggregate_type.is_empty())
        && aggregate
            .get("id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|id| Uuid::parse_str(id).is_ok())
        && aggregate
            .get("version")
            .and_then(serde_json::Value::as_i64)
            .is_some_and(|version| version > 0);
    if !valid_aggregate {
        return Err(WebhookError::InvalidPayload);
    }
    Ok(())
}

fn valid_event_type(event_type: &str) -> bool {
    let mut parts = event_type.split('.').peekable();
    let mut count = 0_usize;
    while let Some(part) = parts.next() {
        count += 1;
        if parts.peek().is_none() {
            return count >= 2
                && part.strip_prefix('v').is_some_and(|version| {
                    version.as_bytes().first().is_some_and(u8::is_ascii_digit)
                        && !version.starts_with('0')
                        && version.bytes().all(|byte| byte.is_ascii_digit())
                });
        }
        let mut bytes = part.bytes();
        if !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
            || !bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return false;
        }
    }
    false
}

fn valid_environment_key(key: &str) -> bool {
    key.len() == 32 && key.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use hmac::{Hmac, Mac as _};
    use http::{HeaderMap, HeaderValue};
    use sha2::Sha256;
    use time::{Duration as TimeDuration, OffsetDateTime};
    use uuid::Uuid;

    use crate::EnvironmentKey;

    use super::{
        EVENT_ID_HEADER, KEY_VERSION_HEADER, SIGNATURE_HEADER, TIMESTAMP_HEADER, WebhookError,
        WebhookSecret, WebhookSecretKeyring, WebhookVerifier,
    };

    const SECRET: &str = "whs_abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ";

    fn verifier() -> WebhookVerifier {
        let Ok(secret) = WebhookSecret::new(SECRET) else {
            panic!("the fixture is a valid webhook secret");
        };
        let Ok(keyring) = WebhookSecretKeyring::new(7, secret) else {
            panic!("the fixture has a positive key version");
        };
        WebhookVerifier::new(keyring)
    }

    #[test]
    fn caller_chosen_webhook_secret_contract_is_validated() {
        assert!(WebhookSecret::new("caller-owned-webhook-secret-00001").is_ok());
        assert!(WebhookSecret::new("x".repeat(512)).is_ok());
        assert!(matches!(
            WebhookSecret::new("x".repeat(31)),
            Err(WebhookError::InvalidSecret)
        ));
        assert!(matches!(
            WebhookSecret::new("caller owned webhook secret 00001"),
            Err(WebhookError::InvalidSecret)
        ));
    }

    fn signed_headers(event_id: Uuid, timestamp: i64, version: i64, body: &[u8]) -> HeaderMap {
        let mut mac = <Hmac<Sha256> as hmac::Mac>::new_from_slice(SECRET.as_bytes())
            .unwrap_or_else(|_| unreachable!("HMAC accepts this fixture key"));
        mac.update(timestamp.to_string().as_bytes());
        mac.update(b".");
        mac.update(body);
        let signature = mac.finalize().into_bytes();
        let signature = signature
            .iter()
            .fold(String::with_capacity(67), |mut encoded, byte| {
                use std::fmt::Write as _;
                if encoded.is_empty() {
                    encoded.push_str("v1=");
                }
                let _ = write!(encoded, "{byte:02x}");
                encoded
            });

        let mut headers = HeaderMap::new();
        let values = [
            (EVENT_ID_HEADER, event_id.to_string()),
            (TIMESTAMP_HEADER, timestamp.to_string()),
            (KEY_VERSION_HEADER, version.to_string()),
            (SIGNATURE_HEADER, signature),
        ];
        for (name, value) in values {
            let Ok(value) = HeaderValue::from_str(&value) else {
                unreachable!("fixtures are valid HTTP header values");
            };
            headers.insert(name, value);
        }
        headers
    }

    fn production_body(event_id: Uuid) -> Vec<u8> {
        format!(
            r#"{{"spec_version":"1.0","event_id":"{event_id}","event_type":"organization.membership.created.v1","occurred_at":"2026-09-04T00:00:00Z","organization_id":null,"aggregate":{{"type":"membership","id":"00000000-0000-0000-0000-000000000002","version":1}},"data":{{"name":"Ada"}}}}"#
        )
        .into_bytes()
    }

    fn testing_body(event_id: Uuid, testing_key: &str) -> Vec<u8> {
        format!(
            r#"{{"test":{{"testing_key":"{testing_key}","metadata":{{"spec_version":"1.0","event_id":"{event_id}","event_type":"organization.membership.created.v1","occurred_at":"2026-09-04T00:00:00Z","organization_id":null,"aggregate":{{"type":"membership","id":"00000000-0000-0000-0000-000000000002","version":1}}}},"data":{{"name":"Ada"}}}}}}"#
        )
        .into_bytes()
    }

    #[test]
    fn verifies_production_bytes_and_cross_checks_the_event_id() {
        let event_id = Uuid::from_u128(1);
        let body = production_body(event_id);
        let now = OffsetDateTime::now_utc();
        let headers = signed_headers(event_id, now.unix_timestamp(), 7, &body);
        let verified = verifier().verify_at(&headers, &body, now);
        let Ok(verified) = verified else {
            panic!("the correctly signed production event must verify");
        };
        assert_eq!(verified.event_id(), event_id);
        assert_eq!(
            verified.event().event_type,
            "organization.membership.created.v1"
        );
        assert_eq!(
            verified
                .event()
                .data
                .get("name")
                .and_then(|name| name.as_str()),
            Some("Ada")
        );
        assert!(!verified.is_testing());

        let different_id = Uuid::from_u128(9);
        let mismatched = signed_headers(different_id, now.unix_timestamp(), 7, &body);
        assert!(matches!(
            verifier().verify_at(&mismatched, &body, now),
            Err(WebhookError::EventIdMismatch)
        ));
    }

    #[test]
    fn verifies_wrapped_test_events_and_never_exposes_the_testing_key() {
        let event_id = Uuid::from_u128(1);
        let testing_key = "A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6";
        let body = testing_body(event_id, testing_key);
        let now = OffsetDateTime::now_utc();
        let headers = signed_headers(event_id, now.unix_timestamp(), 7, &body);
        let verified = verifier().verify_at(&headers, &body, now);
        let Ok(verified) = verified else {
            panic!("the correctly signed testing event must verify");
        };
        assert!(verified.is_testing());
        assert_eq!(verified.event_id(), event_id);
        assert_eq!(
            verified
                .event()
                .data
                .get("name")
                .and_then(|name| name.as_str()),
            Some("Ada")
        );

        let Ok(expected) = EnvironmentKey::new(testing_key) else {
            panic!("the fixture is a valid testing environment key");
        };
        assert!(verified.verify_testing_environment(&expected).is_ok());
        let Ok(other) = EnvironmentKey::new("Z".repeat(32)) else {
            panic!("the fixture is a valid testing environment key");
        };
        assert_eq!(
            verified.verify_testing_environment(&other),
            Err(WebhookError::TestingEnvironmentMismatch)
        );
        assert!(!format!("{verified:?}").contains(testing_key));
    }

    #[test]
    #[allow(
        clippy::expect_used,
        reason = "fixed signed fixtures must decode in regression tests"
    )]
    fn testing_metadata_accepts_direct_fences_and_rejects_ambiguous_fences() {
        let event_id = Uuid::from_u128(1);
        let environment = Uuid::from_u128(7);
        let now = OffsetDateTime::now_utc();
        let fixture: serde_json::Value =
            serde_json::from_slice(&testing_body(event_id, "A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6"))
                .expect("fixture");
        for (direct, aggregate, accepted) in [
            (
                serde_json::json!({"environment_id": environment, "generation": 2}),
                None,
                true,
            ),
            (
                serde_json::json!({"environment_id": environment}),
                None,
                false,
            ),
            (serde_json::json!({"generation": 2}), None, false),
            (
                serde_json::json!({"environment_id": environment, "generation": 0}),
                None,
                false,
            ),
            (
                serde_json::json!({"environment_id": environment, "generation": 2}),
                Some(1),
                false,
            ),
            (
                serde_json::json!({"environment_id": environment, "generation": 2}),
                Some(2),
                true,
            ),
        ] {
            let mut body = fixture.clone();
            let metadata = body["test"]["metadata"].as_object_mut().expect("metadata");
            metadata.extend(direct.as_object().expect("direct").clone());
            if let Some(generation) = aggregate {
                metadata["aggregate"]["generation"] = serde_json::json!(generation);
            }
            let body = serde_json::to_vec(&body).expect("body");
            let headers = signed_headers(event_id, now.unix_timestamp(), 7, &body);
            let result = verifier().verify_at(&headers, &body, now);
            assert_eq!(result.is_ok(), accepted);
            if let Ok(verified) = result {
                assert_eq!(
                    verified.event().aggregate["environment_id"],
                    serde_json::json!(environment)
                );
                assert_eq!(verified.event().aggregate["generation"], 2);
            }
        }
    }

    #[test]
    fn exact_bytes_duplicate_headers_and_timestamp_window_fail_closed() {
        let event_id = Uuid::from_u128(1);
        let body = production_body(event_id);
        let now = OffsetDateTime::now_utc();
        let mut headers = signed_headers(event_id, now.unix_timestamp(), 7, &body);

        let mut changed = body.clone();
        changed.push(b' ');
        assert!(matches!(
            verifier().verify_at(&headers, &changed, now),
            Err(WebhookError::InvalidSignature)
        ));

        headers.append(TIMESTAMP_HEADER, HeaderValue::from_static("1"));
        assert!(matches!(
            verifier().verify_at(&headers, &body, now),
            Err(WebhookError::DuplicateHeader(TIMESTAMP_HEADER))
        ));

        let stale_at = now - TimeDuration::minutes(6);
        let stale = signed_headers(event_id, stale_at.unix_timestamp(), 7, &body);
        assert!(matches!(
            verifier().verify_at(&stale, &body, now),
            Err(WebhookError::TimestampOutsideTolerance)
        ));
    }

    #[test]
    fn rotation_adds_secrets_and_url_rebinding_retains_in_flight_versions() {
        let Ok(secret) = WebhookSecret::new(SECRET) else {
            panic!("the fixture is a valid webhook secret");
        };
        let Ok(mut keyring) = WebhookSecretKeyring::new(7, secret) else {
            panic!("the fixture has a positive version");
        };
        assert!(keyring.rebind_version(7, 8).is_ok());
        assert!(keyring.contains_version(7));
        assert!(keyring.contains_version(8));
        assert_eq!(
            keyring.rebind_version(7, 8),
            Err(WebhookError::KeyVersionAlreadyExists(8))
        );
        assert!(keyring.retire(7));
        assert!(!keyring.contains_version(7));
    }
}
