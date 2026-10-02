//! Failures this client can report.

use std::{fmt, time::Duration};

use serde::Deserialize;

/// Anything that can go wrong on the way to a result.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The service answered with its error envelope.
    ///
    /// Boxed so that `Result<T, Error>` stays small: every method in this
    /// crate returns one, and an inline envelope would widen all of them.
    #[error("{0}")]
    Api(Box<ApiError>),

    /// An abuse-control limit was reached. Held separate from
    /// [`Error::Api`] because it is the one failure whose remedy is
    /// mechanical: wait the stated interval and repeat the same request.
    #[error("rate limited; retry in {}s", retry_after.as_secs())]
    RateLimited {
        /// How long to wait before repeating the request.
        retry_after: Duration,
        /// Requests permitted in the current window.
        limit: Option<u64>,
        /// Requests still available in the current window.
        remaining: Option<u64>,
        /// The envelope the service sent alongside the limit.
        source: Box<ApiError>,
    },

    /// The request never completed: connection, TLS, or timeout.
    #[error("the request did not reach a response: {0}")]
    Transport(#[source] reqwest::Error),

    /// A response arrived but did not match the contract.
    #[error("the response could not be understood: {0}")]
    Decode(String),

    /// An HTTP failure arrived without IAM's structured error envelope.
    ///
    /// This is not evidence that IAM rejected the caller's authority. An edge
    /// proxy, gateway, or incompatible service may have generated the response.
    /// Its raw body is deliberately not retained or displayed.
    #[error(
        "HTTP {status} response without a Silicon IAM error envelope; an intermediary or incompatible service may have answered (request ID: {})",
        request_id.as_deref().unwrap_or("not provided")
    )]
    UnstructuredResponse {
        /// HTTP status actually received.
        status: u16,
        /// A well-formed UUID from the response's `X-Request-Id`, if supplied.
        /// This is a correlation hint, not proof the response came from IAM.
        request_id: Option<String>,
    },

    /// A response body exceeded the client's fixed memory-safety bound.
    #[error("the response body exceeded the {limit}-byte limit")]
    ResponseTooLarge {
        /// Maximum body size accepted by this client release.
        limit: usize,
    },

    /// The client and service share no API version.
    #[error("no mutually supported API version; the service offers {}", offered.join(", "))]
    ApiVersionUnsupported {
        /// Versions the service advertised.
        offered: Vec<String>,
    },

    /// A value supplied to this client cannot be used to build a request.
    #[error("{0}")]
    Invalid(String),
}

/// The service's structured error envelope.
///
/// Every failing route answers with this shape, so `code` is the value worth
/// matching on. `message` is written for a person and may change; `code` is
/// part of the contract.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub struct ApiError {
    /// HTTP status that carried the envelope.
    pub status: u16,
    /// Stable machine-readable code, such as `etag_mismatch`.
    pub code: String,
    /// Human-readable description.
    pub message: String,
    /// Field-level or contextual detail, when the route supplies any.
    pub details: Option<serde_json::Value>,
    /// Correlation identifier to quote when reporting the failure.
    pub request_id: Option<String>,
}

impl fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} ({})", self.message, self.code)?;
        if let Some(fields) = self
            .details
            .as_ref()
            .and_then(|details| details.get("fields"))
            .and_then(serde_json::Value::as_array)
        {
            for field in fields {
                let name = field.get("field").and_then(serde_json::Value::as_str);
                let message = field.get("message").and_then(serde_json::Value::as_str);
                if let (Some(name), Some(message)) = (name, message) {
                    write!(formatter, " {name}: {message}.")?;
                }
            }
        }
        if let Some(request_id) = &self.request_id {
            write!(formatter, " [request {request_id}]")?;
        }
        Ok(())
    }
}

impl ApiError {
    /// Whether the credential was missing, expired, or revoked.
    #[must_use]
    pub fn is_unauthenticated(&self) -> bool {
        self.status == 401
    }

    /// Whether the caller was recognized but not permitted.
    #[must_use]
    pub fn is_forbidden(&self) -> bool {
        self.status == 403
    }

