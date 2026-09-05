use thiserror::Error;

pub(crate) const MAX_IDENTIFIER_CHARS: usize = 512;

/// Input validation failure suitable for conversion into an API field error.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("{field} is required")]
    Required { field: &'static str },
    #[error("{field} must be at most {max} characters")]
    TooLong { field: &'static str, max: usize },
    #[error("{field} must contain at most {max} items")]
    TooMany { field: &'static str, max: usize },
    #[error("{field} must be between {min} and {max}")]
    OutOfRange { field: &'static str, min: u64, max: u64 },
    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },
    #[error("{left} conflicts with {right}")]
    Conflict { left: &'static str, right: &'static str },
}

pub trait Validate {
    fn validate(&self) -> Result<(), ValidationError>;
}

pub(crate) fn required(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if value.trim().is_empty() { Err(ValidationError::Required { field }) } else { Ok(()) }
}

pub(crate) fn bounded(value: &str, field: &'static str, max: usize) -> Result<(), ValidationError> {
    required(value, field)?;
    if value.chars().count() > max {
        return Err(ValidationError::TooLong { field, max });
    }
    Ok(())
}

pub(crate) fn collection_len(len: usize, field: &'static str, max: usize) -> Result<(), ValidationError> {
    if len > max {
        return Err(ValidationError::TooMany { field, max });
    }
    Ok(())
}

pub(crate) fn identifier(value: &str, field: &'static str) -> Result<(), ValidationError> {
    bounded(value, field, MAX_IDENTIFIER_CHARS)?;
    if value.chars().any(|ch| ch.is_control() || ch.is_whitespace() || matches!(ch, '/' | '\\')) {
        return Err(ValidationError::Invalid {
            field,
            reason: "must not contain whitespace, control characters, or path separators".into(),
        });
    }
    Ok(())
}

pub(crate) fn purpose(value: &str) -> Result<(), ValidationError> {
    bounded(value, "purpose", 2_000)
}
