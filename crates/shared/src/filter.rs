use std::str::FromStr;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{Identity, Recording, Session, SessionStatus, Usage};

const MAX_FILTER_CHARS: usize = 16_384;
const MAX_FILTER_STAGES: usize = 64;
const MAX_FILTER_VALUE_CHARS: usize = 4_096;
const MAX_FILTER_PRINCIPAL_CHARS: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum FilterError {
    #[error("filter stage {stage} is empty")]
    EmptyStage { stage: usize },
    #[error("filter stage {stage} must use key:value")]
    MissingColon { stage: usize },
    #[error("{service} filters do not support {key}")]
    Unsupported { service: &'static str, key: String },
    #[error("invalid {key} filter: {reason}")]
    Invalid { key: String, reason: String },
}

/// Case-insensitive CLI text pattern. `^text` anchors a prefix and `*` is a
/// zero-or-more wildcard; an unadorned pattern is an exact match.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TextPattern(String);

impl TextPattern {
    pub fn new(value: impl Into<String>) -> Result<Self, FilterError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(FilterError::Invalid { key: "pattern".into(), reason: "value is required".into() });
        }
        if value.chars().count() > MAX_FILTER_VALUE_CHARS {
            return Err(FilterError::Invalid {
                key: "pattern".into(),
                reason: format!("must be at most {MAX_FILTER_VALUE_CHARS} characters"),
            });
        }
        Ok(Self(value.trim().to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn matches(&self, candidate: &str) -> bool {
        let pattern = self.0.to_lowercase();
        let candidate = candidate.to_lowercase();
        if let Some(prefix) = pattern.strip_prefix('^') {
            if prefix.contains('*') {
                return glob_matches(prefix, &candidate, true);
            }
            return candidate.starts_with(prefix);
        }
        glob_matches(&pattern, &candidate, false)
    }
}

fn glob_matches(pattern: &str, candidate: &str, prefix_only: bool) -> bool {
    if !pattern.contains('*') {
        return pattern == candidate;
    }

    let starts_with_wildcard = pattern.starts_with('*');
    let ends_with_wildcard = pattern.ends_with('*');
    let parts: Vec<_> = pattern.split('*').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return true;
    }

    let mut offset = 0;
    for (index, part) in parts.iter().enumerate() {
        let Some(found) = candidate[offset..].find(part) else {
            return false;
        };
        if index == 0 && !starts_with_wildcard && found != 0 {
            return false;
        }
        offset += found + part.len();
    }

    prefix_only || ends_with_wildcard || offset == candidate.len()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionFacet {
    Active,
    Ended,
    Expired,
    Incognito,
    Mine,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "field", content = "value", rename_all = "snake_case")]
pub enum SessionPredicate {
    Is(SessionFacet),
    For(String),
    Name(TextPattern),
    Description(TextPattern),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionFilter {
    pub predicates: Vec<SessionPredicate>,
}

impl SessionFilter {
    pub fn parse(value: &str) -> Result<Self, FilterError> {
        value.parse()
    }

    /// Every pipeline stage narrows the preceding result, so predicates are ANDed.
    pub fn matches(&self, session: &Session, viewer_id: &str) -> bool {
        self.predicates.iter().all(|predicate| match predicate {
            SessionPredicate::Is(facet) => match facet {
                SessionFacet::Active => session.status == SessionStatus::Active,
                SessionFacet::Ended => session.status == SessionStatus::Ended,
                SessionFacet::Expired => session.status == SessionStatus::Expired,
                SessionFacet::Incognito => session.incognito,
                SessionFacet::Mine => session.is_participant(viewer_id),
            },
            SessionPredicate::For(identity_id) => session.is_participant(identity_id),
            SessionPredicate::Name(pattern) => pattern.matches(&session.name),
            SessionPredicate::Description(pattern) => pattern.matches(&session.description),
        })
    }
}

impl FromStr for SessionFilter {
    type Err = FilterError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut predicates = Vec::new();
        for (stage, key, value) in stages(value)? {
            let predicate = match key.as_str() {
                "is" => SessionPredicate::Is(match value.to_ascii_lowercase().as_str() {
                    "active" => SessionFacet::Active,
                    "ended" => SessionFacet::Ended,
                    "expired" => SessionFacet::Expired,
                    "incognito" => SessionFacet::Incognito,
                    "mine" => SessionFacet::Mine,
                    _ => return Err(invalid(&key, "expected active, ended, expired, incognito, or mine")),
                }),
                "for" => SessionPredicate::For(principal(&key, &value)?),
                "name" => SessionPredicate::Name(pattern(&key, &value)?),
                "description" => SessionPredicate::Description(pattern(&key, &value)?),
                _ => return Err(FilterError::Unsupported { service: "session", key: stage_key(stage, key) }),
            };
            predicates.push(predicate);
        }
        Ok(Self { predicates })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingFacet {
    Incognito,
    Mine,
    Shared,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "field", content = "value", rename_all = "snake_case")]
pub enum RecordingPredicate {
    Contains(String),
    Profile(String),
    For(String),
    Name(TextPattern),
    Description(TextPattern),
    Is(RecordingFacet),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingFilter {
    pub predicates: Vec<RecordingPredicate>,
}

impl RecordingFilter {
    pub fn parse(value: &str) -> Result<Self, FilterError> {
        value.parse()
    }

    /// The caller must establish visibility before applying this filter.
    /// `is:shared` consequently means visible to `viewer`, but not owned by it,
    /// regardless of whether visibility came from participation or profile ACLs.
    pub fn matches(&self, recording: &Recording, viewer: &Identity) -> bool {
        self.predicates.iter().all(|predicate| match predicate {
            RecordingPredicate::Contains(needle) => {
                let needle = needle.to_lowercase();
                recording.session_name.to_lowercase().contains(&needle)
                    || recording.session_description.to_lowercase().contains(&needle)
            }
            RecordingPredicate::Profile(profile_id) => recording.profile_id.as_deref() == Some(profile_id),
            RecordingPredicate::For(identity_id) => recording.is_participant(identity_id),
            RecordingPredicate::Name(pattern) => pattern.matches(&recording.session_name),
            RecordingPredicate::Description(pattern) => pattern.matches(&recording.session_description),
            RecordingPredicate::Is(RecordingFacet::Incognito) => recording.incognito,
            RecordingPredicate::Is(RecordingFacet::Mine) => viewer.matches_principal(&recording.owner_id),
            RecordingPredicate::Is(RecordingFacet::Shared) => !viewer.matches_principal(&recording.owner_id),
        })
    }
}

impl FromStr for RecordingFilter {
    type Err = FilterError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut predicates = Vec::new();
        for (stage, key, value) in stages(value)? {
            let predicate = match key.as_str() {
                "contains" if value.chars().count() > MAX_FILTER_VALUE_CHARS => {
                    return Err(invalid(&key, &format!("must be at most {MAX_FILTER_VALUE_CHARS} characters")));
                }
                "contains" if !value.is_empty() => RecordingPredicate::Contains(value),
                "contains" => return Err(invalid(&key, "value is required")),
                "profile" => RecordingPredicate::Profile(resource_id(&key, &value, "profile id")?),
                "for" => RecordingPredicate::For(principal(&key, &value)?),
                "name" => RecordingPredicate::Name(pattern(&key, &value)?),
                "description" => RecordingPredicate::Description(pattern(&key, &value)?),
                "is" => RecordingPredicate::Is(match value.to_ascii_lowercase().as_str() {
                    "incognito" => RecordingFacet::Incognito,
                    "mine" => RecordingFacet::Mine,
                    "shared" => RecordingFacet::Shared,
                    _ => return Err(invalid(&key, "expected incognito, mine, or shared")),
                }),
                _ => return Err(FilterError::Unsupported { service: "recording", key: stage_key(stage, key) }),
            };
            predicates.push(predicate);
        }
        Ok(Self { predicates })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "field", content = "value", rename_all = "snake_case")]
pub enum UsagePredicate {
    Between { start: NaiveDate, end: NaiveDate },
    For(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageFilter {
    pub predicates: Vec<UsagePredicate>,
}

impl UsageFilter {
    pub fn parse(value: &str) -> Result<Self, FilterError> {
        value.parse()
    }

    pub fn matches(&self, usage: &Usage) -> bool {
        self.predicates.iter().all(|predicate| match predicate {
            UsagePredicate::Between { start, end } => {
                let date = usage.started_at.date_naive();
                (*start..=*end).contains(&date)
            }
            UsagePredicate::For(identity_id) => usage.is_for(identity_id),
        })
    }
}

impl FromStr for UsageFilter {
    type Err = FilterError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut predicates = Vec::new();
        for (stage, key, value) in stages(value)? {
            let predicate = match key.as_str() {
                "between" => {
                    let (start, end) =
                        value.split_once('=').ok_or_else(|| invalid(&key, "expected DD-MM-YYYY=DD-MM-YYYY"))?;
                    let start = parse_date(&key, start)?;
                    let end = parse_date(&key, end)?;
                    if start > end {
                        return Err(invalid(&key, "start must not be later than end"));
                    }
                    UsagePredicate::Between { start, end }
                }
                "for" => UsagePredicate::For(principal(&key, &value)?),
                _ => return Err(FilterError::Unsupported { service: "usage", key: stage_key(stage, key) }),
            };
            predicates.push(predicate);
        }
        Ok(Self { predicates })
    }
}

fn stages(value: &str) -> Result<Vec<(usize, String, String)>, FilterError> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    if value.chars().count() > MAX_FILTER_CHARS {
        return Err(invalid("filter", &format!("must be at most {MAX_FILTER_CHARS} characters")));
    }
    value
        .split("->")
        .enumerate()
        .map(|(index, raw)| {
            let stage = index + 1;
            if stage > MAX_FILTER_STAGES {
                return Err(invalid("filter", &format!("must contain at most {MAX_FILTER_STAGES} stages")));
            }
            let raw = raw.trim();
            if raw.is_empty() {
                return Err(FilterError::EmptyStage { stage });
            }
            let (key, value) = raw.split_once(':').ok_or(FilterError::MissingColon { stage })?;
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim().to_owned();
            if key.is_empty() {
                return Err(FilterError::MissingColon { stage });
            }
            Ok((stage, key, value))
        })
        .collect()
}

fn principal(key: &str, value: &str) -> Result<String, FilterError> {
    let value = value.trim().trim_start_matches('@');
    if value.is_empty()
        || value.chars().count() > MAX_FILTER_PRINCIPAL_CHARS
        || value.chars().any(|ch| ch.is_control() || ch.is_whitespace() || matches!(ch, '/' | '\\'))
    {
        return Err(invalid(key, "expected @identity"));
    }
    Ok(value.to_owned())
}

fn resource_id(key: &str, value: &str, expected: &str) -> Result<String, FilterError> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > MAX_FILTER_PRINCIPAL_CHARS
        || value.chars().any(|ch| ch.is_control() || ch.is_whitespace() || matches!(ch, '/' | '\\'))
    {
        return Err(invalid(key, &format!("expected {expected}")));
    }
    Ok(value.to_owned())
}

fn pattern(key: &str, value: &str) -> Result<TextPattern, FilterError> {
    TextPattern::new(value).map_err(|_| invalid(key, "value is required"))
}

fn parse_date(key: &str, value: &str) -> Result<NaiveDate, FilterError> {
    NaiveDate::parse_from_str(value.trim(), "%d-%m-%Y").map_err(|_| invalid(key, "expected a valid DD-MM-YYYY date"))
}

fn invalid(key: &str, reason: &str) -> FilterError {
    FilterError::Invalid { key: key.to_owned(), reason: reason.to_owned() }
}

fn stage_key(stage: usize, key: String) -> String {
    format!("{key} (stage {stage})")
}

#[cfg(test)]
mod filter_tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::{IdentityKind, Money, ProxyLocation, RecordingStatus, SessionTtl, UsageCost, UsageTotal};

    fn identity(id: &str) -> Identity {
        Identity {
            id: id.into(),
            name: id.into(),
            kind: IdentityKind::Silicon,
            tags: Vec::new(),
            verified_aliases: Vec::new(),
        }
    }

    fn session() -> Session {
        let started_at = Utc.with_ymd_and_hms(2026, 8, 10, 12, 0, 0).unwrap();
        Session {
            id: "session-1".into(),
            profile_id: Some("profile-1".into()),
            incognito: false,
            location: Some(ProxyLocation { code: "in".into(), name: "India".into(), country: Some("IN".into()) }),
            name: "Market scan".into(),
            description: "Research browser vendors".into(),
            status: SessionStatus::Active,
            initiator_id: "silicon-1".into(),
            participant_ids: vec!["carbon-1".into()],
            ttl: SessionTtl::Minutes30,
            started_at,
            expires_at: started_at + chrono::Duration::minutes(30),
            ended_at: None,
            end_note: None,
            usage: UsageTotal::default(),
        }
    }

    fn recording() -> Recording {
        Recording {
            session_id: "session-1".into(),
            profile_id: Some("profile-1".into()),
            incognito: false,
            session_name: "Market scan".into(),
            session_description: "Research browser vendors".into(),
            owner_id: "silicon-1".into(),
            participant_ids: vec!["silicon-1".into(), "carbon-1".into()],
            briefcase_path: "private/silicon-1/sb/session-1".into(),
            briefcase_link: None,
            command_log_link: None,
            command_log_path: None,
            delivery_error: None,
            duration_seconds: 60,
            size_bytes: 100,
            status: RecordingStatus::Pending,
            created_at: Utc.with_ymd_and_hms(2026, 8, 10, 12, 0, 0).unwrap(),
            trashed_at: None,
            purge_at: None,
        }
    }

    fn usage() -> Usage {
        Usage {
            session_id: "session-1".into(),
            started_at: Utc.with_ymd_and_hms(2026, 8, 10, 12, 0, 0).unwrap(),
            principal_ids: vec!["silicon-1".into(), "carbon-1".into()],
            browser_seconds: 60,
            proxy_bytes_in: 0,
            proxy_bytes_out: 0,
            proxy_bytes_unclassified: 0,
            cost: UsageCost {
                browser: Money::default(),
                proxy_in: Money::default(),
                proxy_out: Money::default(),
                proxy_unclassified: Money::default(),
                total: Money::default(),
            },
        }
    }

    /// Test group: session pipeline stages are service-specific and ANDed.
    #[test]
    fn session_filters_status_people_and_text() {
        let filter =
            SessionFilter::parse("is: active -> for: @carbon-1 -> name:market* -> description:^research").unwrap();
        assert!(filter.matches(&session(), "someone-else"));

        let ended = SessionFilter::parse("is:ended").unwrap();
        assert!(!ended.matches(&session(), "silicon-1"));
        let mine = SessionFilter::parse("is:mine").unwrap();
        assert!(mine.matches(&session(), "carbon-1"));
    }

    /// Test group: incognito is a mode facet, independent of terminal status.
    #[test]
    fn session_incognito_filter_uses_mode() {
        let mut value = session();
        value.profile_id = None;
        value.location = None;
        value.incognito = true;
        value.status = SessionStatus::Expired;
        assert!(SessionFilter::parse("is:incognito").unwrap().matches(&value, "silicon-1"));
        assert!(SessionFilter::parse("is:expired").unwrap().matches(&value, "silicon-1"));
    }

    /// Test group: recording contains searches both discoverability fields.
    #[test]
    fn recording_contains_name_or_description() {
        let viewer = identity("silicon-1");
        assert!(RecordingFilter::parse("contains:market").unwrap().matches(&recording(), &viewer));
        assert!(RecordingFilter::parse("contains:VENDOR").unwrap().matches(&recording(), &viewer));
        assert!(!RecordingFilter::parse("contains:checkout").unwrap().matches(&recording(), &viewer));
    }

    /// Test group: recording discovery composes profile, actor, and source-session metadata.
    #[test]
    fn recording_filters_profile_actor_text_and_incognito() {
        let viewer = identity("silicon-1");
        let filter =
            RecordingFilter::parse("profile:profile-1 -> for:@carbon-1 -> name:market* -> description:^research")
                .unwrap();
        assert!(filter.matches(&recording(), &viewer));
        assert!(!RecordingFilter::parse("profile:profile-2").unwrap().matches(&recording(), &viewer));

        let mut owner_is_implicit_actor = recording();
        owner_is_implicit_actor.participant_ids.retain(|participant| participant != "silicon-1");
        assert!(RecordingFilter::parse("for:@silicon-1").unwrap().matches(&owner_is_implicit_actor, &viewer));

        let mut incognito = recording();
        incognito.profile_id = None;
        incognito.incognito = true;
        assert!(RecordingFilter::parse("is:incognito").unwrap().matches(&incognito, &viewer));
        assert!(!RecordingFilter::parse("profile:profile-1").unwrap().matches(&incognito, &viewer));
    }

    /// Test group: shared means an already-visible recording not owned by the
    /// viewer, including profile-ACL visibility without session participation.
    #[test]
    fn recording_mine_and_shared_are_viewer_relative() {
        let owner = identity("silicon-1");
        let acl_viewer = identity("acl-viewer");
        assert!(RecordingFilter::parse("is:mine").unwrap().matches(&recording(), &owner));
        assert!(RecordingFilter::parse("is:shared").unwrap().matches(&recording(), &acl_viewer));
        assert!(!RecordingFilter::parse("is:shared").unwrap().matches(&recording(), &owner));

        let mut aliased_owner = identity("public-silicon");
        aliased_owner.verified_aliases.push("silicon-1".into());
        assert!(RecordingFilter::parse("is:mine").unwrap().matches(&recording(), &aliased_owner));
        assert!(!RecordingFilter::parse("is:shared").unwrap().matches(&recording(), &aliased_owner));
    }

    /// Test group: usage date windows are inclusive and compose with actor selection.
    #[test]
    fn usage_between_and_for_are_inclusive() {
        let filter = UsageFilter::parse("between:10-08-2026=10-08-2026 -> for:@carbon-1").unwrap();
        assert!(filter.matches(&usage()));
        assert!(!UsageFilter::parse("between:11-08-2026=12-08-2026").unwrap().matches(&usage()));
    }

    /// Test group: parsers reject cross-service predicates and invalid ranges.
    #[test]
    fn invalid_service_predicates_do_not_leak_between_services() {
        assert!(SessionFilter::parse("contains:research").is_err());
        assert!(RecordingFilter::parse("between:01-08-2026=30-08-2026").is_err());
        assert!(RecordingFilter::parse("profile:profile/escape").is_err());
        assert!(UsageFilter::parse("between:30-08-2026=01-08-2026").is_err());
        assert!(UsageFilter::parse("between:31-02-2026=01-03-2026").is_err());
    }

    /// Test group: an omitted filter is represented by an empty, match-all pipeline.
    #[test]
    fn empty_filters_match_everything() {
        assert!(SessionFilter::parse("").unwrap().matches(&session(), "nobody"));
        assert!(RecordingFilter::parse("  ").unwrap().matches(&recording(), &identity("nobody")));
        assert!(UsageFilter::default().matches(&usage()));
    }

    /// Test group: wildcard and caret examples in UNDERSTANDING.md have prefix semantics.
    #[test]
    fn documented_text_patterns_match() {
        assert!(TextPattern::new("market*").unwrap().matches("Market scan"));
        assert!(TextPattern::new("^research").unwrap().matches("Research browser vendors"));
        assert!(!TextPattern::new("market*").unwrap().matches("A market scan"));
    }

    /// Test group: filter pipelines, patterns, and principals have finite parsing work.
    #[test]
    fn filter_input_work_is_bounded() {
        let too_many_stages = vec!["is:active"; MAX_FILTER_STAGES + 1].join(" -> ");
        assert!(SessionFilter::parse(&too_many_stages).is_err());

        let oversized_filter = format!("name:{}", "x".repeat(MAX_FILTER_CHARS));
        assert!(SessionFilter::parse(&oversized_filter).is_err());
        assert!(TextPattern::new("x".repeat(MAX_FILTER_VALUE_CHARS + 1)).is_err());
        assert!(RecordingFilter::parse(&format!("contains:{}", "x".repeat(MAX_FILTER_VALUE_CHARS + 1))).is_err());
        assert!(RecordingFilter::parse(&format!("profile:{}", "x".repeat(MAX_FILTER_PRINCIPAL_CHARS + 1))).is_err());
        assert!(RecordingFilter::parse("for:@person\0escape").is_err());
        assert!(SessionFilter::parse(&format!("for:@{}", "x".repeat(MAX_FILTER_PRINCIPAL_CHARS + 1))).is_err());
        assert!(SessionFilter::parse("for:@person\0escape").is_err());
    }
}