    /// Whether the resource does not exist, or is not visible to this caller.
    ///
    /// Silicon IAM answers 404 rather than 403 wherever admitting existence
    /// would itself disclose something, so this can mean either.
    #[must_use]
    pub fn is_not_found(&self) -> bool {
        self.status == 404
    }

    /// Whether an `If-Match` version no longer matched the resource.
    #[must_use]
    pub fn is_version_conflict(&self) -> bool {
        self.status == 412 && matches!(self.code.as_str(), "etag_mismatch" | "version_mismatch")
    }

    /// Whether the route requires a step-up assertion this request lacked.
    #[must_use]
    pub fn requires_step_up(&self) -> bool {
        self.code == "step_up_required"
            || self.code == "step_up_invalid"
            || (self.code == "precondition_required"
                && self
                    .details
                    .as_ref()
                    .and_then(|details| details.get("precondition"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|precondition| {
                        precondition.eq_ignore_ascii_case("X-Step-Up-Token")
                    }))
    }

    /// Whether the same idempotency key was reused with a different body.
    #[must_use]
    pub fn is_idempotency_conflict(&self) -> bool {
        self.code == "idempotency_conflict"
    }

    /// Whether repeating the identical request could plausibly succeed.
    ///
    /// True only for the service's own "try again" signals. A 4xx is never
    /// retryable: the request has to change first.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self.status, 429 | 502 | 503 | 504)
    }
}

impl From<ApiError> for Error {
    fn from(error: ApiError) -> Self {
        Self::Api(Box::new(error))
    }
}

#[derive(Deserialize)]
pub(crate) struct Envelope {
    pub(crate) error: EnvelopeBody,
}

#[derive(Deserialize)]
pub(crate) struct EnvelopeBody {
    pub(crate) code: String,
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) details: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) request_id: Option<String>,
}

/// Result alias used throughout this crate.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::{ApiError, Error};

    fn api(status: u16, code: &str) -> ApiError {
        ApiError {
            status,
            code: code.to_owned(),
            message: "something went wrong".to_owned(),
            details: None,
            request_id: Some("01a0-req".to_owned()),
        }
    }

    #[test]
    fn a_client_error_is_never_reported_as_retryable() {
        for status in [400, 401, 403, 404, 409, 412, 422, 428] {
            assert!(!api(status, "whatever").is_retryable(), "{status}");
        }
        for status in [429, 502, 503, 504] {
            assert!(api(status, "whatever").is_retryable(), "{status}");
        }
    }

    #[test]
    fn classifiers_read_status_and_code_together() {
        assert!(api(412, "etag_mismatch").is_version_conflict());
        // A precondition failure that is not about the version must not be
        // mistaken for one; the remedy is different.
        assert!(!api(412, "step_up_invalid").is_version_conflict());
        assert!(api(412, "step_up_invalid").requires_step_up());
        assert!(api(401, "unauthenticated").is_unauthenticated());
        assert!(api(404, "not_found").is_not_found());
    }

    #[test]
    fn display_quotes_the_request_id_for_a_report() {
        let rendered = api(409, "conflict").to_string();
        assert!(rendered.contains("conflict"), "{rendered}");
        assert!(rendered.contains("01a0-req"), "{rendered}");
    }

    #[test]
    fn display_surfaces_safe_field_validation_details() {
        let mut error = api(422, "validation_failed");
        error.details = Some(serde_json::json!({
            "fields": [{
                "field": "app_id",
                "message": "must be a qualified Application ID"
            }]
        }));
        let rendered = error.to_string();
        assert!(rendered.contains("app_id"), "{rendered}");
        assert!(rendered.contains("must be a qualified"), "{rendered}");
    }

    #[test]
    fn an_unsupported_version_names_what_the_service_offers() {
        let error = Error::ApiVersionUnsupported {
            offered: vec!["v2".to_owned(), "v3".to_owned()],
        };
        assert!(error.to_string().contains("v2, v3"), "{error}");
    }
}
