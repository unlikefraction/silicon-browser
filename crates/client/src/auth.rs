use crate::Error;

/// An opaque, short-lived bearer. Debug output is deliberately redacted.
#[derive(Clone)]
pub struct Auth(String);

impl Auth {
    pub fn new(token: impl Into<String>) -> Result<Self, Error> {
        let token = token.into();
        let token = token.trim();
        if token.is_empty()
            || token.len() > 16 * 1024
            || token.chars().any(|character| character.is_whitespace() || character.is_control())
        {
            return Err(Error::Local(
                "auth token must be one bounded, non-empty value without whitespace or controls".into(),
            ));
        }
        Ok(Self(token.to_owned()))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Auth([REDACTED])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_validated_and_never_debugged() {
        assert!(Auth::new("").is_err());
        assert!(Auth::new("two words").is_err());
        let auth = Auth::new("oat_do-not-print").unwrap();
        assert_eq!(format!("{auth:?}"), "Auth([REDACTED])");
        assert!(!format!("{auth:?}").contains("do-not-print"));
    }
}
