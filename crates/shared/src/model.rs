use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::validation::{bounded, collection_len, identifier, purpose, required};
use crate::{AccessList, ApiError, IdentityId, OrgId, ProfileId, SessionId, Validate, ValidationError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityKind {
    Carbon,
    Silicon,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub id: IdentityId,
    pub name: String,
    pub kind: IdentityKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Additional identifiers verified by the identity authority for this same
    /// principal. They are authorization context, never part of the public wire
    /// representation.
    #[doc(hidden)]
    #[serde(skip)]
    pub verified_aliases: Vec<IdentityId>,
}

impl Identity {
    pub fn matches_principal(&self, principal_id: &str) -> bool {
        same_principal(&self.id, principal_id)
            || self.verified_aliases.iter().any(|alias| same_principal(alias, principal_id))
    }

    pub fn principal_ids(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.id.as_str()).chain(self.verified_aliases.iter().map(String::as_str))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Org {
    pub id: OrgId,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IamInfo {
    pub app_id: String,
}

/// Exchanges an IAM short-lived token (SLT) for browser OAuth-style tokens.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthExchangeRequest {
    #[serde(alias = "token")]
    pub short_lived_token: String,
    /// Optional workspace preference among organizations already authorized in IAM.
    /// When absent, Browser selects the first authorized organization by ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_id: Option<OrgId>,
}

impl fmt::Debug for AuthExchangeRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthExchangeRequest")
            .field("short_lived_token", &"[REDACTED]")
            .field("org_id", &self.org_id)
            .finish()
    }
}

impl Validate for AuthExchangeRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        opaque_auth_token(&self.short_lived_token, "oac_", "short_lived_token")?;
        if let Some(org) = &self.org_id {
            identifier(org, "org_id")?;
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthRefreshRequest {
    pub refresh_token: String,
    /// Refresh tokens are opaque; the selected organization must remain explicit.
    pub org_id: OrgId,
}

impl fmt::Debug for AuthRefreshRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthRefreshRequest")
            .field("refresh_token", &"[REDACTED]")
            .field("org_id", &self.org_id)
            .finish()
    }
}

impl Validate for AuthRefreshRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        opaque_auth_token(&self.refresh_token, "ort_", "refresh_token")?;
        identifier(&self.org_id, "org_id")?;
        Ok(())
    }
}

/// Tokens returned by IAM. Callers must persist these owner-only and atomically.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthSession {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: DateTime<Utc>,
    pub identity: Identity,
    pub org: Org,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<String>,
}

pub type AuthRefreshResponse = AuthSession;

impl fmt::Debug for AuthSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthSession")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .field("identity", &self.identity)
            .field("org", &self.org)
            .field("services", &self.services)
            .finish()
    }
}

impl Validate for AuthSession {
    fn validate(&self) -> Result<(), ValidationError> {
        opaque_auth_token(&self.access_token, "oat_", "access_token")?;
        opaque_auth_token(&self.refresh_token, "ort_", "refresh_token")?;
        identifier(&self.identity.id, "identity.id")?;
        identifier(&self.org.id, "org.id")?;
        for service in &self.services {
            identifier(service, "services")?;
        }
        Ok(())
    }
}

