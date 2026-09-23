//! Delegated access between declared applications across organizations.
//!
//! One application asks Silicon IAM for a single-use proof that it may call
//! another, then the callee verifies that proof. Both ends authenticate with
//! their own application credential.

use hmac::{Hmac, Mac as _};
use secrecy::ExposeSecret as _;
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;

use crate::{Client, Credential, Error, Mutation, Result, models};

type HmacSha256 = Hmac<Sha256>;

/// Hashes exact downstream request bytes in the OBO wire format.
///
/// The returned value is the 64-character lowercase hexadecimal SHA-256
/// digest required by exchange and verification request bindings.
#[must_use]
pub fn body_sha256(body: &[u8]) -> String {
    lower_hex(&Sha256::digest(body))
}

/// On-behalf-of access.
pub struct Obo<'a>(pub(super) &'a Client);

impl Obo<'_> {
    /// The endpoints an application publishes for delegated access.
    ///
    /// # Errors
    ///
    /// Returns an error when the audience is unavailable in the selected
    /// production or testing environment. Application ownership may differ.
    pub async fn endpoints(&self, app_id: &str) -> Result<models::OboEndpointCatalog> {
        self.0
            .get(&["obo-access", "applications", app_id, "endpoints"])
            .await
    }

    /// Signs one exchange with this client's Application secret.
    ///
    /// The endpoint path is selected from `catalog`; callers never supply a
    /// second path that could drift from the discovered endpoint. The request,
    /// timestamp, and [`Mutation`] are the same values that must be passed to
    /// [`Self::exchange`]. Prefer [`Self::exchange_signed`] when this client is
    /// also sending the exchange.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invalid`] unless the client has an Application
    /// credential, the catalog matches the request's audience and endpoint,
    /// and every signed field has the backend's canonical wire form.
    pub fn sign_exchange(
        &self,
        request: &models::OboExchangeRequest,
        catalog: &models::OboEndpointCatalog,
        timestamp: i64,
        mutation: &Mutation,
    ) -> Result<String> {
        let timestamp = canonical_timestamp(timestamp)?;
        self.sign_exchange_at(request, catalog, &timestamp, mutation)
    }

    /// Signs and sends one OBO exchange with a fresh timestamp.
    ///
    /// Use the current catalog, the exact request body digest, and one
    /// [`Mutation`] for the logical exchange. After an uncertain response,
    /// reuse the request and mutation; this method signs the retry with a new
    /// timestamp because the server's signature window is short.
    ///
    /// # Errors
    ///
    /// Returns the validation failures documented by [`Self::sign_exchange`],
    /// or any error returned by the exchange endpoint.
    pub async fn exchange_signed(
        &self,
        request: &models::OboExchangeRequest,
        catalog: &models::OboEndpointCatalog,
        mutation: &Mutation,
    ) -> Result<models::OboProofResponse> {
        self.exchange_signed_at(
            request,
            catalog,
            OffsetDateTime::now_utc().unix_timestamp(),
            mutation,
        )
        .await
    }

    /// Signs and sends one OBO exchange with an explicit Unix timestamp.
    ///
    /// This is useful for a caller that must reproduce a particular signed
    /// request. Normal exchanges and retries should use
    /// [`Self::exchange_signed`] so the short freshness window cannot expire
    /// while a timestamp is being restored.
    ///
    /// # Errors
    ///
    /// Returns the validation failures documented by [`Self::sign_exchange`],
    /// or any error returned by the exchange endpoint.
    pub async fn exchange_signed_at(
        &self,
        request: &models::OboExchangeRequest,
        catalog: &models::OboEndpointCatalog,
        timestamp: i64,
        mutation: &Mutation,
    ) -> Result<models::OboProofResponse> {
        let timestamp = canonical_timestamp(timestamp)?;
        let signature = self.sign_exchange_at(request, catalog, &timestamp, mutation)?;
        self.exchange(request, &timestamp, &signature, mutation)
            .await
    }

    /// Exchanges a signed request for a single-use capability proof.
    ///
    /// The proof is bound to the exact method, endpoint and body digest given
    /// here, so it cannot be replayed against a different call. `timestamp`
    /// and `signature` are the request signature the contract specifies.
    ///
    /// # Errors
    ///
    /// Returns an error when the signature does not verify, the timestamp is
    /// outside tolerance, or the callee does not grant the caller access.
    pub async fn exchange(
        &self,
        request: &models::OboExchangeRequest,
        timestamp: &str,
        signature: &str,
        mutation: &Mutation,
    ) -> Result<models::OboProofResponse> {
        let built = mutation
            .apply(
                self.0
                    .route(reqwest::Method::POST, &["obo-access", "exchanges"])?,
            )
            .header("x-obo-timestamp", timestamp)
            .header("x-obo-signature", signature)
            .json(request);
        self.0.send_json(built).await
    }

    /// Consumes a proof, confirming what the caller may do.
    ///
    /// Single use: a second verification of the same proof fails, which is
    /// what stops a captured proof from being replayed.
    ///
    /// # Errors
    ///
    /// Returns an error when the proof is unknown, expired, already consumed,
    /// or does not match the request it is presented against.
    pub async fn verify(
        &self,
        request: &models::OboVerifyRequest,
    ) -> Result<models::OboAccessResult> {
        let built = self
            .0
            .route(reqwest::Method::POST, &["obo-access", "verify"])?
            .json(request);
        self.0.send_json(built).await
    }

    fn sign_exchange_at(
        &self,
        request: &models::OboExchangeRequest,
        catalog: &models::OboEndpointCatalog,
        timestamp: &str,
        mutation: &Mutation,
    ) -> Result<String> {
        let Credential::Application { secret, .. } = self.0.credential() else {
            return Err(Error::Invalid(
                "OBO exchange signing requires an Application credential".to_owned(),
            ));
        };
        let endpoint_path = signing_endpoint_path(catalog, request)?;
        validate_canonical_method(&request.request.method)?;
        validate_body_sha256(&request.request.body_sha256)?;

        let canonical = format!(
            "{}.{}.{}.{}.{}",
            timestamp,
            request.request.method,
            endpoint_path,
            request.request.body_sha256,
            mutation.key().as_str()
        );
        let mut mac = <HmacSha256 as hmac::Mac>::new_from_slice(secret.expose_secret().as_bytes())
            .map_err(|_| Error::Invalid("could not initialize OBO request signing".to_owned()))?;
        mac.update(canonical.as_bytes());
        Ok(lower_hex(&mac.finalize().into_bytes()))
    }
}

