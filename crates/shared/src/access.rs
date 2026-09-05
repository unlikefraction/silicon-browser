use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::validation::{MAX_IDENTIFIER_CHARS, collection_len, identifier};
use crate::{Identity, Validate, ValidationError};

/// Normalized union of explicit principals (`@id`) and IAM tags (`tag`).
///
/// Serialization is deliberately a plain string array so it maps directly to
/// `--access [@carbon,@silicon,growth]`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccessList(Vec<String>);

impl AccessList {
    /// Generous wire limit that bounds ACL evaluation and serialized profile rows.
    pub const MAX_ENTRIES: usize = 256;
    pub const MAX_ENTRY_CHARS: usize = MAX_IDENTIFIER_CHARS;

    pub fn new<I, S>(entries: I) -> Result<Self, ValidationError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut seen = HashSet::new();
        let mut normalized = Vec::new();
        for (index, entry) in entries.into_iter().enumerate() {
            collection_len(index + 1, "access", Self::MAX_ENTRIES)?;
            let entry = normalize_entry(entry.as_ref())?;
            if seen.insert(entry.clone()) {
                normalized.push(entry);
            }
        }
        Ok(Self(normalized))
    }

    /// Adds the immutable owner grant, retaining the first occurrence of every
    /// other grant. IAM IDs remain case-sensitive.
    pub fn with_owner<I, S>(owner_id: &str, entries: I) -> Result<Self, ValidationError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let owner = canonical_principal(owner_id)?;
        let entries = Self::new(entries)?;
        let mut values = vec![owner.clone()];
        values.extend(entries.into_iter().filter(|entry| entry != &owner));
        Self::new(values)
    }

    pub fn normalized_with_owner(&self, owner_id: &str) -> Result<Self, ValidationError> {
        Self::with_owner(owner_id, self.0.iter().map(String::as_str))
    }

    pub fn allows(&self, identity: &Identity) -> bool {
        identity.principal_ids().any(|principal_id| self.allows_principal_and_tags(principal_id, &identity.tags))
    }

    pub fn allows_principal_and_tags<S: AsRef<str>>(&self, principal_id: &str, tags: &[S]) -> bool {
        let Ok(principal) = canonical_principal(principal_id) else {
            return false;
        };
        self.0.iter().any(|grant| {
            grant == &principal
                || (!grant.starts_with('@') && tags.iter().any(|tag| tag.as_ref().trim() == grant.as_str()))
        })
    }

    pub fn contains_principal(&self, principal_id: &str) -> bool {
        canonical_principal(principal_id).is_ok_and(|principal| self.0.contains(&principal))
    }

    pub fn as_slice(&self) -> &[String] {
        &self.0
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl Validate for AccessList {
    fn validate(&self) -> Result<(), ValidationError> {
        collection_len(self.0.len(), "access", Self::MAX_ENTRIES)?;
        for entry in &self.0 {
            normalize_entry(entry)?;
        }
        Ok(())
    }
}

impl TryFrom<Vec<String>> for AccessList {
    type Error = ValidationError;

    fn try_from(value: Vec<String>) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl IntoIterator for AccessList {
    type Item = String;
    type IntoIter = std::vec::IntoIter<String>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

fn canonical_principal(value: &str) -> Result<String, ValidationError> {
    let value = value.trim().strip_prefix('@').unwrap_or(value.trim());
    identifier(value, "access")?;
    if value.starts_with('@') || has_access_delimiter(value) {
        return Err(invalid_access());
    }
    Ok(format!("@{value}"))
}

fn normalize_entry(value: &str) -> Result<String, ValidationError> {
    let value = value.trim();
    if value.is_empty() || has_access_delimiter(value) {
        return Err(invalid_access());
    }
    if let Some(principal) = value.strip_prefix('@') {
        if principal.starts_with('@') {
            return Err(invalid_access());
        }
        canonical_principal(principal)
    } else {
        if value.chars().count() > AccessList::MAX_ENTRY_CHARS {
            return Err(ValidationError::TooLong { field: "access", max: AccessList::MAX_ENTRY_CHARS });
        }
        if value.chars().any(|character| character.is_whitespace() || character.is_control()) {
            return Err(invalid_access());
        }
        Ok(value.to_owned())
    }
}

fn has_access_delimiter(value: &str) -> bool {
    value.chars().any(|ch| matches!(ch, ',' | '[' | ']'))
}

fn invalid_access() -> ValidationError {
    ValidationError::Invalid {
        field: "access",
        reason: "expected @principal or a tag without whitespace, controls, commas, or brackets".into(),
    }
}

#[cfg(test)]
mod access_list_tests {
    use super::*;
    use crate::IdentityKind;

    fn identity(id: &str, tags: &[&str]) -> Identity {
        Identity {
            id: id.into(),
            name: id.into(),
            kind: IdentityKind::Silicon,
            tags: tags.iter().map(ToString::to_string).collect(),
            verified_aliases: Vec::new(),
        }
    }

    /// Test group: profile access is a set union with an owner who cannot be omitted.
    #[test]
    fn owner_is_first_and_duplicates_are_removed() {
        let access = AccessList::with_owner("silicon-1", ["growth", "@silicon-1", "growth"]).unwrap();
        assert_eq!(access.as_slice(), ["@silicon-1", "growth"]);
    }

    /// Test group: either an explicit identity or any IAM tag grants access.
    #[test]
    fn matches_principals_and_tags() {
        let access = AccessList::new(["@carbon-1", "growth"]).unwrap();
        assert!(access.allows(&identity("carbon-1", &[])));
        assert!(access.allows(&identity("silicon-2", &["growth"])));
        assert!(!access.allows(&identity("silicon-3", &["sales"])));
    }

    /// Test group: malformed CLI list fragments never become ambiguous grants.
    #[test]
    fn rejects_empty_or_delimited_entries() {
        assert!(AccessList::new([""]).is_err());
        assert!(AccessList::new(["@one,@two"]).is_err());
        assert!(AccessList::new(["sales team"]).is_err());
        assert!(AccessList::new(["@@principal"]).is_err());
        assert!(AccessList::new(["sales\0team"]).is_err());
    }

    /// Test group: ACL work and persisted rows remain bounded before normalization.
    #[test]
    fn bounds_entry_count_and_size() {
        let entries: Vec<_> = (0..=AccessList::MAX_ENTRIES).map(|index| format!("team-{index}")).collect();
        assert!(AccessList::new(&entries[..AccessList::MAX_ENTRIES]).is_ok());
        assert_eq!(
            AccessList::new(&entries),
            Err(ValidationError::TooMany { field: "access", max: AccessList::MAX_ENTRIES })
        );
        assert!(AccessList::new(["x".repeat(AccessList::MAX_ENTRY_CHARS)]).is_ok());
        assert!(AccessList::new([format!("@{}", "x".repeat(AccessList::MAX_ENTRY_CHARS))]).is_ok());
        assert_eq!(
            AccessList::new(["x".repeat(AccessList::MAX_ENTRY_CHARS + 1)]),
            Err(ValidationError::TooLong { field: "access", max: AccessList::MAX_ENTRY_CHARS })
        );
    }

    #[test]
    fn full_acl_with_owner_can_be_normalized_again() {
        let entries = (0..AccessList::MAX_ENTRIES - 1).map(|index| format!("team-{index}"));
        let access = AccessList::with_owner("owner", entries).unwrap();
        assert_eq!(access.len(), AccessList::MAX_ENTRIES);
        assert_eq!(access.normalized_with_owner("owner").unwrap(), access);
        assert!(access.normalized_with_owner("another-owner").is_err());
    }

    /// Test group: access remains a minimal string-array wire representation.
    #[test]
    fn serializes_as_string_array() {
        let access = AccessList::new(["@one", "growth"]).unwrap();
        assert_eq!(serde_json::to_string(&access).unwrap(), r#"["@one","growth"]"#);
    }
}
