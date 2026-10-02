//! What can go wrong, said plainly enough to act on.

use std::fmt;

use serde::Deserialize;

/// The result of any client operation.
pub type Result<T> = std::result::Result<T, Error>;

/// A failed client operation.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Briefcase answered, and said no.
    #[error("{0}")]
    Api(#[from] ApiError),

    /// The server and this build do not agree on the contract.
    ///
    /// Returned before the first real call, so an incompatible pairing never
    /// half-succeeds.
    #[error("{0}")]
    Incompatible(#[from] IncompatibleContract),

    /// The request never produced an answer.
    #[error("briefcase could not be reached: {reason}")]
    Transport {
        /// What went wrong at the transport layer.
        reason: String,
        /// The underlying transport failure.
        #[source]
        source: reqwest::Error,
    },

    /// A local file could not be read or written.
    #[error("local file operation failed: {path}")]
    Io {
        /// The path being read or written.
        path: String,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// The client was configured with something it cannot use.
    #[error("invalid client configuration: {0}")]
    Configuration(String),

    /// Briefcase answered with a body this build cannot read.
    #[error("briefcase returned an unreadable response: {0}")]
    Protocol(String),
}

impl Error {
    /// Returns the error code Briefcase named, when it named one.
    ///
    /// Codes are stable, so this is what a caller matches on to tell a spent
    /// upload allowance from a missing file.
    #[must_use]
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Api(error) => Some(error.code.as_str()),
            _ => None,
        }
    }

    /// Returns whether the target does not exist, or is hidden from the caller.
    ///
    /// Briefcase reports both the same way on purpose: an entry you may not
    /// read is never confirmed to exist.
    #[must_use]
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Api(error) if error.status == 404)
    }

    /// Returns whether the caller is authenticated but not allowed to do this.
    #[must_use]
    pub fn is_forbidden(&self) -> bool {
        matches!(self, Self::Api(error) if error.status == 403)
    }

    /// Returns whether the credential was missing, expired, or rejected.
    #[must_use]
    pub fn is_unauthenticated(&self) -> bool {
        matches!(self, Self::Api(error) if error.status == 401)
    }

    /// Returns whether retrying later is the right response.
    ///
    /// A spent daily upload allowance and an unavailable dependency both say
    /// "not now"; a refused permission never will.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Api(error) if error.status == 429 || error.status >= 500)
            || matches!(self, Self::Transport { .. })
    }

    /// Returns how long to wait before retrying, when Briefcase said.
    #[must_use]
    pub const fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            Self::Api(error) => error.retry_after,
            _ => None,
        }
    }
}

/// A refusal Briefcase described in its own error envelope.
#[derive(Clone, Debug, thiserror::Error)]
pub struct ApiError {
    /// HTTP status.
    pub status: u16,
    /// Stable error code, such as `daily_upload_limit_exhausted`.
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// Correlation identifier to quote when asking an operator about it.
    pub request_id: Option<String>,
    /// How long to wait before retrying, when the answer carried it.
    pub retry_after: Option<std::time::Duration>,
    /// Authoritative file length from a `416` response's `Content-Range`.
    ///
    /// Absent when the server omitted the header or did not send `bytes */N`.
    pub unsatisfied_range_length: Option<u64>,
}

impl fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} ({}", self.message, self.code)?;
        if let Some(request_id) = &self.request_id {
            write!(formatter, ", request {request_id}")?;
        }
        write!(formatter, ")")
    }
}

/// The server serves a contract this build was not written against.
#[derive(Clone, Debug, thiserror::Error)]
pub struct IncompatibleContract {
    /// API majors the server serves.
    pub served_api_versions: Vec<String>,
    /// Service, negotiation, or operation catalog items that differ from this build.
    pub mismatched_operations: Vec<OperationMismatch>,
    /// Operations this build calls that the server does not serve at all.
    pub missing_operations: Vec<String>,
}

impl fmt::Display for IncompatibleContract {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "briefcase serves a contract this client was not built for (serving {})",
            self.served_api_versions.join(", ")
        )?;
        for mismatch in &self.mismatched_operations {
            write!(
                formatter,
                "; {} is {} here and {} there",
                mismatch.id, mismatch.expected, mismatch.served
            )?;
        }
        for missing in &self.missing_operations {
            write!(formatter, "; {missing} is not served")?;
        }
        Ok(())
    }
}

/// One contract catalog item this build does not match.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationMismatch {
    /// Operation identifier, or the service/negotiation field name.
    pub id: String,
    /// Exact value or operation signature this build expects.
    pub expected: String,
    /// Value or operation signature the server serves.
    pub served: String,
}

#[derive(Deserialize)]
pub(crate) struct WireErrorEnvelope {
    pub(crate) error: WireError,
}

#[derive(Deserialize)]
pub(crate) struct WireError {
    pub(crate) code: String,
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) request_id: Option<String>,
}

pub(crate) fn transport(source: reqwest::Error) -> Error {
    let reason = if source.is_timeout() {
        "the request timed out"
    } else if source.is_connect() {
        "the connection could not be established"
    } else if source.is_body() || source.is_decode() {
        "the response could not be read"
    } else {
        "the request failed"
    };
    Error::Transport {
        reason: reason.to_owned(),
        source,
    }
}

pub(crate) fn io(path: impl Into<String>, source: std::io::Error) -> Error {
    Error::Io {
        path: path.into(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::{ApiError, Error};

    fn api(status: u16, code: &str) -> Error {
        Error::Api(ApiError {
            status,
            code: code.to_owned(),
            message: "refused".to_owned(),
            request_id: Some("01a0".to_owned()),
            retry_after: None,
            unsatisfied_range_length: None,
        })
    }

    #[test]
    fn callers_can_tell_the_answers_apart() {
        assert!(api(404, "not_found").is_not_found());
        assert!(api(403, "forbidden").is_forbidden());
        assert!(api(401, "unauthenticated").is_unauthenticated());
        assert!(api(429, "daily_upload_limit_exhausted").is_retryable());
        assert!(api(503, "dependency_unavailable").is_retryable());
        assert!(!api(403, "forbidden").is_retryable());
        assert_eq!(
            api(507, "storage_limit_exhausted").code(),
            Some("storage_limit_exhausted")
        );
    }

    #[test]
    fn an_error_reads_as_a_sentence_with_its_request_id() {
        assert_eq!(
            api(404, "not_found").to_string(),
            "refused (not_found, request 01a0)"
        );
    }
}
