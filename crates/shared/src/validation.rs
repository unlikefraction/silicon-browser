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

/// Complete IAM actor ID. Organization authority is always a separate field.
pub fn actor_id(value: &str, field: &'static str) -> Result<crate::IdentityKind, ValidationError> {
    let parsed = value
        .strip_prefix("c:")
        .map(|handle| (handle, 30, crate::IdentityKind::Carbon))
        .or_else(|| value.strip_prefix("si:").map(|handle| (handle, 50, crate::IdentityKind::Silicon)));
    if let Some((handle, max, kind)) = parsed
        && (3..=max).contains(&handle.len())
        && handle.bytes().all(handle_byte)
    {
        return Ok(kind);
    }
    Err(ValidationError::Invalid { field, reason: "expected c:<carbon-handle> (3–30 characters) or si:<silicon-handle> (3–50 characters), using lowercase letters, digits, _ or -; migrate old selectors or sign in again".into() })
}

/// Bare IAM application handle; bundle and release selectors are separate grammars.
pub fn app_id(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if (1..=80).contains(&value.len()) && value.as_bytes()[0].is_ascii_lowercase() && value.bytes().all(handle_byte) {
        return Ok(());
    }
    Err(ValidationError::Invalid {
        field,
        reason: "expected a bare application ID of 1–80 lowercase letters, digits, _ or -, beginning with a letter"
            .into(),
    })
}

fn handle_byte(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
}

#[cfg(test)]
#[test]
fn public_identifiers_preserve_kinds_and_handle_limits() {
    use crate::IdentityKind::{Carbon, Silicon};
    assert_eq!(actor_id("c:alice0", "id"), Ok(Carbon));
    assert_eq!(actor_id("si:chef", "id"), Ok(Silicon));
    for (prefix, max) in [("c:", 30), ("si:", 50)] {
        assert!(actor_id(&format!("{prefix}{}", "a".repeat(max)), "id").is_ok());
        assert!(actor_id(&format!("{prefix}{}", "a".repeat(max + 1)), "id").is_err());
    }
    for value in ["alice", "chef:tos", "c:ab", "si:ab", "si:Chef", "c:alice:tos", "@c:alice"] {
        assert!(actor_id(value, "id").is_err(), "{value}");
    }
    for value in ["browser", "briefcase", "a", &"a".repeat(80)] {
        assert!(app_id(value, "app").is_ok(), "{value}");
    }
    for value in ["", "tos>browser", "browser>test@1.0.0", "1browser", "Browser", &"a".repeat(81)] {
        assert!(app_id(value, "app").is_err(), "{value}");
    }
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
