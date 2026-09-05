use std::collections::BTreeMap;
use std::fmt;

use silicon_browser_shared::{ApiErrorEnvelope, FieldError};

#[derive(Debug)]
pub enum Error {
    Local(String),
    Transport(String),
    Api {
        code: String,
        message: String,
        fields: Box<[FieldError]>,
        details: Box<BTreeMap<String, String>>,
        request_id: Option<String>,
        retry_after_ms: Option<u64>,
    },
    Protocol(String),
}

impl Error {
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Api { code, .. } => Some(code),
            _ => None,
        }
    }

    pub fn request_id(&self) -> Option<&str> {
        match self {
            Self::Api { request_id, .. } => request_id.as_deref(),
            _ => None,
        }
    }

    pub fn fields(&self) -> &[FieldError] {
        match self {
            Self::Api { fields, .. } => fields.as_ref(),
            _ => &[],
        }
    }

    pub fn details(&self) -> Option<&BTreeMap<String, String>> {
        match self {
            Self::Api { details, .. } => Some(details.as_ref()),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local(message) => formatter.write_str(message),
            Self::Transport(message) => write!(formatter, "Silicon Browser could not be reached: {message}"),
            Self::Api { code, message, details, .. } => {
                write!(formatter, "{code}: {message}")?;
                if !details.is_empty() {
                    formatter.write_str(" (")?;
                    for (index, (key, value)) in details.iter().enumerate() {
                        if index > 0 {
                            formatter.write_str(", ")?;
                        }
                        write!(formatter, "{key}: {value}")?;
                    }
                    formatter.write_str(")")?;
                }
                Ok(())
            }
            Self::Protocol(message) => write!(formatter, "Silicon Browser returned an invalid response: {message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<ApiErrorEnvelope> for Error {
    fn from(envelope: ApiErrorEnvelope) -> Self {
        Self::Api {
            code: envelope.error.code,
            message: envelope.error.message,
            fields: envelope.error.fields.into_boxed_slice(),
            details: Box::new(envelope.error.details),
            request_id: envelope.error.request_id,
            retry_after_ms: envelope.error.retry_after_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use silicon_browser_shared::{ApiError, ApiErrorEnvelope};

    use super::*;

    /// Test group: structured conflict context remains programmatically
    /// available and the default CLI-facing rendering includes who owns the
    /// active slot and when it expires.
    #[test]
    fn api_error_preserves_and_renders_busy_context() {
        let mut details = BTreeMap::new();
        details.insert("actor_id".into(), "silicon-7".into());
        details.insert("expires_at".into(), "2026-09-04T12:00:00Z".into());
        details.insert("session_id".into(), "session-1".into());
        let error = Error::from(ApiErrorEnvelope {
            error: ApiError {
                code: "profile_busy".into(),
                message: "profile already has a live session".into(),
                fields: Vec::new(),
                details,
                request_id: Some("request-1".into()),
                retry_after_ms: None,
            },
        });
        assert_eq!(error.details().unwrap()["actor_id"], "silicon-7");
        let rendered = error.to_string();
        assert!(rendered.contains("silicon-7"));
        assert!(rendered.contains("2026-09-04T12:00:00Z"));
        assert!(rendered.contains("session-1"));
    }
}