fn opaque_auth_token(value: &str, prefix: &'static str, field: &'static str) -> Result<(), ValidationError> {
    if !value.starts_with(prefix)
        || value.len() == prefix.len()
        || value.len() > 16 * 1024
        || value.chars().any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(ValidationError::Invalid {
            field,
            reason: format!("expected one bounded {prefix} token without whitespace or controls"),
        });
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyLocation {
    pub code: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
}

impl Validate for ProxyLocation {
    fn validate(&self) -> Result<(), ValidationError> {
        required(&self.code, "location.code")?;
        required(&self.name, "location.name")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileStatus {
    Active,
    Retired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub fingerprint: String,
    pub location: ProxyLocation,
    pub access: AccessList,
    pub owner_id: IdentityId,
    pub sessions_run: u64,
    pub status: ProfileStatus,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileCreate {
    pub name: String,
    pub location: String,
    #[serde(default)]
    pub access: AccessList,
}

impl Validate for ProfileCreate {
    fn validate(&self) -> Result<(), ValidationError> {
        bounded(&self.name, "name", 100)?;
        if self.location.len() != 2 || !self.location.bytes().all(|byte| byte.is_ascii_alphabetic()) {
            return Err(ValidationError::Invalid {
                field: "location",
                reason: "expected a two-letter provider location code".into(),
            });
        }
        self.access.validate()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<AccessList>,
}

impl Validate for ProfileUpdate {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.is_none() && self.access.is_none() {
            return Err(ValidationError::Required { field: "name or access" });
        }
        if let Some(name) = &self.name {
            bounded(name, "name", 100)?;
        }
        if let Some(access) = &self.access {
            access.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileEnd {
    pub note: String,
}

impl Validate for ProfileEnd {
    fn validate(&self) -> Result<(), ValidationError> {
        bounded(&self.note, "note", 4_000)
    }
}

/// The only session TTLs accepted by Silicon Browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SessionTtl {
    #[serde(rename = "15m")]
    Minutes15,
    #[serde(rename = "30m")]
    Minutes30,
    #[serde(rename = "45m")]
    Minutes45,
    #[serde(rename = "60m")]
    Minutes60,
    #[serde(rename = "120m")]
    Minutes120,
    #[serde(rename = "240m")]
    Minutes240,
}

impl SessionTtl {
    pub const ALL: [Self; 6] =
        [Self::Minutes15, Self::Minutes30, Self::Minutes45, Self::Minutes60, Self::Minutes120, Self::Minutes240];

    pub const fn minutes(self) -> u16 {
        match self {
            Self::Minutes15 => 15,
            Self::Minutes30 => 30,
            Self::Minutes45 => 45,
            Self::Minutes60 => 60,
            Self::Minutes120 => 120,
            Self::Minutes240 => 240,
        }
    }

    pub const fn seconds(self) -> i64 {
        self.minutes() as i64 * 60
    }
}

impl fmt::Display for SessionTtl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}m", self.minutes())
    }
}

impl FromStr for SessionTtl {
    type Err = ValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "15m" => Ok(Self::Minutes15),
            "30m" => Ok(Self::Minutes30),
            "45m" => Ok(Self::Minutes45),
            "60m" => Ok(Self::Minutes60),
            "120m" => Ok(Self::Minutes120),
            "240m" => Ok(Self::Minutes240),
            _ => Err(ValidationError::Invalid {
                field: "ttl",
                reason: "expected 15m, 30m, 45m, 60m, 120m, or 240m".into(),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Ended,
    Expired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<ProfileId>,
    pub incognito: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<ProxyLocation>,
    pub name: String,
    pub description: String,
    pub status: SessionStatus,
    pub initiator_id: IdentityId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participant_ids: Vec<IdentityId>,
    pub ttl: SessionTtl,
    pub started_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_note: Option<String>,
    pub usage: UsageTotal,
}

impl Session {
    pub fn is_participant(&self, identity_id: &str) -> bool {
        same_principal(&self.initiator_id, identity_id)
            || self.participant_ids.iter().any(|participant| same_principal(participant, identity_id))
    }

    pub fn ttl_left_seconds_at(&self, now: DateTime<Utc>) -> u64 {
        if self.status == SessionStatus::Active {
            self.expires_at.signed_duration_since(now).num_seconds().max(0) as u64
        } else {
            0
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionCreate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<ProfileId>,
    #[serde(default)]
    pub incognito: bool,
    pub name: String,
    pub description: String,
    pub ttl: SessionTtl,
}

impl SessionCreate {
    pub fn with_profile(
        profile_id: impl Into<ProfileId>,
        name: impl Into<String>,
        description: impl Into<String>,
        ttl: SessionTtl,
    ) -> Self {
        Self {
            profile_id: Some(profile_id.into()),
            incognito: false,
            name: name.into(),
            description: description.into(),
            ttl,
        }
    }

    pub fn incognito(name: impl Into<String>, description: impl Into<String>, ttl: SessionTtl) -> Self {
        Self { profile_id: None, incognito: true, name: name.into(), description: description.into(), ttl }
    }
}

impl Validate for SessionCreate {
    fn validate(&self) -> Result<(), ValidationError> {
        match (&self.profile_id, self.incognito) {
            (Some(_), true) => {
                return Err(ValidationError::Conflict { left: "profile_id", right: "incognito" });
            }
            (None, false) => {
                return Err(ValidationError::Required { field: "profile_id or incognito" });
            }
            (Some(profile_id), false) => identifier(profile_id, "profile_id")?,
            (None, true) => {}
        }
        bounded(&self.name, "name", 120)?;
        bounded(&self.description, "description", 2_000)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionEnd {
    pub note: String,
}

impl Validate for SessionEnd {
    fn validate(&self) -> Result<(), ValidationError> {
        bounded(&self.note, "note", 4_000)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionLog {
    pub sequence: u64,
    pub at: DateTime<Utc>,
    pub actor_id: IdentityId,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionLogs {
    pub session_id: SessionId,
    pub date: NaiveDate,
    pub entries: Vec<SessionLog>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveLink {
    pub session_id: SessionId,
    pub url: String,
    pub expires_at: DateTime<Utc>,
}

impl Validate for LiveLink {
    fn validate(&self) -> Result<(), ValidationError> {
        identifier(&self.session_id, "session_id")?;
        validate_http_url(&self.url, "url")
    }
}

/// Redeem the opaque grant carried in a Silicon Browser live-link fragment.
///
/// The grant remains explicit so opening a link never places it in an HTTP URL,
/// browser history, or intermediary access log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveRedeemRequest {
    pub grant: String,
}

impl Validate for LiveRedeemRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        bounded(&self.grant, "grant", 64 * 1024)?;
        if self.grant.chars().any(|character| character.is_whitespace() || character.is_control()) {
            return Err(ValidationError::Invalid {
                field: "grant",
                reason: "expected one opaque value without whitespace or controls".into(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingStatus {
    Pending,
    Available,
    Trashed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recording {
    pub session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<ProfileId>,
    pub incognito: bool,
    pub session_name: String,
    pub session_description: String,
    pub owner_id: IdentityId,
    /// Identities that participated in the source session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participant_ids: Vec<IdentityId>,
    /// Actual path returned by Briefcase; empty until a verified video receipt exists.
    pub briefcase_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub briefcase_link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_log_link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_log_path: Option<String>,
    /// Stable non-secret delivery diagnostic, when an artifact is waiting or failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_error: Option<String>,
    pub duration_seconds: u64,
    pub size_bytes: u64,
    pub status: RecordingStatus,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trashed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purge_at: Option<DateTime<Utc>>,
}

impl Recording {
    /// Legacy logical path helper. It does not create or predict a Briefcase directory;
    /// actual storage paths come only from upload receipts.
    pub fn private_path(owner_id: &str, session_id: &str) -> Result<String, ValidationError> {
        identifier(owner_id, "owner_id")?;
        identifier(session_id, "session_id")?;
        Ok(format!("private/{owner_id}/sb/{session_id}"))
    }

    pub fn is_participant(&self, identity_id: &str) -> bool {
        same_principal(&self.owner_id, identity_id)
            || self.participant_ids.iter().any(|participant| same_principal(participant, identity_id))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    /// ISO currency code. Empty is permitted only for a zero, not-yet-priced value.
    pub currency: String,
    /// Millionths of one currency unit, avoiding floating-point billing errors.
    pub micros: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageCost {
    pub browser: Money,
    pub proxy_in: Money,
    pub proxy_out: Money,
    /// Provider-reported proxy cost without a trustworthy directional split.
    #[serde(default)]
    pub proxy_unclassified: Money,
    pub total: Money,
}

/// Current shared browser-account capacity. This is not an organization's
/// allocation, remaining capacity, or a count of anyone's active sessions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageLimits {
    pub concurrent_browser_limit: u64,
    /// Account-reported rate limit; no interval is implied by this value.
    #[serde(default)]
    pub rate_limit: Option<u64>,
    /// When the account was successfully checked, preserved on cache hits.
    pub checked_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageTotal {
    pub sessions: u64,
    pub browser_seconds: u64,
    pub proxy_bytes_in: u64,
    pub proxy_bytes_out: u64,
    /// Provider-reported proxy traffic without a trustworthy directional split.
    #[serde(default)]
    pub proxy_bytes_unclassified: u64,
    pub cost: UsageCost,
}

impl UsageTotal {
    pub fn browser_minutes(&self) -> f64 {
        self.browser_seconds as f64 / 60.0
    }

    pub fn proxy_gb_in(&self) -> f64 {
        self.proxy_bytes_in as f64 / 1_000_000_000.0
    }

    pub fn proxy_gb_out(&self) -> f64 {
        self.proxy_bytes_out as f64 / 1_000_000_000.0
    }

    pub fn proxy_gb_unclassified(&self) -> f64 {
        self.proxy_bytes_unclassified as f64 / 1_000_000_000.0
    }

    pub fn proxy_gb_total(&self) -> f64 {
        (self.proxy_bytes_in as f64 + self.proxy_bytes_out as f64 + self.proxy_bytes_unclassified as f64)
            / 1_000_000_000.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub session_id: SessionId,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub principal_ids: Vec<IdentityId>,
    pub browser_seconds: u64,
    pub proxy_bytes_in: u64,
    pub proxy_bytes_out: u64,
    /// Provider-reported proxy traffic without a trustworthy directional split.
    #[serde(default)]
    pub proxy_bytes_unclassified: u64,
    pub cost: UsageCost,
}

impl Usage {
    pub fn is_for(&self, identity_id: &str) -> bool {
        self.principal_ids.iter().any(|principal| same_principal(principal, identity_id))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchType {
    #[default]
    Web,
    News,
    Research,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    pub query: String,
    pub purpose: String,
    #[serde(default, rename = "type")]
    pub search_type: SearchType,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recency_minutes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<NaiveDate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<NaiveDate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_year_min: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_year_max: Option<i32>,
    #[serde(default)]
    pub page: u8,
}

impl Validate for SearchRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        bounded(&self.query, "query", 16_384)?;
        purpose(&self.purpose)?;
        collection_len(self.include_domains.len(), "include_domains", 100)?;
        collection_len(self.exclude_domains.len(), "exclude_domains", 100)?;
        if self.page > 10 {
            return Err(ValidationError::OutOfRange { field: "page", min: 0, max: 10 });
        }
        if self.recency_minutes == Some(0) {
            return Err(ValidationError::OutOfRange { field: "recency_minutes", min: 1, max: u64::MAX });
        }
        if self.recency_minutes.is_some() && (self.after.is_some() || self.before.is_some()) {
            return Err(ValidationError::Conflict { left: "recency_minutes", right: "after/before" });
        }
        if self.after.zip(self.before).is_some_and(|(after, before)| after > before) {
            return Err(ValidationError::Invalid { field: "after", reason: "must not be later than before".into() });
        }
        if (self.pub_year_min.is_some() || self.pub_year_max.is_some()) && self.search_type != SearchType::Research {
            return Err(ValidationError::Invalid {
                field: "pub_year_min/pub_year_max",
                reason: "publication years are available only for research search".into(),
            });
        }
        if self.pub_year_min.zip(self.pub_year_max).is_some_and(|(min, max)| min > max) {
            return Err(ValidationError::Invalid {
                field: "pub_year_min",
                reason: "must not exceed pub_year_max".into(),
            });
        }
        for domain in &self.include_domains {
            validate_domain(domain, "include_domains")?;
        }
        for domain in &self.exclude_domains {
            validate_domain(domain, "exclude_domains")?;
        }
        for (field, value) in [("location", self.location.as_deref()), ("language", self.language.as_deref())] {
            if let Some(value) = value {
                bounded(value, field, 256)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    pub rank: u32,
    pub title: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub page: u8,
    #[serde(default)]
    pub queued_ms: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchFormat {
    #[default]
    Markdown,
    Html,
    Json,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FetchRequest {
    pub urls: Vec<String>,
    pub purpose: String,
    #[serde(default)]
    pub format: FetchFormat,
    #[serde(default)]
    pub links: bool,
    #[serde(default)]
    pub image_links: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_selectors: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_selectors: Vec<String>,
}

impl FetchRequest {
    pub const UPSTREAM_BATCH_SIZE: usize = 10;

    /// Stable, allocation-free batches; callers can queue these in input order.
    pub fn batches(&self) -> impl Iterator<Item = &[String]> {
        self.urls.chunks(Self::UPSTREAM_BATCH_SIZE)
    }
}

impl Validate for FetchRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.urls.is_empty() {
            return Err(ValidationError::Required { field: "urls" });
        }
        collection_len(self.urls.len(), "urls", 1_000)?;
        purpose(&self.purpose)?;
        for url in &self.urls {
            validate_http_url(url, "urls")?;
        }
        if let Some(timeout) = self.timeout_ms
            && !(1..=110_000).contains(&timeout)
        {
            return Err(ValidationError::OutOfRange { field: "timeout_ms", min: 1, max: 110_000 });
        }
        collection_len(
            self.include_selectors.len().saturating_add(self.exclude_selectors.len()),
            "include_selectors/exclude_selectors",
            20,
        )?;
        validate_selectors(&self.include_selectors, "include_selectors")?;
        validate_selectors(&self.exclude_selectors, "exclude_selectors")?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchStatus {
    Ok,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchItem {
    pub url: String,
    pub status: FetchStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_links: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
    #[serde(default)]
    pub cached: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchResponse {
    /// One item per requested URL, in caller order.
    pub items: Vec<FetchItem>,
    #[serde(default)]
    pub queued_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    pub session_id: SessionId,
    /// Shell-decoded agent-browser command; implementations must not rewrite it.
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
}

impl Validate for RunRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        identifier(&self.session_id, "session_id")?;
        bounded(&self.command, "command", 1_048_576)?;
        if self.command.contains('\0') {
            return Err(ValidationError::Invalid { field: "command", reason: "must not contain NUL".into() });
        }
        collection_len(self.flags.len(), "flags", 256)?;
        for flag in &self.flags {
            if flag.chars().count() > 16_384 {
                return Err(ValidationError::TooLong { field: "flags", max: 16_384 });
            }
            if flag.contains('\0') {
                return Err(ValidationError::Invalid { field: "flags", reason: "must not contain NUL".into() });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunEvent {
    Queued {
        position: u64,
    },
    Started {
        at: DateTime<Utc>,
    },
    Stdout {
        chunk: String,
    },
    Stderr {
        chunk: String,
    },
    Warning {
        message: String,
    },
    /// Terminal client-side failure. A non-zero `Finished` remains reserved
    /// for an agent-browser process that actually ran and exited non-zero.
    Failed {
        error: ApiError,
    },
    Finished {
        result: RunResult,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunResult {
    pub session_id: SessionId,
    pub exit_code: i32,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stdout: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stderr: String,
}

impl RunResult {
    pub fn succeeded(&self) -> bool {
        self.exit_code == 0
    }
}

pub(crate) fn same_principal(left: &str, right: &str) -> bool {
    left.trim().trim_start_matches('@') == right.trim().trim_start_matches('@')
}

fn validate_http_url(value: &str, field: &'static str) -> Result<(), ValidationError> {
    if value.chars().count() > 16_384 {
        return Err(ValidationError::TooLong { field, max: 16_384 });
    }
    let parsed = Url::parse(value).map_err(|error| ValidationError::Invalid { field, reason: error.to_string() })?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(ValidationError::Invalid { field, reason: "expected an absolute http(s) URL".into() });
    }
    Ok(())
}

fn validate_domain(value: &str, field: &'static str) -> Result<(), ValidationError> {
    bounded(value, field, 253)?;
    if value.chars().any(char::is_whitespace) || value.contains('/') || value.contains("://") {
        return Err(ValidationError::Invalid { field, reason: "expected a hostname, not a URL".into() });
    }
    let candidate = format!("https://{}", value.trim_start_matches("*."));
    let parsed =
        Url::parse(&candidate).map_err(|error| ValidationError::Invalid { field, reason: error.to_string() })?;
    if parsed.host_str().is_none() {
        return Err(ValidationError::Invalid { field, reason: "expected a hostname".into() });
    }
    Ok(())
}

fn validate_selectors(selectors: &[String], field: &'static str) -> Result<(), ValidationError> {
    collection_len(selectors.len(), field, 20)?;
    for selector in selectors {
        bounded(selector, field, 4_096)?;
    }
    Ok(())
}

#[cfg(test)]
mod model_tests {
    use chrono::TimeZone;
    use serde::de::DeserializeOwned;
    use serde_json::{Value, json};

    use super::*;

    fn search() -> SearchRequest {
        SearchRequest {
            query: "browser research".into(),
            purpose: "Find primary sources".into(),
            search_type: SearchType::Web,
            include_domains: Vec::new(),
            exclude_domains: Vec::new(),
            location: None,
            language: None,
            recency_minutes: None,
            after: None,
            before: None,
            pub_year_min: None,
            pub_year_max: None,
            page: 0,
        }
    }

    fn fetch(urls: usize) -> FetchRequest {
        FetchRequest {
            urls: (0..urls).map(|index| format!("https://example.com/{index}")).collect(),
            purpose: "Read the relevant sections".into(),
            format: FetchFormat::Markdown,
            links: false,
            image_links: false,
            ttl_seconds: None,
            timeout_ms: None,
            include_selectors: Vec::new(),
            exclude_selectors: Vec::new(),
        }
    }

    fn rejects_unknown_field<T: DeserializeOwned>(mut value: Value) {
        value.as_object_mut().unwrap().insert("unexpected".into(), Value::Bool(true));
        assert!(serde_json::from_value::<T>(value).is_err());
    }

    /// Test group: every client-supplied JSON object rejects misspelled or future fields explicitly.
    #[test]
    fn request_payloads_reject_unknown_fields() {
        rejects_unknown_field::<AuthExchangeRequest>(json!({"short_lived_token": "slt_value", "org_id": "tos"}));
        rejects_unknown_field::<AuthRefreshRequest>(json!({"refresh_token": "ort_value", "org_id": "tos"}));
        rejects_unknown_field::<ProfileCreate>(json!({"name": "primary", "location": "in", "access": []}));
        rejects_unknown_field::<ProfileUpdate>(json!({"name": "renamed"}));
        rejects_unknown_field::<ProfileEnd>(json!({"note": "done"}));
        rejects_unknown_field::<SessionCreate>(json!({
            "profile_id": "profile-1",
            "name": "research",
            "description": "primary sources",
            "ttl": "30m"
        }));
        rejects_unknown_field::<SessionEnd>(json!({"note": "done"}));
        rejects_unknown_field::<LiveRedeemRequest>(json!({"grant": "opaque-grant"}));
        rejects_unknown_field::<SearchRequest>(json!({"query": "browsers", "purpose": "research"}));
        rejects_unknown_field::<FetchRequest>(json!({"urls": ["https://example.com"], "purpose": "read"}));
        rejects_unknown_field::<RunRequest>(json!({"session_id": "session-1", "command": "snapshot"}));
    }

    /// Test group: a live-link grant is opaque, bounded, and safe to place only in a request body.
    #[test]
    fn live_redeem_grant_is_bounded_and_control_free() {
        assert!(LiveRedeemRequest { grant: "opaque-grant".into() }.validate().is_ok());
        assert!(LiveRedeemRequest { grant: String::new() }.validate().is_err());
        assert!(LiveRedeemRequest { grant: "grant\nheader".into() }.validate().is_err());
        assert!(LiveRedeemRequest { grant: "x".repeat(64 * 1024 + 1) }.validate().is_err());
    }

    /// Test group: TTL accepts and serializes only the six documented values.
    #[test]
    fn ttl_contract_is_exact() {
        let expected = ["15m", "30m", "45m", "60m", "120m", "240m"];
        for (ttl, expected) in SessionTtl::ALL.into_iter().zip(expected) {
            assert_eq!(ttl.to_string(), expected);
            assert_eq!(expected.parse::<SessionTtl>().unwrap(), ttl);
            assert_eq!(serde_json::to_string(&ttl).unwrap(), format!(r#""{expected}""#));
        }
        assert!("90m".parse::<SessionTtl>().is_err());
    }

    /// Test group: a new session is exactly one of profile-backed or incognito.
    #[test]
    fn session_mode_is_exclusive() {
        assert!(
            SessionCreate::with_profile("profile-1", "name", "description", SessionTtl::Minutes30).validate().is_ok()
        );
        assert!(SessionCreate::incognito("name", "description", SessionTtl::Minutes15).validate().is_ok());

        let neither = SessionCreate {
            profile_id: None,
            incognito: false,
            name: "name".into(),
            description: "description".into(),
            ttl: SessionTtl::Minutes15,
        };
        assert!(neither.validate().is_err());

        let both = SessionCreate { profile_id: Some("profile-1".into()), incognito: true, ..neither };
        assert!(both.validate().is_err());
    }

    /// Test group: countdown never becomes negative or advertises time after a session ends.
    #[test]
    fn ttl_left_is_clamped_at_zero() {
        let expires = Utc.with_ymd_and_hms(2026, 8, 1, 12, 0, 0).unwrap();
        let mut session = Session {
            id: "session-1".into(),
            profile_id: None,
            incognito: true,
            location: None,
            name: "research".into(),
            description: "test".into(),
            status: SessionStatus::Expired,
            initiator_id: "silicon-1".into(),
            participant_ids: Vec::new(),
            ttl: SessionTtl::Minutes15,
            started_at: expires - chrono::Duration::minutes(15),
            expires_at: expires,
            ended_at: Some(expires),
            end_note: Some("ttl reached".into()),
            usage: UsageTotal::default(),
        };
        assert_eq!(session.ttl_left_seconds_at(expires + chrono::Duration::seconds(3)), 0);
        let before_expiry = expires - chrono::Duration::minutes(5);
        session.status = SessionStatus::Active;
        assert_eq!(session.ttl_left_seconds_at(before_expiry), 300);
        assert_eq!(session.ttl_left_seconds_at(expires + chrono::Duration::seconds(3)), 0);
        session.status = SessionStatus::Ended;
        session.ended_at = Some(before_expiry);
        assert_eq!(session.ttl_left_seconds_at(before_expiry), 0);
    }

    /// Test group: profile updates cannot be empty and immutable fields are absent.
    #[test]
    fn profile_update_requires_a_mutable_field() {
        assert!(ProfileUpdate::default().validate().is_err());
        assert!(ProfileUpdate { name: Some("new name".into()), access: None }.validate().is_ok());
    }

    /// Test group: IAM secrets never appear in Debug output and token families validate.
    #[test]
    fn auth_tokens_are_redacted_and_typed() {
        let exchange = AuthExchangeRequest { short_lived_token: "oac_single_use".into(), org_id: Some("tos".into()) };
        assert!(exchange.validate().is_ok());
        assert_eq!(
            serde_json::to_value(&exchange).unwrap(),
            json!({"short_lived_token": "oac_single_use", "org_id": "tos"})
        );
        assert!(!format!("{exchange:?}").contains("oac_single_use"));
        assert!(
            AuthExchangeRequest { short_lived_token: "oac_single_use".into(), org_id: Some(String::new()) }
                .validate()
                .is_err()
        );
        assert!(
            AuthExchangeRequest { short_lived_token: "oat_wrong_family".into(), org_id: Some("tos".into()) }
                .validate()
                .is_err()
        );
        assert!(
            serde_json::from_value::<AuthExchangeRequest>(json!({
                "short_lived_token": "oac_single_use"
            }))
            .is_ok()
        );

        let request = AuthRefreshRequest { refresh_token: "ort_secret".into(), org_id: "tos".into() };
        assert!(request.validate().is_ok());
        assert!(!format!("{request:?}").contains("ort_secret"));

        let bad = AuthRefreshRequest { refresh_token: "oat_wrong-family".into(), org_id: "tos".into() };
        assert!(bad.validate().is_err());

        let mut session = AuthSession {
            access_token: "oat_access".into(),
            refresh_token: "ort_refresh".into(),
            expires_at: Utc.timestamp_opt(1_800_000_000, 0).unwrap(),
            identity: Identity {
                id: "silicon-1".into(),
                name: "Silicon".into(),
                kind: IdentityKind::Silicon,
                tags: vec![],
                verified_aliases: Vec::new(),
            },
            org: Org { id: "tos".into(), name: "TOS".into() },
            services: vec!["session".into()],
        };
        assert!(session.validate().is_ok());
        session.access_token = "oat_bad\nheader".into();
        assert!(session.validate().is_err());
    }

    /// Test group: search validates page, date mode, research-only years, and purpose length.
    #[test]
    fn search_constraints_match_the_cli_contract() {
        let mut request = search();
        request.page = 11;
        assert!(request.validate().is_err());

        let mut request = search();
        request.recency_minutes = Some(60);
        request.after = NaiveDate::from_ymd_opt(2026, 8, 1);
        assert!(request.validate().is_err());

        let mut request = search();
        request.pub_year_min = Some(2020);
        assert!(request.validate().is_err());
        request.search_type = SearchType::Research;
        assert!(request.validate().is_ok());

        let mut request = search();
        request.purpose = "x".repeat(2_001);
        assert!(request.validate().is_err());
    }

    /// Test group: search text and domain collections have generous denial-of-service bounds.
    #[test]
    fn search_text_and_domain_bounds_are_enforced() {
        let mut request = search();
        request.query = "q".repeat(16_385);
        assert_eq!(request.validate(), Err(ValidationError::TooLong { field: "query", max: 16_384 }));

        let mut request = search();
        request.include_domains = vec!["example.com".into(); 101];
        assert_eq!(request.validate(), Err(ValidationError::TooMany { field: "include_domains", max: 100 }));

        let mut request = search();
        request.exclude_domains = vec!["x".repeat(254)];
        assert_eq!(request.validate(), Err(ValidationError::TooLong { field: "exclude_domains", max: 253 }));
    }

    /// Test group: fetch uses at most ten upstream URLs while preserving order.
    #[test]
    fn fetch_batches_boundaries_without_reordering() {
        let request = fetch(21);
        assert!(request.validate().is_ok());
        let sizes: Vec<_> = request.batches().map(<[String]>::len).collect();
        assert_eq!(sizes, [10, 10, 1]);
        assert_eq!(request.batches().nth(1).unwrap()[0], "https://example.com/10");
    }

    /// Test group: fetch validates URL schemes, timeouts, selectors, and ttl=0.
    #[test]
    fn fetch_validation_accepts_live_cache_bypass_but_rejects_unsafe_inputs() {
        let mut request = fetch(1);
        request.ttl_seconds = Some(0);
        request.timeout_ms = Some(110_000);
        assert!(request.validate().is_ok());

        request.timeout_ms = Some(110_001);
        assert!(request.validate().is_err());

        let mut request = fetch(1);
        request.urls[0] = "file:///etc/passwd".into();
        assert!(request.validate().is_err());

        let mut request = fetch(1);
        request.include_selectors = vec!["main".into(); 21];
        assert!(request.validate().is_err());

        let mut request = fetch(1);
        request.include_selectors = vec!["main".into(); 10];
        request.exclude_selectors = vec!["nav".into(); 11];
        assert_eq!(
            request.validate(),
            Err(ValidationError::TooMany { field: "include_selectors/exclude_selectors", max: 20 })
        );
    }

    /// Test group: fetch batches remain finite and individual URLs/selectors cannot dominate a request.
    #[test]
    fn fetch_collection_and_string_bounds_are_enforced() {
        let request = fetch(1_001);
        assert_eq!(request.validate(), Err(ValidationError::TooMany { field: "urls", max: 1_000 }));

        let mut request = fetch(1);
        request.urls[0] = format!("https://example.com/{}", "x".repeat(16_365));
        assert_eq!(request.validate(), Err(ValidationError::TooLong { field: "urls", max: 16_384 }));

        let mut request = fetch(1);
        request.exclude_selectors = vec!["x".repeat(4_097)];
        assert_eq!(request.validate(), Err(ValidationError::TooLong { field: "exclude_selectors", max: 4_096 }));
    }

    /// Test group: user-authored labels and terminal notes are bounded without trimming their content.
    #[test]
    fn profile_and_session_text_bounds_are_enforced() {
        let profile = ProfileCreate { name: "x".repeat(101), location: "in".into(), access: AccessList::default() };
        assert_eq!(profile.validate(), Err(ValidationError::TooLong { field: "name", max: 100 }));

        let profile = ProfileCreate { name: "name".into(), location: "1n".into(), access: AccessList::default() };
        assert!(profile.validate().is_err());

        let session = SessionCreate::incognito("research", "x".repeat(2_001), SessionTtl::Minutes15);
        assert_eq!(session.validate(), Err(ValidationError::TooLong { field: "description", max: 2_000 }));

        let end = SessionEnd { note: "x".repeat(4_001) };
        assert_eq!(end.validate(), Err(ValidationError::TooLong { field: "note", max: 4_000 }));
    }

    /// Test group: Briefcase paths cannot escape the initiating principal's private root.
    #[test]
    fn recording_path_is_safe_and_deterministic() {
        assert_eq!(Recording::private_path("silicon-1", "session-1").unwrap(), "private/silicon-1/sb/session-1");
        assert!(Recording::private_path("../other", "session-1").is_err());
    }

    /// Test group: recording discovery metadata is explicit on the wire.
    #[test]
    fn recording_wire_exposes_source_session_metadata() {
        let recording: Recording = serde_json::from_value(json!({
            "session_id": "session-1",
            "profile_id": "profile-1",
            "incognito": false,
            "session_name": "Market scan",
            "session_description": "Research browser vendors",
            "owner_id": "silicon-1",
            "participant_ids": ["silicon-1", "carbon-1"],
            "briefcase_path": "private/silicon-1/sb/session-1/recording.mp4",
            "briefcase_link": null,
            "duration_seconds": 60,
            "size_bytes": 100,
            "status": "pending",
            "created_at": "2026-08-10T12:00:00Z"
        }))
        .unwrap();
        assert_eq!(recording.profile_id.as_deref(), Some("profile-1"));
        assert!(!recording.incognito);
        assert_eq!(recording.participant_ids, ["silicon-1", "carbon-1"]);

        let value = serde_json::to_value(recording).unwrap();
        assert_eq!(value["profile_id"], "profile-1");
        assert_eq!(value["incognito"], false);
        assert_eq!(value["participant_ids"], json!(["silicon-1", "carbon-1"]));
    }

    /// Test group: usage conversions expose minutes and decimal GB without changing stored integers.
    #[test]
    fn usage_units_are_derived_from_exact_counters() {
        let usage = UsageTotal {
            browser_seconds: 90,
            proxy_bytes_in: 1_500_000_000,
            proxy_bytes_out: 2_000_000_000,
            ..UsageTotal::default()
        };
        assert_eq!(usage.browser_minutes(), 1.5);
        assert_eq!(usage.proxy_gb_in(), 1.5);
        assert_eq!(usage.proxy_gb_out(), 2.0);
    }

    /// Test group: run commands retain significant whitespace and pass-through flags.
    #[test]
    fn run_request_validation_does_not_rewrite_commands() {
        let request = RunRequest {
            session_id: "session-1".into(),
            command: "evaluate 'a  b'  ".into(),
            flags: vec!["--json".into()],
        };
        let before = request.clone();
        request.validate().unwrap();
        assert_eq!(request, before);
    }

    /// Test group: pass-through command arguments are preserved but bounded and NUL-free.
    #[test]
    fn run_request_bounds_command_and_flags() {
        let mut request = RunRequest {
            session_id: "session-1".into(),
            command: "snapshot".into(),
            flags: vec!["--json".into(); 257],
        };
        assert_eq!(request.validate(), Err(ValidationError::TooMany { field: "flags", max: 256 }));

        request.flags = vec!["x".repeat(16_385)];
        assert_eq!(request.validate(), Err(ValidationError::TooLong { field: "flags", max: 16_384 }));

        request.flags = vec!["--header\0secret".into()];
        assert!(matches!(request.validate(), Err(ValidationError::Invalid { field: "flags", .. })));

        request.flags.clear();
        request.command = "x".repeat(1_048_577);
        assert_eq!(request.validate(), Err(ValidationError::TooLong { field: "command", max: 1_048_576 }));
    }
}

/// A sensitive, direct provider connection. The client controls the browser;
/// this is not a backend proxy and grants are bounded by the browser's lifetime.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionConnection {
    pub session_id: SessionId,
    /// Immutable requesting caller identity for local controller isolation.
    pub principal_id: String,
    pub cdp_url: String,
    pub expires_at: DateTime<Utc>,
}
impl std::fmt::Debug for SessionConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionConnection")
            .field("session_id", &self.session_id)
            .field("cdp_url", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Cooperative client telemetry, not proof that these were all browser actions.
/// Retrying this report must never execute the command again. Browser output
/// remains on the client and has no field in this telemetry protocol.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandReport {
    pub command_id: uuid::Uuid,
    pub command: String,
    #[serde(default)]
    pub flags: Vec<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub exit_code: i32,
    #[serde(default)]
    pub truncated: bool,
}
impl std::fmt::Debug for CommandReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandReport").field("command_id", &self.command_id).field("content", &"[REDACTED]").finish()
    }
}
impl Validate for CommandReport {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.command_id.is_nil() || self.finished_at < self.started_at {
            return Err(ValidationError::Invalid {
                field: "command_report",
                reason: "requires a nonzero ID and ordered timestamps".into(),
            });
        }
        RunRequest { session_id: "report".into(), command: self.command.clone(), flags: self.flags.clone() }
            .validate()?;
        if self.command.len() + self.flags.iter().map(String::len).sum::<usize>() > 1_048_576 {
            return Err(ValidationError::TooLong { field: "command", max: 1_048_576 });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandReportReceipt {
    pub command_id: uuid::Uuid,
    /// Carbon sessions intentionally do not retain command history.
    pub sequence: Option<u64>,
}