fn canonical_timestamp(timestamp: i64) -> Result<String> {
    if timestamp <= 0 || OffsetDateTime::from_unix_timestamp(timestamp).is_err() {
        return Err(Error::Invalid(
            "an OBO timestamp must be a positive, representable Unix timestamp in seconds"
                .to_owned(),
        ));
    }
    if OffsetDateTime::now_utc()
        .unix_timestamp()
        .abs_diff(timestamp)
        > 60
    {
        return Err(Error::Invalid(
            "an OBO timestamp must be within 60 seconds of the local clock".to_owned(),
        ));
    }
    Ok(timestamp.to_string())
}

fn signing_endpoint_path<'a>(
    catalog: &'a models::OboEndpointCatalog,
    request: &models::OboExchangeRequest,
) -> Result<&'a str> {
    if catalog.application.app_id != request.audience {
        return Err(Error::Invalid(
            "the OBO endpoint catalog does not belong to the request audience".to_owned(),
        ));
    }
    let mut matches = catalog
        .endpoints
        .iter()
        .filter(|endpoint| endpoint.endpoint_id == request.endpoint_id);
    let endpoint = matches.next().ok_or_else(|| {
        Error::Invalid("the OBO endpoint is absent from the current audience catalog".to_owned())
    })?;
    if matches.next().is_some() {
        return Err(Error::Invalid(
            "the OBO endpoint catalog contains a duplicate endpoint identifier".to_owned(),
        ));
    }
    validate_request_path(&endpoint.path)?;
    Ok(&endpoint.path)
}

fn validate_canonical_method(method: &str) -> Result<()> {
    let valid = !method.is_empty()
        && method.len() <= 32
        && method.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_uppercase()
                || (index > 0
                    && (byte.is_ascii_digit()
                        || matches!(
                            byte,
                            b'!' | b'#'
                                | b'$'
                                | b'%'
                                | b'&'
                                | b'\''
                                | b'*'
                                | b'+'
                                | b'-'
                                | b'.'
                                | b'^'
                                | b'_'
                                | b'`'
                                | b'|'
                                | b'~'
                        )))
        });
    if valid {
        Ok(())
    } else {
        Err(Error::Invalid(
            "an OBO request method must be canonical uppercase HTTP syntax with at most 32 characters"
                .to_owned(),
        ))
    }
}

fn validate_body_sha256(digest: &str) -> Result<()> {
    if digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(Error::Invalid(
            "an OBO body SHA-256 must be exactly 64 lowercase hexadecimal characters".to_owned(),
        ))
    }
}

fn validate_request_path(path: &str) -> Result<()> {
    let valid = !path.is_empty()
        && path.len() <= 2_048
        && path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains(['?', '#'])
        && !path
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        && !path.split('/').any(|segment| matches!(segment, "." | ".."));
    if valid {
        Ok(())
    } else {
        Err(Error::Invalid(
            "an OBO endpoint path must be an absolute path without authority, query, fragment, whitespace, or dot segments"
                .to_owned(),
        ))
    }
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}
