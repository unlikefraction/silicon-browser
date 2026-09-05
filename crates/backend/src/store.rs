//! SQLite-backed domain state and lifecycle transitions.
//!
//! Provider calls happen outside this module. The store reserves scarce state
//! first and every subsequent transition is compare-and-swap, so retries and
//! concurrent workers cannot create two live sessions for one profile.

use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, Utc};
use silicon_browser_shared::{
    AccessList, Identity, IdentityKind, LiveLink, Money, Profile, ProfileCreate, ProfileEnd, ProfileStatus,
    ProfileUpdate, ProxyLocation, Recording, RecordingFilter, RecordingStatus, Session, SessionCreate, SessionEnd,
    SessionLog, SessionStatus, SessionTtl, Usage, UsageCost, UsageFilter, UsageTotal, Validate,
};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow};
use sqlx::{Row, SqlitePool};
use thiserror::Error;
use uuid::Uuid;

use crate::crypto::SecretBox;
use crate::decimal::decimal_to_millionths;

mod command_reports;
mod delivery;
#[cfg(test)]
mod delivery_tests;
pub use delivery::{RecordingArtifactKind, RecordingDeliveryClaim};

pub type StoreResult<T> = Result<T, StoreError>;

/// Give the provider a short post-stop materialization window before the
/// first recording lookup. Later retries are scheduled by the maintenance
/// worker with exponential backoff.
pub const RECORDING_SOURCE_INITIAL_DELAY: Duration = Duration::from_secs(15);

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("database migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("command report rejected: {code}")]
    ReportConflict { code: &'static str },
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("{kind} {id} was not found")]
    NotFound { kind: &'static str, id: String },
    #[error("not allowed to access {kind} {id}")]
    Forbidden { kind: &'static str, id: String },
    #[error("profile {profile_id} is retired")]
    ProfileRetired { profile_id: String },
    #[error("profile {profile_id} is in use by {actor_id} until {expires_at}")]
    ProfileBusy { profile_id: String, session_id: String, actor_id: String, expires_at: DateTime<Utc> },
    #[error("session {session_id} is {status}")]
    SessionState { session_id: String, status: String },
    #[error("stored {kind} data is invalid: {reason}")]
    Corrupt { kind: &'static str, reason: String },
    #[error("secret operation failed: {0}")]
    Crypto(String),
}

#[derive(Clone, PartialEq, Eq)]
pub struct ProviderSession {
    pub id: String,
    pub cdp_url: String,
    pub live_url: String,
    pub recording_url: Option<String>,
}

impl std::fmt::Debug for ProviderSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderSession")
            .field("id", &self.id)
            .field("cdp_url", &"[REDACTED]")
            .field("live_url", &"[REDACTED]")
            .field("recording_url", &self.recording_url.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

/// Decrypted only at the provider boundary; Debug output cannot disclose signed URLs.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderRuntime {
    pub provider_session_id: String,
    pub cdp_url: String,
    pub live_url: String,
    pub recording_url: Option<String>,
}

/// Minimal, non-secret correlation needed to reconcile a profile whose
/// provider create may have committed before local activation completed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvisioningProfile {
    pub org_id: String,
    pub profile_id: String,
}

/// A provider browser claimed by the TTL reaper. The database session remains
/// in `ending` until the provider confirms it stopped, so its profile cannot be
/// reused while a remote browser may still be alive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpiringSession {
    pub org_id: String,
    pub session_id: String,
    pub incognito: bool,
    pub started_at: DateTime<Utc>,
    pub lease_id: String,
    pub runtime: ProviderRuntime,
}

/// A leased durable request to discover a recording URL which the browser
/// provider had not materialized in its initial terminal response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingSourceClaim {
    pub event_id: String,
    pub org_id: String,
    pub session_id: String,
    pub provider_session_id: String,
    /// One-based number of the provider lookup represented by this lease.
    pub attempt: u32,
}

/// Values observed while compensating a browser start which could not be
/// activated locally, bundled so the terminal transition remains explicit.
#[derive(Clone, Debug)]
pub struct FailedSessionFinalization<'a> {
    pub provider_session_id: &'a str,
    pub reason: &'a str,
    pub duration_ms: u64,
    pub has_recording_source: bool,
    pub at: DateTime<Utc>,
}

impl std::fmt::Debug for ProviderRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderRuntime")
            .field("provider_session_id", &self.provider_session_id)
            .field("cdp_url", &"[REDACTED]")
            .field("live_url", &"[REDACTED]")
            .field("recording_url", &self.recording_url.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticipantRole {
    Initiator,
    Runner,
    Viewer,
}

impl ParticipantRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Initiator => "initiator",
            Self::Runner => "runner",
            Self::Viewer => "viewer",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageSample {
    pub browser_millis: u64,
    pub proxy_bytes_in: Option<u64>,
    pub proxy_bytes_out: Option<u64>,
    /// Aggregate provider telemetry for which no ingress/egress split exists.
    pub proxy_bytes_unclassified: Option<u64>,
    /// Decimal currency units. Provider precision beyond millionths is rounded
    /// half-up when exposed through the public integer-micros model.
    pub browser_cost: String,
    /// Combined inbound and outbound proxy cost from the provider.
    pub proxy_cost: String,
    pub currency: String,
    pub sampled_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
    #[cfg(test)]
    usage_write_test_hook: Option<UsageWriteTestHook>,
}

#[cfg(test)]
#[derive(Clone)]
struct UsageWriteTestHook {
    after_read: std::sync::Arc<tokio::sync::Notify>,
    resume: std::sync::Arc<tokio::sync::Notify>,
}

impl Store {
    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn connect(database_url: &str) -> StoreResult<Self> {
        let options = SqliteConnectOptions::from_str(database_url)?
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(10));
        let max_connections = if database_url.contains(":memory:") { 1 } else { 8 };
        let pool = SqlitePoolOptions::new().max_connections(max_connections).connect_with(options).await?;
        let store = Self {
            pool,
            #[cfg(test)]
            usage_write_test_hook: None,
        };
        store.migrate().await?;
        Ok(store)
    }

    pub async fn in_memory() -> StoreResult<Self> {
        Self::connect("sqlite::memory:").await
    }

    pub async fn migrate(&self) -> StoreResult<()> {
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }

    pub async fn remember_identity_projection(
        &self,
        org_id: &str,
        principal_id: &str,
        public_id: &str,
        kind: IdentityKind,
        secrets: &SecretBox,
        now: DateTime<Utc>,
    ) -> StoreResult<Identity> {
        safe_id(org_id, "org_id")?;
        safe_id(principal_id, "principal_id")?;
        safe_id(public_id, "public_id")?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let existing =
            sqlx::query("SELECT public_id, kind FROM identity_projection WHERE org_id = ? AND principal_id = ?")
                .bind(org_id)
                .bind(principal_id)
                .fetch_optional(&mut *transaction)
                .await?;
        let previous_public_id = if let Some(existing) = existing.as_ref() {
            let stored_public_id: String = existing.try_get("public_id")?;
            let stored_kind: String = existing.try_get("kind")?;
            if stored_kind != identity_kind(kind) {
                return Err(corrupt("identity projection", "IAM identity kind changed for a principal"));
            }
            Some(stored_public_id)
        } else {
            None
        };
        sqlx::query(
            "INSERT INTO identity_projection (org_id, principal_id, public_id, kind, updated_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(org_id, principal_id) DO UPDATE SET \
             public_id = excluded.public_id, kind = excluded.kind, updated_at = excluded.updated_at",
        )
        .bind(org_id)
        .bind(principal_id)
        .bind(public_id)
        .bind(identity_kind(kind))
        .bind(timestamp(now))
        .execute(&mut *transaction)
        .await?;

        // Canonicalize both the OAT-only UUID and a previous public id. This
        // keeps ownership stable across first exchange and a later verified
        // IAM public-id rotation.
        let mut legacy_ids = Vec::new();
        if principal_id != public_id {
            legacy_ids.push(principal_id.to_owned());
        }
        if let Some(previous) = previous_public_id
            && previous != public_id
            && previous != principal_id
        {
            legacy_ids.push(previous);
        }
        for legacy_id in legacy_ids {
            let rows = sqlx::query("SELECT id, access_json FROM profiles WHERE org_id = ?")
                .bind(org_id)
                .fetch_all(&mut *transaction)
                .await?;
            for row in rows {
                let profile_id: String = row.try_get("id")?;
                let access: AccessList = serde_json::from_str(row.try_get("access_json")?).map_err(corrupt_json)?;
                let rewritten = AccessList::new(access.iter().map(|entry| {
                    if same_principal(entry, &legacy_id) { format!("@{public_id}") } else { entry.to_owned() }
                }))
                .map_err(invalid)?;
                sqlx::query(
                    "UPDATE profiles SET owner_id = CASE WHEN owner_id = ? THEN ? ELSE owner_id END, \
                     access_json = ? WHERE org_id = ? AND id = ?",
                )
                .bind(&legacy_id)
                .bind(public_id)
                .bind(serde_json::to_string(&rewritten).map_err(corrupt_json)?)
                .bind(org_id)
                .bind(profile_id)
                .execute(&mut *transaction)
                .await?;
            }
            sqlx::query("UPDATE sessions SET started_by = ? WHERE org_id = ? AND started_by = ?")
                .bind(public_id)
                .bind(org_id)
                .bind(&legacy_id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query(
                "INSERT OR IGNORE INTO session_participants (session_id, actor_id, role, first_seen_at) \
                 SELECT sp.session_id, ?, sp.role, sp.first_seen_at FROM session_participants sp \
                 JOIN sessions s ON s.id = sp.session_id \
                 WHERE s.org_id = ? AND sp.actor_id = ?",
            )
            .bind(public_id)
            .bind(org_id)
            .bind(&legacy_id)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "DELETE FROM session_participants WHERE actor_id = ? AND session_id IN \
                 (SELECT id FROM sessions WHERE org_id = ?)",
            )
            .bind(&legacy_id)
            .bind(org_id)
            .execute(&mut *transaction)
            .await?;
            let command_rows = sqlx::query(
                "SELECT c.session_id, c.sequence, c.command_enc FROM commands c \
                 JOIN sessions s ON s.id = c.session_id WHERE s.org_id = ? AND c.actor_id = ?",
            )
            .bind(org_id)
            .bind(&legacy_id)
            .fetch_all(&mut *transaction)
            .await?;
            for row in command_rows {
                let command_session_id: String = row.try_get("session_id")?;
                let sequence: i64 = row.try_get("sequence")?;
                let encrypted: String = row.try_get("command_enc")?;
                let command = secrets
                    .open_for(&command_secret_context(org_id, &command_session_id, sequence, &legacy_id), &encrypted)
                    .map_err(StoreError::Crypto)?;
                let encrypted = secrets
                    .seal_for(&command_secret_context(org_id, &command_session_id, sequence, public_id), &command)
                    .map_err(StoreError::Crypto)?;
                sqlx::query("UPDATE commands SET actor_id = ?, command_enc = ? WHERE session_id = ? AND sequence = ?")
                    .bind(public_id)
                    .bind(encrypted)
                    .bind(command_session_id)
                    .bind(sequence)
                    .execute(&mut *transaction)
                    .await?;
            }
            sqlx::query(
                "UPDATE recordings SET owner_id = ? WHERE owner_id = ? AND session_id IN \
                 (SELECT id FROM sessions WHERE org_id = ?)",
            )
            .bind(public_id)
            .bind(&legacy_id)
            .bind(org_id)
            .execute(&mut *transaction)
            .await?;
            // Briefcase paths are provider receipts, not identity-derived local paths.
            // A public-ID projection must never invent a remote rename.
            sqlx::query("UPDATE discovery_log SET actor_id = ? WHERE org_id = ? AND actor_id = ?")
                .bind(public_id)
                .bind(org_id)
                .bind(&legacy_id)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(projected_identity_value(public_id, principal_id, kind))
    }

    pub async fn projected_identity(
        &self,
        org_id: &str,
        principal_id: &str,
        kind: IdentityKind,
    ) -> StoreResult<Option<Identity>> {
        safe_id(org_id, "org_id")?;
        safe_id(principal_id, "principal_id")?;
        let row = sqlx::query("SELECT public_id, kind FROM identity_projection WHERE org_id = ? AND principal_id = ?")
            .bind(org_id)
            .bind(principal_id)
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else { return Ok(None) };
        let stored_kind: String = row.try_get("kind")?;
        if stored_kind != identity_kind(kind) {
            return Err(corrupt("identity projection", "IAM identity kind changed for a principal"));
        }
        let public_id: String = row.try_get("public_id")?;
        Ok(Some(projected_identity_value(&public_id, principal_id, kind)))
    }

    /// Reserve a stable local profile id before provisioning the upstream
    /// profile. The temporary provider values are never returned by an HTTP
    /// handler; callers must either activate or fail this reservation.
    pub async fn reserve_profile(
        &self,
        org_id: &str,
        owner: &Identity,
        request: &ProfileCreate,
        now: DateTime<Utc>,
    ) -> StoreResult<Profile> {
        request.validate().map_err(invalid)?;
        safe_id(org_id, "org_id")?;
        safe_id(&owner.id, "owner_id")?;
        if request.name.chars().count() > 100 {
            return Err(StoreError::Invalid("profile name must contain at most 100 characters".into()));
        }
        if request.location.chars().count() != 2 || !request.location.bytes().all(|byte| byte.is_ascii_alphabetic()) {
            return Err(StoreError::Invalid("profile location must be a two-letter location code".into()));
        }

        let id = Uuid::now_v7().to_string();
        let pending = format!("pending-{id}");
        let access = request.access.normalized_with_owner(&owner.id).map_err(invalid)?;
        sqlx::query(
            "INSERT INTO profiles \
             (id, org_id, name, provider_profile_id, fingerprint, location, access_json, owner_id, status, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'provisioning', ?)",
        )
        .bind(&id)
        .bind(org_id)
        .bind(request.name.trim())
        .bind(&pending)
        .bind(&pending)
        .bind(request.location.trim().to_ascii_lowercase())
        .bind(serde_json::to_string(&access).map_err(corrupt_json)?)
        .bind(&owner.id)
        .bind(timestamp(now))
        .execute(&self.pool)
        .await
        .map_err(map_profile_write)?;
        self.profile_unchecked(org_id, &id).await
    }

    /// Complete exactly one profile reservation with the upstream identity.
    pub async fn activate_profile(
        &self,
        org_id: &str,
        profile_id: &str,
        provider_profile_id: &str,
        fingerprint: &str,
    ) -> StoreResult<Profile> {
        safe_id(provider_profile_id, "provider_profile_id")?;
        required(fingerprint, "fingerprint")?;
        let result = sqlx::query(
            "UPDATE profiles SET provider_profile_id = ?, fingerprint = ?, status = 'active' \
             WHERE org_id = ? AND id = ? AND status = 'provisioning'",
        )
        .bind(provider_profile_id)
        .bind(fingerprint)
        .bind(org_id)
        .bind(profile_id)
        .execute(&self.pool)
        .await
        .map_err(map_profile_write)?;
        if result.rows_affected() == 0 {
            let row = self.profile_row(org_id, profile_id).await?;
            return Err(StoreError::SessionState { session_id: profile_id.into(), status: row.status });
        }
        self.profile_unchecked(org_id, profile_id).await
    }

    /// Return a finite snapshot of every profile awaiting reconciliation.
    /// Provider profile creation uses the local profile id as Browser Use's
    /// stable `userId`, so no provider credential or mutable request data
    /// needs to be persisted here. Do not cap the oldest rows: an unresolved
    /// prefix must not permanently starve profiles created later.
    pub async fn provisioning_profiles(&self) -> StoreResult<Vec<ProvisioningProfile>> {
        let rows = sqlx::query(
            "SELECT org_id, id FROM profiles WHERE status = 'provisioning' \
             ORDER BY created_at, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| Ok(ProvisioningProfile { org_id: row.try_get("org_id")?, profile_id: row.try_get("id")? }))
            .collect()
    }

    /// Make a failed reservation permanently unusable without deleting its
    /// audit identity.
    pub async fn fail_profile(
        &self,
        org_id: &str,
        profile_id: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> StoreResult<Profile> {
        required(reason, "reason")?;
        let result = sqlx::query(
            "UPDATE profiles SET status = 'failed', ended_at = ?, end_note = ? \
             WHERE org_id = ? AND id = ? AND status = 'provisioning'",
        )
        .bind(timestamp(now))
        .bind(reason.trim())
        .bind(org_id)
        .bind(profile_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            let row = self.profile_row(org_id, profile_id).await?;
            return Err(StoreError::SessionState { session_id: profile_id.into(), status: row.status });
        }
        self.profile_unchecked(org_id, profile_id).await
    }

    pub async fn create_profile(
        &self,
        org_id: &str,
        owner: &Identity,
        request: &ProfileCreate,
        provider_profile_id: &str,
        fingerprint: &str,
        now: DateTime<Utc>,
    ) -> StoreResult<Profile> {
        request.validate().map_err(invalid)?;
        safe_id(org_id, "org_id")?;
        safe_id(&owner.id, "owner_id")?;
        safe_id(provider_profile_id, "provider_profile_id")?;
        required(fingerprint, "fingerprint")?;
        if request.name.chars().count() > 100 {
            return Err(StoreError::Invalid("profile name must contain at most 100 characters".into()));
        }
        if request.location.chars().count() != 2 {
            return Err(StoreError::Invalid("profile location must be a two-letter location code".into()));
        }
        let id = Uuid::now_v7().to_string();
        let access = request.access.normalized_with_owner(&owner.id).map_err(invalid)?;
        let access_json = serde_json::to_string(&access).map_err(corrupt_json)?;
        sqlx::query(
            "INSERT INTO profiles \
             (id, org_id, name, provider_profile_id, fingerprint, location, access_json, owner_id, status, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'active', ?)",
        )
        .bind(&id)
        .bind(org_id)
        .bind(request.name.trim())
        .bind(provider_profile_id)
        .bind(fingerprint)
        .bind(request.location.trim().to_ascii_lowercase())
        .bind(access_json)
        .bind(&owner.id)
        .bind(timestamp(now))
        .execute(&self.pool)
        .await
        .map_err(map_profile_write)?;
        self.profile_unchecked(org_id, &id).await
    }

    pub async fn profiles(&self, org_id: &str, viewer: &Identity) -> StoreResult<Vec<Profile>> {
        let rows = sqlx::query(
            "SELECT * FROM profiles WHERE org_id = ? AND status <> 'provisioning' ORDER BY created_at DESC",
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;
        let mut profiles = Vec::new();
        for row in rows {
            let db = DbProfile::from_row(&row)?;
            if db.access()?.allows(viewer) {
                profiles.push(self.map_profile(db).await?);
            }
        }
        Ok(profiles)
    }

    pub async fn profile(&self, org_id: &str, viewer: &Identity, profile_id: &str) -> StoreResult<Profile> {
        let db = self.profile_row(org_id, profile_id).await?;
        if db.status == "provisioning" || !db.access()?.allows(viewer) {
            return Err(StoreError::NotFound { kind: "profile", id: profile_id.to_owned() });
        }
        self.map_profile(db).await
    }

    /// Resolve the opaque upstream id only after applying the same visibility
    /// rules as a normal profile read.
    pub async fn provider_profile_id(&self, org_id: &str, viewer: &Identity, profile_id: &str) -> StoreResult<String> {
        let db = self.profile_row(org_id, profile_id).await?;
        if !db.access()?.allows(viewer) {
            return Err(StoreError::NotFound { kind: "profile", id: profile_id.to_owned() });
        }
        if db.status != "active" {
            return Err(StoreError::ProfileRetired { profile_id: profile_id.to_owned() });
        }
        Ok(db.provider_profile_id)
    }

    pub async fn update_profile(
        &self,
        org_id: &str,
        actor: &Identity,
        profile_id: &str,
        request: &ProfileUpdate,
    ) -> StoreResult<Profile> {
        request.validate().map_err(invalid)?;
        if request.name.as_ref().is_some_and(|name| name.chars().count() > 100) {
            return Err(StoreError::Invalid("profile name must contain at most 100 characters".into()));
        }
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let row = sqlx::query("SELECT owner_id, name, access_json, status FROM profiles WHERE org_id = ? AND id = ?")
            .bind(org_id)
            .bind(profile_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| not_found("profile", profile_id))?;
        let owner_id: String = row.try_get("owner_id")?;
        if !actor.matches_principal(&owner_id) {
            return Err(forbidden("profile", profile_id));
        }
        let status: String = row.try_get("status")?;
        if status == "ended" {
            return Err(StoreError::ProfileRetired { profile_id: profile_id.to_owned() });
        }
        if !matches!(status.as_str(), "active" | "provisioning") {
            return Err(StoreError::SessionState { session_id: profile_id.to_owned(), status });
        }
        let name = request.name.as_deref().unwrap_or(row.try_get::<&str, _>("name")?).trim().to_owned();
        let access = match &request.access {
            Some(access) => access.normalized_with_owner(&owner_id).map_err(invalid)?,
            None => serde_json::from_str::<AccessList>(row.try_get("access_json")?).map_err(corrupt_json)?,
        };
        sqlx::query("UPDATE profiles SET name = ?, access_json = ? WHERE org_id = ? AND id = ?")
            .bind(name)
            .bind(serde_json::to_string(&access).map_err(corrupt_json)?)
            .bind(org_id)
            .bind(profile_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        self.profile_unchecked(org_id, profile_id).await
    }

    pub async fn end_profile(
        &self,
        org_id: &str,
        actor: &Identity,
        profile_id: &str,
        request: &ProfileEnd,
        now: DateTime<Utc>,
    ) -> StoreResult<Profile> {
        request.validate().map_err(invalid)?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let row = sqlx::query("SELECT owner_id, status FROM profiles WHERE org_id = ? AND id = ?")
            .bind(org_id)
            .bind(profile_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| not_found("profile", profile_id))?;
        let owner_id: String = row.try_get("owner_id")?;
        if !actor.matches_principal(&owner_id) {
            return Err(forbidden("profile", profile_id));
        }
        let live = sqlx::query(
            "SELECT id, started_by, expires_at FROM sessions \
             WHERE profile_id = ? AND status IN ('starting', 'active', 'ending') LIMIT 1",
        )
        .bind(profile_id)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(live) = live {
            return Err(StoreError::ProfileBusy {
                profile_id: profile_id.to_owned(),
                session_id: live.try_get("id")?,
                actor_id: live.try_get("started_by")?,
                expires_at: parse_timestamp(live.try_get("expires_at")?, "session.expires_at")?,
            });
        }
        sqlx::query(
            "UPDATE profiles SET status = 'ended', ended_at = ?, end_note = ? \
             WHERE org_id = ? AND id = ? AND status <> 'ended'",
        )
        .bind(timestamp(now))
        .bind(request.note.trim())
        .bind(org_id)
        .bind(profile_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.profile_unchecked(org_id, profile_id).await
    }

    async fn profile_unchecked(&self, org_id: &str, profile_id: &str) -> StoreResult<Profile> {
        let row = self.profile_row(org_id, profile_id).await?;
        self.map_profile(row).await
    }

    async fn profile_row(&self, org_id: &str, profile_id: &str) -> StoreResult<DbProfile> {
        let row = sqlx::query("SELECT * FROM profiles WHERE org_id = ? AND id = ?")
            .bind(org_id)
            .bind(profile_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| not_found("profile", profile_id))?;
        DbProfile::from_row(&row)
    }

    async fn map_profile(&self, row: DbProfile) -> StoreResult<Profile> {
        let sessions_run: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions WHERE profile_id = ?")
            .bind(&row.id)
            .fetch_one(&self.pool)
            .await?;
        let access = row.access()?;
        Ok(Profile {
            id: row.id,
            name: row.name,
            fingerprint: row.fingerprint,
            location: proxy_location(row.location),
            access,
            owner_id: row.owner_id,
            sessions_run: nonnegative(sessions_run, "profile.sessions_run")?,
            status: match row.status.as_str() {
                "active" | "provisioning" => ProfileStatus::Active,
                "ended" | "failed" => ProfileStatus::Retired,
                value => return Err(corrupt("profile", format!("unknown status {value}"))),
            },
            created_at: parse_timestamp(&row.created_at, "profile.created_at")?,
            ended_at: optional_timestamp(row.ended_at.as_deref(), "profile.ended_at")?,
            end_note: row.end_note,
        })
    }
}

struct DbProfile {
    id: String,
    provider_profile_id: String,
    name: String,
    fingerprint: String,
    location: String,
    access_json: String,
    owner_id: String,
    status: String,
    created_at: String,
    ended_at: Option<String>,
    end_note: Option<String>,
}

impl DbProfile {
    fn from_row(row: &SqliteRow) -> StoreResult<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            provider_profile_id: row.try_get("provider_profile_id")?,
            name: row.try_get("name")?,
            fingerprint: row.try_get("fingerprint")?,
            location: row.try_get("location")?,
            access_json: row.try_get("access_json")?,
            owner_id: row.try_get("owner_id")?,
            status: row.try_get("status")?,
            created_at: row.try_get("created_at")?,
            ended_at: row.try_get("ended_at")?,
            end_note: row.try_get("end_note")?,
        })
    }

    fn access(&self) -> StoreResult<AccessList> {
        serde_json::from_str(&self.access_json).map_err(corrupt_json)
    }
}

impl Store {
    /// Atomically reserves the one live slot for a profile before a provider is called.
    pub async fn reserve_session(
        &self,
        org_id: &str,
        initiator: &Identity,
        request: &SessionCreate,
        now: DateTime<Utc>,
    ) -> StoreResult<Session> {
        request.validate().map_err(invalid)?;
        safe_id(org_id, "org_id")?;
        safe_id(&initiator.id, "initiator_id")?;
        if request.name.chars().count() > 120 {
            return Err(StoreError::Invalid("session name must contain at most 120 characters".into()));
        }
        if request.description.chars().count() > 2_000 {
            return Err(StoreError::Invalid("session description must contain at most 2000 characters".into()));
        }

        let id = Uuid::now_v7().to_string();
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        if let Some(profile_id) = &request.profile_id {
            let row = sqlx::query("SELECT status, access_json FROM profiles WHERE org_id = ? AND id = ?")
                .bind(org_id)
                .bind(profile_id)
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or_else(|| not_found("profile", profile_id))?;
            let status: String = row.try_get("status")?;
            if matches!(status.as_str(), "ended" | "failed") {
                return Err(StoreError::ProfileRetired { profile_id: profile_id.clone() });
            }
            if status != "active" {
                return Err(StoreError::Invalid(format!("profile is not ready ({status})")));
            }
            let access: AccessList = serde_json::from_str(row.try_get("access_json")?).map_err(corrupt_json)?;
            if !access.allows(initiator) {
                return Err(not_found("profile", profile_id));
            }
        }

        let expires_at = now + ChronoDuration::seconds(request.ttl.seconds());
        let insert = sqlx::query(
            "INSERT INTO sessions \
             (id, org_id, profile_id, started_by, started_by_kind, name, description, status, started_at, expires_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 'starting', ?, ?)",
        )
        .bind(&id)
        .bind(org_id)
        .bind(&request.profile_id)
        .bind(&initiator.id)
        .bind(identity_kind(initiator.kind))
        .bind(request.name.trim())
        .bind(request.description.trim())
        .bind(timestamp(now))
        .bind(timestamp(expires_at))
        .execute(&mut *transaction)
        .await;
        if let Err(error) = insert {
            transaction.rollback().await?;
            if is_unique_violation(&error)
                && let Some(profile_id) = &request.profile_id
                && let Some(busy) = self.profile_busy(org_id, profile_id).await?
            {
                return Err(busy);
            }
            return Err(StoreError::Database(error));
        }

        sqlx::query(
            "INSERT INTO session_participants (session_id, actor_id, role, first_seen_at) VALUES (?, ?, 'initiator', ?)",
        )
        .bind(&id)
        .bind(&initiator.id)
        .bind(timestamp(now))
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO recordings (session_id, owner_id, status, artifact_path) VALUES (?, ?, 'recording', ?)",
        )
        .bind(&id)
        .bind(&initiator.id)
        .bind("")
        .execute(&mut *transaction)
        .await?;
        let proxy_counter: Option<i64> = request.profile_id.as_ref().map(|_| 0);
        sqlx::query(
            "INSERT INTO usage \
             (session_id, browser_millis, proxy_bytes_in, proxy_bytes_out, proxy_bytes_unclassified, browser_cost, proxy_cost, currency, sampled_at) \
             VALUES (?, 0, ?, ?, ?, '0', '0', 'USD', ?)",
        )
        .bind(&id)
        .bind(proxy_counter)
        .bind(proxy_counter)
        .bind(proxy_counter)
        .bind(timestamp(now))
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.session_unchecked(org_id, &id).await
    }

    pub async fn sessions(&self, org_id: &str, viewer: &Identity) -> StoreResult<Vec<Session>> {
        Ok(self.visible_sessions_with_usage(org_id, viewer).await?.into_iter().map(|(session, _)| session).collect())
    }

    /// Two snapshot-consistent reads replace per-session identity, participant,
    /// and billing lookups when a dashboard lists a growing session history.
    async fn visible_sessions_with_usage(&self, org_id: &str, viewer: &Identity) -> StoreResult<Vec<(Session, Usage)>> {
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT s.*, p.location AS profile_location, p.access_json, u.* FROM sessions s \
             LEFT JOIN profiles p ON p.id = s.profile_id AND p.org_id = s.org_id \
             JOIN usage u ON u.session_id = s.id WHERE s.org_id = ? ORDER BY s.started_at DESC, s.id",
        )
        .bind(org_id)
        .fetch_all(&mut *tx)
        .await?;
        let participants = sqlx::query(
            "SELECT sp.session_id, sp.actor_id, MIN(sp.first_seen_at) AS first_seen FROM session_participants sp \
             JOIN sessions s ON s.id = sp.session_id WHERE s.org_id = ? \
             GROUP BY sp.session_id, sp.actor_id ORDER BY first_seen, sp.actor_id",
        )
        .bind(org_id)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        let mut actors: HashMap<String, Vec<String>> = HashMap::new();
        for participant in participants {
            actors.entry(participant.try_get("session_id")?).or_default().push(participant.try_get("actor_id")?);
        }
        let mut visible = Vec::new();
        for row in rows {
            let session = DbSession::from_row(&row)?;
            let participant_ids = actors.remove(&session.id).unwrap_or_default();
            let participant = viewer.principal_ids().any(|id| participant_ids.iter().any(|actor| actor == id));
            let allowed = participant
                || row
                    .try_get::<Option<&str>, _>("access_json")?
                    .map(|json| serde_json::from_str::<AccessList>(json).map_err(corrupt_json))
                    .transpose()?
                    .is_some_and(|access| access.allows(viewer));
            if allowed {
                let usage = map_usage(&row, participant_ids.clone())?;
                visible.push((map_session_model(session, participant_ids, &usage)?, usage));
            }
        }
        Ok(visible)
    }

    pub async fn session(&self, org_id: &str, viewer: &Identity, session_id: &str) -> StoreResult<Session> {
        let row = self.session_row(org_id, session_id).await?;
        if !self.session_visible(&row, viewer).await? {
            return Err(not_found("session", session_id));
        }
        self.map_session(row).await
    }

    pub async fn activate_session(
        &self,
        org_id: &str,
        session_id: &str,
        provider: &ProviderSession,
        secrets: &SecretBox,
        now: DateTime<Utc>,
    ) -> StoreResult<Session> {
        safe_id(&provider.id, "provider_session_id")?;
        required(&provider.cdp_url, "provider.cdp_url")?;
        required(&provider.live_url, "provider.live_url")?;
        let cdp = secrets
            .seal_for(&session_secret_context(org_id, session_id, "provider-cdp-url"), &provider.cdp_url)
            .map_err(StoreError::Crypto)?;
        let live = secrets
            .seal_for(&session_secret_context(org_id, session_id, "provider-live-url"), &provider.live_url)
            .map_err(StoreError::Crypto)?;
        let recording = provider
            .recording_url
            .as_deref()
            .map(|url| {
                secrets
                    .seal_for(&session_secret_context(org_id, session_id, "provider-recording-url"), url)
                    .map_err(StoreError::Crypto)
            })
            .transpose()?;
        let result = sqlx::query(
            "UPDATE sessions SET status = 'active', provider_session_id = ?, provider_cdp_url_enc = ?, \
             provider_live_url_enc = ?, provider_recording_url_enc = ? \
             WHERE org_id = ? AND id = ? AND status = 'starting' AND expires_at > ?",
        )
        .bind(&provider.id)
        .bind(cdp)
        .bind(live)
        .bind(recording)
        .bind(org_id)
        .bind(session_id)
        .bind(timestamp(now))
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            let row = self.session_row(org_id, session_id).await?;
            if parse_timestamp(&row.expires_at, "session.expires_at")? <= now && row.is_live() {
                // Persist the just-created remote runtime before returning the
                // expiry error. The caller can now stop it, and a crash leaves
                // enough encrypted state for the TTL reaper to reconcile it.
                sqlx::query(
                    "UPDATE sessions SET status = 'ending', end_note = 'ttl reached', provider_session_id = ?, \
                     provider_cdp_url_enc = ?, provider_live_url_enc = ?, provider_recording_url_enc = ? \
                     WHERE org_id = ? AND id = ? AND status = 'starting'",
                )
                .bind(&provider.id)
                .bind(
                    secrets
                        .seal_for(&session_secret_context(org_id, session_id, "provider-cdp-url"), &provider.cdp_url)
                        .map_err(StoreError::Crypto)?,
                )
                .bind(
                    secrets
                        .seal_for(&session_secret_context(org_id, session_id, "provider-live-url"), &provider.live_url)
                        .map_err(StoreError::Crypto)?,
                )
                .bind(
                    provider
                        .recording_url
                        .as_deref()
                        .map(|url| {
                            secrets
                                .seal_for(&session_secret_context(org_id, session_id, "provider-recording-url"), url)
                                .map_err(StoreError::Crypto)
                        })
                        .transpose()?,
                )
                .bind(org_id)
                .bind(session_id)
                .execute(&self.pool)
                .await?;
                return Err(StoreError::SessionState { session_id: session_id.into(), status: "ending".into() });
            }
            return Err(StoreError::SessionState { session_id: session_id.into(), status: row.status });
        }
        self.session_unchecked(org_id, session_id).await
    }

    /// Durably retain a provider browser after local activation failed. This
    /// transition deliberately keeps the profile uniqueness guard held while
    /// the caller confirms the upstream browser has stopped. It is idempotent
    /// for the same provider runtime and refuses to overwrite another one.
    pub async fn retain_session_runtime(
        &self,
        org_id: &str,
        session_id: &str,
        provider: &ProviderSession,
        reason: &str,
        secrets: &SecretBox,
    ) -> StoreResult<()> {
        safe_id(org_id, "org_id")?;
        safe_id(session_id, "session_id")?;
        safe_id(&provider.id, "provider_session_id")?;
        required(&provider.cdp_url, "provider.cdp_url")?;
        required(&provider.live_url, "provider.live_url")?;
        required(reason, "reason")?;
        let cdp = secrets
            .seal_for(&session_secret_context(org_id, session_id, "provider-cdp-url"), &provider.cdp_url)
            .map_err(StoreError::Crypto)?;
        let live = secrets
            .seal_for(&session_secret_context(org_id, session_id, "provider-live-url"), &provider.live_url)
            .map_err(StoreError::Crypto)?;
        let recording = provider
            .recording_url
            .as_deref()
            .map(|url| {
                secrets
                    .seal_for(&session_secret_context(org_id, session_id, "provider-recording-url"), url)
                    .map_err(StoreError::Crypto)
            })
            .transpose()?;
        let result = sqlx::query(
            "UPDATE sessions SET status = 'ending', end_note = COALESCE(end_note, ?), \
             provider_session_id = COALESCE(provider_session_id, ?), \
             provider_cdp_url_enc = COALESCE(provider_cdp_url_enc, ?), \
             provider_live_url_enc = COALESCE(provider_live_url_enc, ?), \
             provider_recording_url_enc = COALESCE(provider_recording_url_enc, ?) \
             WHERE org_id = ? AND id = ? AND status IN ('starting', 'active', 'ending') \
             AND (provider_session_id IS NULL OR provider_session_id = ?)",
        )
        .bind(reason.trim())
        .bind(&provider.id)
        .bind(cdp)
        .bind(live)
        .bind(recording)
        .bind(org_id)
        .bind(session_id)
        .bind(&provider.id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            let row = sqlx::query("SELECT status, provider_session_id FROM sessions WHERE org_id = ? AND id = ?")
                .bind(org_id)
                .bind(session_id)
                .fetch_optional(&self.pool)
                .await?
                .ok_or_else(|| not_found("session", session_id))?;
            let status: String = row.try_get("status")?;
            let existing: Option<String> = row.try_get("provider_session_id")?;
            if existing.as_deref().is_some_and(|existing| existing != provider.id) {
                return Err(corrupt("session", "provider session identity changed during activation recovery"));
            }
            return Err(StoreError::SessionState { session_id: session_id.into(), status });
        }
        Ok(())
    }

    pub async fn fail_session(
        &self,
        org_id: &str,
        session_id: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> StoreResult<Session> {
        required(reason, "reason")?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let result = sqlx::query(
            "UPDATE sessions SET status = 'failed', ended_at = ?, end_note = ? \
             WHERE org_id = ? AND id = ? AND status IN ('starting', 'active', 'ending')",
        )
        .bind(timestamp(now))
        .bind(reason.trim())
        .bind(org_id)
        .bind(session_id)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() != 0 {
            sqlx::query("UPDATE recordings SET status = 'failed' WHERE session_id = ? AND status = 'recording'")
                .bind(session_id)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        self.session_unchecked(org_id, session_id).await
    }

    /// Atomically release a failed-start profile slot after the caller has
    /// authoritatively confirmed the provider browser stopped. If its recording
    /// URL has not materialized yet, retain a durable resolution intent rather
    /// than losing the visual recording at this boundary.
    pub async fn finalize_failed_session_after_stop(
        &self,
        org_id: &str,
        session_id: &str,
        finalization: FailedSessionFinalization<'_>,
    ) -> StoreResult<Session> {
        let FailedSessionFinalization { provider_session_id, reason, duration_ms, has_recording_source, at: now } =
            finalization;
        safe_id(provider_session_id, "provider_session_id")?;
        required(reason, "reason")?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let result = sqlx::query(
            "UPDATE sessions SET status = 'failed', ended_at = ?, end_note = ?, \
             provider_session_id = COALESCE(provider_session_id, ?) \
             WHERE org_id = ? AND id = ? AND status IN ('starting', 'active', 'ending') \
             AND (provider_session_id IS NULL OR provider_session_id = ?)",
        )
        .bind(timestamp(now))
        .bind(reason.trim())
        .bind(provider_session_id)
        .bind(org_id)
        .bind(session_id)
        .bind(provider_session_id)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            let row = sqlx::query("SELECT status, provider_session_id FROM sessions WHERE org_id = ? AND id = ?")
                .bind(org_id)
                .bind(session_id)
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or_else(|| not_found("session", session_id))?;
            let status: String = row.try_get("status")?;
            let stored_provider_id: Option<String> = row.try_get("provider_session_id")?;
            if stored_provider_id.as_deref().is_some_and(|stored| stored != provider_session_id) {
                return Err(corrupt("session", "provider session identity changed during stop finalization"));
            }
            match status.as_str() {
                "failed" => {}
                status => {
                    return Err(StoreError::SessionState { session_id: session_id.into(), status: status.into() });
                }
            }
        }
        if has_recording_source {
            sqlx::query(
                "UPDATE recordings SET status = 'pending', duration_ms = ?, size_bytes = COALESCE(size_bytes, 0) \
                 WHERE session_id = ? AND status IN ('recording', 'pending')",
            )
            .bind(to_i64(duration_ms, "recording.duration_ms")?)
            .bind(session_id)
            .execute(&mut *transaction)
            .await?;
            let payload = serde_json::to_string(&serde_json::json!({
                "org_id": org_id,
                "session_id": session_id,
            }))
            .map_err(corrupt_json)?;
            sqlx::query(
                "INSERT OR IGNORE INTO outbox \
                 (id, event_type, payload_json, attempts, next_attempt_at, created_at) \
                 VALUES (?, 'recording.store', ?, 0, ?, ?)",
            )
            .bind(format!("recording.store:{session_id}"))
            .bind(payload)
            .bind(timestamp(now))
            .bind(timestamp(now))
            .execute(&mut *transaction)
            .await?;
        } else {
            sqlx::query(
                "UPDATE recordings SET status = 'pending', duration_ms = ?, size_bytes = COALESCE(size_bytes, 0) \
                 WHERE session_id = ? AND status IN ('recording', 'pending')",
            )
            .bind(to_i64(duration_ms, "recording.duration_ms")?)
            .bind(session_id)
            .execute(&mut *transaction)
            .await?;
            let payload = serde_json::to_string(&serde_json::json!({
                "org_id": org_id,
                "session_id": session_id,
            }))
            .map_err(corrupt_json)?;
            let first_attempt = now
                + ChronoDuration::from_std(RECORDING_SOURCE_INITIAL_DELAY)
                    .map_err(|_| StoreError::Invalid("recording retry delay is too large".into()))?;
            sqlx::query(
                "INSERT OR IGNORE INTO outbox \
                 (id, event_type, payload_json, attempts, next_attempt_at, created_at) \
                 VALUES (?, 'recording.resolve', ?, 0, ?, ?)",
            )
            .bind(format!("recording.resolve:{session_id}"))
            .bind(payload)
            .bind(timestamp(first_attempt))
            .bind(timestamp(now))
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        self.session_unchecked(org_id, session_id).await
    }

    #[cfg(test)]
    async fn end_session(
        &self,
        org_id: &str,
        session_id: &str,
        request: &SessionEnd,
        now: DateTime<Utc>,
    ) -> StoreResult<Session> {
        request.validate().map_err(invalid)?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let result = sqlx::query(
            "UPDATE sessions SET status = 'ended', ended_at = ?, end_note = ? \
             WHERE org_id = ? AND id = ? AND status IN ('starting', 'active', 'ending')",
        )
        .bind(timestamp(now))
        .bind(request.note.trim())
        .bind(org_id)
        .bind(session_id)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() != 0 {
            sqlx::query("UPDATE recordings SET status = 'pending' WHERE session_id = ? AND status = 'recording'")
                .bind(session_id)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        self.session_unchecked(org_id, session_id).await
    }

    pub async fn associate_participant(
        &self,
        org_id: &str,
        session_id: &str,
        actor_id: &str,
        role: ParticipantRole,
        now: DateTime<Utc>,
    ) -> StoreResult<bool> {
        safe_id(actor_id, "actor_id")?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let row = sqlx::query("SELECT status, expires_at FROM sessions WHERE org_id = ? AND id = ?")
            .bind(org_id)
            .bind(session_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| not_found("session", session_id))?;
        let status: String = row.try_get("status")?;
        let expires_at = parse_timestamp(row.try_get("expires_at")?, "session.expires_at")?;
        if status != "active" || expires_at <= now {
            return Err(StoreError::SessionState {
                session_id: session_id.into(),
                status: if expires_at <= now { "expired".into() } else { status },
            });
        }
        let result = sqlx::query(
            "INSERT OR IGNORE INTO session_participants (session_id, actor_id, role, first_seen_at) VALUES (?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(actor_id)
        .bind(role.as_str())
        .bind(timestamp(now))
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn live_link(
        &self,
        org_id: &str,
        viewer: &Identity,
        session_id: &str,
        secrets: &SecretBox,
    ) -> StoreResult<LiveLink> {
        let session = self.session(org_id, viewer, session_id).await?;
        if session.status != SessionStatus::Active {
            return Err(StoreError::SessionState {
                session_id: session_id.into(),
                status: format!("{:?}", session.status),
            });
        }
        let encrypted: Option<String> =
            sqlx::query_scalar("SELECT provider_live_url_enc FROM sessions WHERE org_id = ? AND id = ?")
                .bind(org_id)
                .bind(session_id)
                .fetch_optional(&self.pool)
                .await?
                .flatten();
        let encrypted = encrypted.ok_or_else(|| corrupt("session", "active session has no live URL"))?;
        Ok(LiveLink {
            session_id: session_id.into(),
            url: secrets
                .open_for(&session_secret_context(org_id, session_id, "provider-live-url"), &encrypted)
                .map_err(StoreError::Crypto)?,
            expires_at: session.expires_at,
        })
    }

    pub async fn provider_runtime(
        &self,
        org_id: &str,
        viewer: &Identity,
        session_id: &str,
        secrets: &SecretBox,
    ) -> StoreResult<ProviderRuntime> {
        self.session(org_id, viewer, session_id).await?;
        let row = sqlx::query(
            "SELECT status, provider_session_id, provider_cdp_url_enc, provider_live_url_enc, \
             provider_recording_url_enc FROM sessions WHERE org_id = ? AND id = ?",
        )
        .bind(org_id)
        .bind(session_id)
        .fetch_one(&self.pool)
        .await?;
        let status: String = row.try_get("status")?;
        if status != "active" {
            return Err(StoreError::SessionState { session_id: session_id.into(), status });
        }
        let decrypt_required =
            |label: &'static str, context_field: &'static str, encrypted: Option<String>| -> StoreResult<String> {
                let encrypted =
                    encrypted.ok_or_else(|| corrupt("session", format!("active session has no {label}")))?;
                secrets
                    .open_for(&session_secret_context(org_id, session_id, context_field), &encrypted)
                    .map_err(StoreError::Crypto)
            };
        let recording = row
            .try_get::<Option<String>, _>("provider_recording_url_enc")?
            .map(|encrypted| {
                secrets
                    .open_for(&session_secret_context(org_id, session_id, "provider-recording-url"), &encrypted)
                    .map_err(StoreError::Crypto)
            })
            .transpose()?;
        Ok(ProviderRuntime {
            provider_session_id: row
                .try_get::<Option<String>, _>("provider_session_id")?
                .ok_or_else(|| corrupt("session", "active session has no provider session ID"))?,
            cdp_url: decrypt_required("CDP URL", "provider-cdp-url", row.try_get("provider_cdp_url_enc")?)?,
            live_url: decrypt_required("live URL", "provider-live-url", row.try_get("provider_live_url_enc")?)?,
            recording_url: recording,
        })
    }

    /// Claim an active session for an explicit end operation. Retrying an
    /// already-claimed `ending` session returns the same runtime, allowing the
    /// caller to retry an uncertain provider stop without releasing the local
    /// profile slot.
    pub async fn begin_end_session(
        &self,
        org_id: &str,
        actor: &Identity,
        session_id: &str,
        request: &SessionEnd,
        secrets: &SecretBox,
    ) -> StoreResult<ProviderRuntime> {
        request.validate().map_err(invalid)?;
        self.session(org_id, actor, session_id).await?;
        let result = sqlx::query(
            "UPDATE sessions SET status = 'ending', end_note = ? \
             WHERE org_id = ? AND id = ? AND status = 'active'",
        )
        .bind(request.note.trim())
        .bind(org_id)
        .bind(session_id)
        .execute(&self.pool)
        .await?;

        let row = sqlx::query(
            "SELECT status, provider_session_id, provider_cdp_url_enc, provider_live_url_enc, \
             provider_recording_url_enc FROM sessions WHERE org_id = ? AND id = ?",
        )
        .bind(org_id)
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| not_found("session", session_id))?;
        let status: String = row.try_get("status")?;
        if status != "ending" {
            return Err(StoreError::SessionState { session_id: session_id.into(), status });
        }
        // A zero-row update is expected for a retry of an already-ending
        // session. In that case preserve the note recorded by the first claim.
        let _ = result;
        runtime_from_row(&row, org_id, session_id, secrets)
    }

    /// Finalize a provider-confirmed stop. Only `ending` can become `ended`;
    /// this compare-and-swap is what releases the profile uniqueness guard.
    pub async fn finalize_end_session(
        &self,
        org_id: &str,
        session_id: &str,
        now: DateTime<Utc>,
    ) -> StoreResult<Session> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let result = sqlx::query(
            "UPDATE sessions SET status = 'ended', ended_at = ? \
             WHERE org_id = ? AND id = ? AND status = 'ending'",
        )
        .bind(timestamp(now))
        .bind(org_id)
        .bind(session_id)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            let status: Option<String> = sqlx::query_scalar("SELECT status FROM sessions WHERE org_id = ? AND id = ?")
                .bind(org_id)
                .bind(session_id)
                .fetch_optional(&mut *transaction)
                .await?;
            match status.as_deref() {
                Some("ended") => {}
                Some(status) => {
                    return Err(StoreError::SessionState { session_id: session_id.into(), status: status.into() });
                }
                None => return Err(not_found("session", session_id)),
            }
        }
        sqlx::query("UPDATE recordings SET status = 'pending' WHERE session_id = ? AND status = 'recording'")
            .bind(session_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        self.session_unchecked(org_id, session_id).await
    }

    /// Atomically claim all provider-backed sessions whose TTL elapsed.
    /// Provider-less reservations are safe to expire immediately; live remote
    /// browsers remain `ending` until `finalize_expired_session` is called.
    pub async fn claim_expired_sessions(
        &self,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
        limit: u32,
        secrets: &SecretBox,
    ) -> StoreResult<Vec<ExpiringSession>> {
        if limit == 0 || limit > 100 {
            return Err(StoreError::Invalid("expired-session claim limit must be between 1 and 100".into()));
        }
        if lease_until <= now {
            return Err(StoreError::Invalid("expired-session lease must end in the future".into()));
        }
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let starting: Vec<String> = sqlx::query_scalar(
            "UPDATE sessions SET status = 'expired', ended_at = ?, end_note = 'ttl reached' \
             WHERE status = 'starting' AND expires_at <= ? RETURNING id",
        )
        .bind(timestamp(now))
        .bind(timestamp(now))
        .fetch_all(&mut *transaction)
        .await?;
        for id in starting {
            // A starting row has no validated provider runtime/source. It may
            // represent a pre-provider crash or an ambiguous create, so do not
            // advertise an uploadable recording that Briefcase can never fetch.
            sqlx::query("UPDATE recordings SET status = 'failed' WHERE session_id = ? AND status = 'recording'")
                .bind(id)
                .execute(&mut *transaction)
                .await?;
        }
        sqlx::query(
            "UPDATE sessions SET status = 'ending', end_note = COALESCE(end_note, 'ttl reached') \
             WHERE status = 'active' AND expires_at <= ?",
        )
        .bind(timestamp(now))
        .execute(&mut *transaction)
        .await?;
        let lease_id = Uuid::now_v7().to_string();
        let rows = sqlx::query(
            "UPDATE sessions SET stop_lease_id = ?, stop_lease_until = ? WHERE id IN ( \
               SELECT id FROM sessions WHERE status = 'ending' AND expires_at <= ? \
               AND provider_session_id IS NOT NULL AND (stop_lease_until IS NULL OR stop_lease_until <= ?) \
               ORDER BY expires_at, id LIMIT ? \
             ) RETURNING org_id, id, profile_id, started_at, status, provider_session_id, \
             provider_cdp_url_enc, provider_live_url_enc, provider_recording_url_enc",
        )
        .bind(&lease_id)
        .bind(timestamp(lease_until))
        .bind(timestamp(now))
        .bind(timestamp(now))
        .bind(i64::from(limit))
        .fetch_all(&mut *transaction)
        .await?;
        transaction.commit().await?;

        rows.into_iter()
            .map(|row| {
                let org_id: String = row.try_get("org_id")?;
                let session_id: String = row.try_get("id")?;
                Ok(ExpiringSession {
                    org_id: org_id.clone(),
                    session_id: session_id.clone(),
                    incognito: row.try_get::<Option<String>, _>("profile_id")?.is_none(),
                    started_at: parse_timestamp(row.try_get("started_at")?, "session.started_at")?,
                    lease_id: lease_id.clone(),
                    runtime: runtime_from_row(&row, &org_id, &session_id, secrets)?,
                })
            })
            .collect()
    }

    pub async fn finalize_expired_session(
        &self,
        org_id: &str,
        session_id: &str,
        lease_id: &str,
        now: DateTime<Utc>,
    ) -> StoreResult<()> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let result = sqlx::query(
            "UPDATE sessions SET status = 'expired', ended_at = ?, end_note = 'ttl reached' \
             WHERE org_id = ? AND id = ? AND status = 'ending' AND expires_at <= ? AND stop_lease_id = ?",
        )
        .bind(timestamp(now))
        .bind(org_id)
        .bind(session_id)
        .bind(timestamp(now))
        .bind(lease_id)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            let status: Option<String> = sqlx::query_scalar("SELECT status FROM sessions WHERE org_id = ? AND id = ?")
                .bind(org_id)
                .bind(session_id)
                .fetch_optional(&mut *transaction)
                .await?;
            match status.as_deref() {
                Some("expired") => {}
                Some(status) => {
                    return Err(StoreError::SessionState { session_id: session_id.into(), status: status.into() });
                }
                None => return Err(not_found("session", session_id)),
            }
        }
        sqlx::query("UPDATE recordings SET status = 'pending' WHERE session_id = ? AND status = 'recording'")
            .bind(session_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Confirm that a TTL stop claim still owns an `ending` session. A
    /// session may have been manually finalized while the reaper was waiting
    /// for its in-process gate, or another worker may have acquired the lease
    /// after it expired; neither case may issue another provider stop.
    pub async fn expired_claim_is_current(
        &self,
        org_id: &str,
        session_id: &str,
        lease_id: &str,
        now: DateTime<Utc>,
    ) -> StoreResult<bool> {
        let current: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sessions \
             WHERE org_id = ? AND id = ? AND status = 'ending' \
             AND stop_lease_id = ? AND stop_lease_until > ?)",
        )
        .bind(org_id)
        .bind(session_id)
        .bind(lease_id)
        .bind(timestamp(now))
        .fetch_one(&self.pool)
        .await?;
        Ok(current)
    }

    pub async fn release_expired_claim(&self, org_id: &str, session_id: &str, lease_id: &str) -> StoreResult<()> {
        sqlx::query(
            "UPDATE sessions SET stop_lease_id = NULL, stop_lease_until = NULL \
             WHERE org_id = ? AND id = ? AND stop_lease_id = ?",
        )
        .bind(org_id)
        .bind(session_id)
        .bind(lease_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn profile_busy(&self, org_id: &str, profile_id: &str) -> StoreResult<Option<StoreError>> {
        let row = sqlx::query(
            "SELECT id, started_by, expires_at FROM sessions \
             WHERE org_id = ? AND profile_id = ? AND status IN ('starting', 'active', 'ending') LIMIT 1",
        )
        .bind(org_id)
        .bind(profile_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(StoreError::ProfileBusy {
                profile_id: profile_id.to_owned(),
                session_id: row.try_get("id")?,
                actor_id: row.try_get("started_by")?,
                expires_at: parse_timestamp(row.try_get("expires_at")?, "session.expires_at")?,
            })
        })
        .transpose()
    }

    async fn session_unchecked(&self, org_id: &str, session_id: &str) -> StoreResult<Session> {
        let row = self.session_row(org_id, session_id).await?;
        self.map_session(row).await
    }

    async fn session_row(&self, org_id: &str, session_id: &str) -> StoreResult<DbSession> {
        let row = sqlx::query(
            "SELECT s.*, p.location AS profile_location FROM sessions s \
             LEFT JOIN profiles p ON p.id = s.profile_id WHERE s.org_id = ? AND s.id = ?",
        )
        .bind(org_id)
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| not_found("session", session_id))?;
        DbSession::from_row(&row)
    }

    async fn session_visible(&self, row: &DbSession, viewer: &Identity) -> StoreResult<bool> {
        for principal_id in viewer.principal_ids() {
            let participant: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM session_participants WHERE session_id = ? AND actor_id = ?)",
            )
            .bind(&row.id)
            .bind(principal_id)
            .fetch_one(&self.pool)
            .await?;
            if participant {
                return Ok(true);
            }
        }
        let Some(profile_id) = &row.profile_id else {
            return Ok(false);
        };
        let access_json: Option<String> = sqlx::query_scalar("SELECT access_json FROM profiles WHERE id = ?")
            .bind(profile_id)
            .fetch_optional(&self.pool)
            .await?;
        access_json
            .map(|json| {
                serde_json::from_str::<AccessList>(&json).map_err(corrupt_json).map(|access| access.allows(viewer))
            })
            .transpose()
            .map(|allowed| allowed.unwrap_or(false))
    }

    async fn map_session(&self, row: DbSession) -> StoreResult<Session> {
        let participant_ids: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT actor_id FROM session_participants WHERE session_id = ? ORDER BY first_seen_at, actor_id",
        )
        .bind(&row.id)
        .fetch_all(&self.pool)
        .await?;
        let usage = self.usage_unchecked(&row.id).await?;
        map_session_model(row, participant_ids, &usage)
    }
}

fn map_session_model(row: DbSession, participant_ids: Vec<String>, usage: &Usage) -> StoreResult<Session> {
    let status = match row.status.as_str() {
        "starting" | "active" | "ending" => SessionStatus::Active,
        "ended" | "failed" => SessionStatus::Ended,
        "expired" => SessionStatus::Expired,
        value => return Err(corrupt("session", format!("unknown status {value}"))),
    };
    Ok(Session {
        id: row.id,
        incognito: row.profile_id.is_none(),
        profile_id: row.profile_id,
        location: row.profile_location.map(proxy_location),
        name: row.name,
        description: row.description,
        status,
        initiator_id: row.started_by,
        participant_ids,
        ttl: ttl_from_dates(&row.started_at, &row.expires_at)?,
        started_at: parse_timestamp(&row.started_at, "session.started_at")?,
        expires_at: parse_timestamp(&row.expires_at, "session.expires_at")?,
        ended_at: optional_timestamp(row.ended_at.as_deref(), "session.ended_at")?,
        end_note: row.end_note,
        usage: usage_total(std::slice::from_ref(usage))?,
    })
}

fn runtime_from_row(
    row: &SqliteRow,
    org_id: &str,
    session_id: &str,
    secrets: &SecretBox,
) -> StoreResult<ProviderRuntime> {
    let decrypt_required =
        |label: &'static str, context_field: &'static str, encrypted: Option<String>| -> StoreResult<String> {
            let encrypted = encrypted.ok_or_else(|| corrupt("session", format!("active session has no {label}")))?;
            secrets
                .open_for(&session_secret_context(org_id, session_id, context_field), &encrypted)
                .map_err(StoreError::Crypto)
        };
    Ok(ProviderRuntime {
        provider_session_id: row
            .try_get::<Option<String>, _>("provider_session_id")?
            .ok_or_else(|| corrupt("session", "active session has no provider session ID"))?,
        cdp_url: decrypt_required("CDP URL", "provider-cdp-url", row.try_get("provider_cdp_url_enc")?)?,
        live_url: decrypt_required("live URL", "provider-live-url", row.try_get("provider_live_url_enc")?)?,
        recording_url: row
            .try_get::<Option<String>, _>("provider_recording_url_enc")?
            .map(|encrypted| {
                secrets
                    .open_for(&session_secret_context(org_id, session_id, "provider-recording-url"), &encrypted)
                    .map_err(StoreError::Crypto)
            })
            .transpose()?,
    })
}

/// Length-prefix user-controlled identifiers so the authenticated context is
/// unambiguous even if an IAM identifier contains punctuation.
fn session_secret_context(org_id: &str, session_id: &str, field: &'static str) -> String {
    format!(
        "silicon-browser:v1:org[{}]:{}:session[{}]:{}:field:{field}",
        org_id.len(),
        org_id,
        session_id.len(),
        session_id
    )
}

fn command_secret_context(org_id: &str, session_id: &str, sequence: i64, actor_id: &str) -> String {
    format!(
        "{}:sequence:{sequence}:actor[{}]:{actor_id}",
        session_secret_context(org_id, session_id, "command"),
        actor_id.len()
    )
}

struct DbSession {
    id: String,
    profile_id: Option<String>,
    started_by: String,
    name: String,
    description: String,
    status: String,
    started_at: String,
    expires_at: String,
    ended_at: Option<String>,
    end_note: Option<String>,
    profile_location: Option<String>,
}

impl DbSession {
    fn from_row(row: &SqliteRow) -> StoreResult<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            profile_id: row.try_get("profile_id")?,
            started_by: row.try_get("started_by")?,
            name: row.try_get("name")?,
            description: row.try_get("description")?,
            status: row.try_get("status")?,
            started_at: row.try_get("started_at")?,
            expires_at: row.try_get("expires_at")?,
            ended_at: row.try_get("ended_at")?,
            end_note: row.try_get("end_note")?,
            profile_location: row.try_get("profile_location")?,
        })
    }

    fn is_live(&self) -> bool {
        matches!(self.status.as_str(), "starting" | "active" | "ending")
    }
}

fn map_usage(row: &SqliteRow, principal_ids: Vec<String>) -> StoreResult<Usage> {
    let browser_millis: i64 = row.try_get("browser_millis")?;
    let proxy_bytes_in: Option<i64> = row.try_get("proxy_bytes_in")?;
    let proxy_bytes_out: Option<i64> = row.try_get("proxy_bytes_out")?;
    let proxy_bytes_unclassified: Option<i64> = row.try_get("proxy_bytes_unclassified")?;
    let currency: String = row.try_get("currency")?;
    let browser_cost = parse_cost_micros(row.try_get("browser_cost")?)?;
    let proxy_cost = parse_cost_micros(row.try_get("proxy_cost")?)?;
    let bytes_in = nonnegative(proxy_bytes_in.unwrap_or(0), "usage.proxy_bytes_in")?;
    let bytes_out = nonnegative(proxy_bytes_out.unwrap_or(0), "usage.proxy_bytes_out")?;
    let bytes_unclassified = nonnegative(proxy_bytes_unclassified.unwrap_or(0), "usage.proxy_bytes_unclassified")?;
    let (proxy_in_cost, proxy_out_cost, proxy_unclassified_cost) =
        split_proxy_cost(proxy_cost, bytes_in, bytes_out, bytes_unclassified);
    let total = browser_cost.checked_add(proxy_cost).ok_or_else(|| corrupt("usage", "cost total overflow"))?;
    Ok(Usage {
        session_id: row.try_get("session_id")?,
        started_at: parse_timestamp(row.try_get("started_at")?, "usage.started_at")?,
        principal_ids,
        browser_seconds: nonnegative(browser_millis, "usage.browser_millis")? / 1_000,
        proxy_bytes_in: bytes_in,
        proxy_bytes_out: bytes_out,
        proxy_bytes_unclassified: bytes_unclassified,
        cost: UsageCost {
            browser: money(&currency, browser_cost),
            proxy_in: money(&currency, proxy_in_cost),
            proxy_out: money(&currency, proxy_out_cost),
            proxy_unclassified: money(&currency, proxy_unclassified_cost),
            total: money(&currency, total),
        },
    })
}

fn usage_total(rows: &[Usage]) -> StoreResult<UsageTotal> {
    if rows.is_empty() {
        return Ok(UsageTotal::default());
    }
    let currency = rows[0].cost.total.currency.clone();
    let mut total = UsageTotal { sessions: rows.len() as u64, ..UsageTotal::default() };
    total.cost.browser.currency = currency.clone();
    total.cost.proxy_in.currency = currency.clone();
    total.cost.proxy_out.currency = currency.clone();
    total.cost.proxy_unclassified.currency = currency.clone();
    total.cost.total.currency = currency.clone();
    for row in rows {
        if row.cost.total.currency != currency {
            return Err(corrupt("usage", "cannot aggregate different currencies"));
        }
        checked_add(&mut total.browser_seconds, row.browser_seconds, "browser seconds")?;
        checked_add(&mut total.proxy_bytes_in, row.proxy_bytes_in, "proxy bytes in")?;
        checked_add(&mut total.proxy_bytes_out, row.proxy_bytes_out, "proxy bytes out")?;
        checked_add(&mut total.proxy_bytes_unclassified, row.proxy_bytes_unclassified, "proxy bytes unclassified")?;
        checked_add(&mut total.cost.browser.micros, row.cost.browser.micros, "browser cost")?;
        checked_add(&mut total.cost.proxy_in.micros, row.cost.proxy_in.micros, "proxy-in cost")?;
        checked_add(&mut total.cost.proxy_out.micros, row.cost.proxy_out.micros, "proxy-out cost")?;
        checked_add(
            &mut total.cost.proxy_unclassified.micros,
            row.cost.proxy_unclassified.micros,
            "unclassified proxy cost",
        )?;
        checked_add(&mut total.cost.total.micros, row.cost.total.micros, "total cost")?;
    }
    Ok(total)
}

fn checked_add(target: &mut u64, value: u64, name: &str) -> StoreResult<()> {
    *target = target.checked_add(value).ok_or_else(|| corrupt("usage", format!("{name} overflow")))?;
    Ok(())
}

fn split_proxy_cost(total: u64, bytes_in: u64, bytes_out: u64, bytes_unclassified: u64) -> (u64, u64, u64) {
    let bytes = bytes_in as u128 + bytes_out as u128 + bytes_unclassified as u128;
    if bytes == 0 {
        return (0, 0, total);
    }
    let inbound = (total as u128 * bytes_in as u128 / bytes) as u64;
    let outbound = (total as u128 * bytes_out as u128 / bytes) as u64;
    (inbound, outbound, total - inbound - outbound)
}

fn parse_cost_micros(value: &str) -> StoreResult<u64> {
    decimal_to_millionths(value).map_err(|error| StoreError::Invalid(format!("cost {error}")))
}

fn validate_currency(value: &str) -> StoreResult<()> {
    if value.len() != 3 || !value.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Err(StoreError::Invalid("currency must be a three-letter code".into()));
    }
    Ok(())
}

fn ttl_from_dates(started: &str, expires: &str) -> StoreResult<SessionTtl> {
    let started = parse_timestamp(started, "session.started_at")?;
    let expires = parse_timestamp(expires, "session.expires_at")?;
    let seconds = expires.signed_duration_since(started).num_seconds();
    SessionTtl::ALL
        .into_iter()
        .find(|ttl| ttl.seconds() == seconds)
        .ok_or_else(|| corrupt("session", format!("unsupported persisted TTL of {seconds} seconds")))
}

fn identity_kind(kind: IdentityKind) -> &'static str {
    match kind {
        IdentityKind::Carbon => "carbon",
        IdentityKind::Silicon => "silicon",
    }
}

fn proxy_location(code: String) -> ProxyLocation {
    ProxyLocation { name: code.to_ascii_uppercase(), country: Some(code.to_ascii_uppercase()), code }
}

fn same_principal(left: &str, right: &str) -> bool {
    left.trim().trim_start_matches('@') == right.trim().trim_start_matches('@')
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn parse_timestamp(value: &str, field: &'static str) -> StoreResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| corrupt("timestamp", format!("{field}: {error}")))
}

fn optional_timestamp(value: Option<&str>, field: &'static str) -> StoreResult<Option<DateTime<Utc>>> {
    value.map(|value| parse_timestamp(value, field)).transpose()
}

fn required(value: &str, field: &'static str) -> StoreResult<()> {
    if value.trim().is_empty() { Err(StoreError::Invalid(format!("{field} is required"))) } else { Ok(()) }
}

fn safe_id(value: &str, field: &'static str) -> StoreResult<()> {
    required(value, field)?;
    if value
        .chars()
        .any(|character| character.is_control() || character.is_whitespace() || matches!(character, '/' | '\\'))
    {
        return Err(StoreError::Invalid(format!("{field} contains unsafe characters")));
    }
    Ok(())
}

fn to_i64(value: u64, field: &'static str) -> StoreResult<i64> {
    i64::try_from(value).map_err(|_| StoreError::Invalid(format!("{field} is too large")))
}

fn optional_i64(value: Option<u64>, field: &'static str) -> StoreResult<Option<i64>> {
    value.map(|value| to_i64(value, field)).transpose()
}

fn monotonic_terminal_counter(next: Option<i64>, current: Option<i64>, terminal: bool) -> Option<i64> {
    if terminal { next.map(|next| current.map_or(next, |current| next.max(current))) } else { next }
}

fn nonnegative(value: i64, field: &'static str) -> StoreResult<u64> {
    u64::try_from(value).map_err(|_| corrupt("integer", format!("{field} is negative")))
}

fn money(currency: &str, micros: u64) -> Money {
    Money { currency: currency.to_owned(), micros }
}

fn projected_identity_value(public_id: &str, principal_id: &str, kind: IdentityKind) -> Identity {
    let verified_aliases = (public_id != principal_id).then(|| principal_id.to_owned()).into_iter().collect();
    Identity { id: public_id.into(), name: public_id.into(), kind, tags: Vec::new(), verified_aliases }
}

fn invalid(error: silicon_browser_shared::ValidationError) -> StoreError {
    StoreError::Invalid(error.to_string())
}

fn corrupt(kind: &'static str, reason: impl Into<String>) -> StoreError {
    StoreError::Corrupt { kind, reason: reason.into() }
}

fn corrupt_json(error: serde_json::Error) -> StoreError {
    corrupt("json", error.to_string())
}

fn not_found(kind: &'static str, id: &str) -> StoreError {
    StoreError::NotFound { kind, id: id.to_owned() }
}

fn forbidden(kind: &'static str, id: &str) -> StoreError {
    StoreError::Forbidden { kind, id: id.to_owned() }
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database) if database.is_unique_violation())
}

fn map_profile_write(error: sqlx::Error) -> StoreError {
    if is_unique_violation(&error) {
        StoreError::Invalid("provider profile or fingerprint already exists".into())
    } else {
        StoreError::Database(error)
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, day, hour, 0, 0).unwrap()
    }

    fn report(command: &str, exit_code: i32) -> silicon_browser_shared::CommandReport {
        silicon_browser_shared::CommandReport {
            command_id: Uuid::now_v7(),
            command: command.into(),
            flags: vec![],
            started_at: at(2, 10),
            finished_at: at(2, 10),
            exit_code,
            truncated: false,
        }
    }

    fn identity(id: &str, kind: IdentityKind, tags: &[&str]) -> Identity {
        Identity {
            id: id.into(),
            name: id.into(),
            kind,
            tags: tags.iter().map(ToString::to_string).collect(),
            verified_aliases: Vec::new(),
        }
    }

    fn silicon(id: &str) -> Identity {
        identity(id, IdentityKind::Silicon, &[])
    }

    fn profile_create(access: &[&str]) -> ProfileCreate {
        ProfileCreate { name: "Primary".into(), location: "in".into(), access: AccessList::new(access).unwrap() }
    }

    fn session_create(profile_id: &str) -> SessionCreate {
        SessionCreate::with_profile(profile_id, "Market scan", "Research browser vendors", SessionTtl::Minutes30)
    }

    fn provider(id: &str) -> ProviderSession {
        ProviderSession {
            id: id.into(),
            cdp_url: format!("wss://provider.test/{id}/cdp?secret=yes"),
            live_url: format!("https://provider.test/{id}/live?secret=yes"),
            recording_url: Some(format!("https://provider.test/{id}/recording?secret=yes")),
        }
    }

    fn secrets() -> SecretBox {
        SecretBox::new(&[9; 32])
    }

    async fn create_profile(store: &Store, owner: &Identity, access: &[&str], suffix: &str) -> Profile {
        store
            .create_profile(
                "org-1",
                owner,
                &profile_create(access),
                &format!("provider-profile-{suffix}"),
                &format!("fingerprint-{suffix}"),
                at(1, 10),
            )
            .await
            .unwrap()
    }

    /// Test group: learning IAM's public id after OAT-only use preserves
    /// ownership while replacing the internal principal UUID everywhere it
    /// could otherwise escape through the public API.
    #[tokio::test]
    async fn identity_projection_canonicalizes_existing_resources_atomically() {
        let store = Store::in_memory().await.unwrap();
        let principal_id = "018f75dc-8d7e-7c32-91df-fd7e2ea93731";
        let fallback = silicon(principal_id);
        let profile = create_profile(&store, &fallback, &[principal_id], "projection").await;
        let session = store
            .reserve_session(
                "org-1",
                &fallback,
                &SessionCreate::incognito("Private", "Projection test", SessionTtl::Minutes15),
                at(1, 10),
            )
            .await
            .unwrap();
        let profile_session = active_session(&store, &fallback, &profile.id, "projection-command").await;
        store
            .report_command(
                "org-1",
                &fallback,
                principal_id,
                &profile_session.id,
                &report("snapshot -i", 0),
                &secrets(),
                at(2, 10),
            )
            .await
            .unwrap();

        let projected = store
            .remember_identity_projection(
                "org-1",
                principal_id,
                "silicon-1",
                IdentityKind::Silicon,
                &secrets(),
                at(1, 11),
            )
            .await
            .unwrap();
        assert!(projected.matches_principal(principal_id));
        assert_eq!(projected.verified_aliases, [principal_id]);

        let profile = store.profile("org-1", &projected, &profile.id).await.unwrap();
        assert_eq!(profile.owner_id, "silicon-1");
        assert!(profile.access.contains_principal("silicon-1"));
        assert!(!profile.access.contains_principal(principal_id));
        store
            .update_profile(
                "org-1",
                &projected,
                &profile.id,
                &ProfileUpdate { name: Some("Still mine".into()), access: None },
            )
            .await
            .unwrap();

        let session = store.session("org-1", &projected, &session.id).await.unwrap();
        assert_eq!(session.initiator_id, "silicon-1");
        assert_eq!(session.participant_ids, ["silicon-1"]);
        let recording = store.recording("org-1", &projected, &session.id, &secrets()).await.unwrap();
        assert_eq!(recording.owner_id, "silicon-1");
        assert!(recording.briefcase_path.is_empty());
        assert_eq!(
            store.session_logs("org-1", &projected, &profile_session.id, None, &secrets()).await.unwrap()[0].command,
            "snapshot -i"
        );

        let cached = store.projected_identity("org-1", principal_id, IdentityKind::Silicon).await.unwrap().unwrap();
        assert!(cached.matches_principal(principal_id));

        let rotated = store
            .remember_identity_projection(
                "org-1",
                principal_id,
                "silicon-2",
                IdentityKind::Silicon,
                &secrets(),
                at(3, 10),
            )
            .await
            .unwrap();
        assert_eq!(store.profile("org-1", &rotated, &profile.id).await.unwrap().owner_id, "silicon-2");
        assert_eq!(store.session("org-1", &rotated, &session.id).await.unwrap().initiator_id, "silicon-2");
        assert_eq!(
            store.session_logs("org-1", &rotated, &profile_session.id, None, &secrets()).await.unwrap()[0].actor_id,
            "silicon-2"
        );
    }

    /// Test group: upstream profile identity is attached only after a stable
    /// local id has been reserved, and failed reservations never become usable.
    #[tokio::test]
    async fn profile_provisioning_is_two_phase_and_recoverable() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let reserved = store.reserve_profile("org-1", &owner, &profile_create(&[]), at(1, 10)).await.unwrap();
        assert_eq!(
            store.provisioning_profiles().await.unwrap(),
            [ProvisioningProfile { org_id: "org-1".into(), profile_id: reserved.id.clone() }]
        );
        assert!(store.profiles("org-1", &owner).await.unwrap().is_empty());
        assert!(matches!(
            store.provider_profile_id("org-1", &owner, &reserved.id).await,
            Err(StoreError::ProfileRetired { .. })
        ));
        let active =
            store.activate_profile("org-1", &reserved.id, "provider-two-phase", "sbf_two_phase").await.unwrap();
        assert_eq!(active.id, reserved.id);
        assert_eq!(store.provider_profile_id("org-1", &owner, &reserved.id).await.unwrap(), "provider-two-phase");
        assert!(store.provisioning_profiles().await.unwrap().is_empty());

        let failed = store.reserve_profile("org-1", &owner, &profile_create(&[]), at(1, 10)).await.unwrap();
        store.fail_profile("org-1", &failed.id, "provider failed", at(1, 11)).await.unwrap();
        assert!(matches!(
            store.reserve_session("org-1", &owner, &session_create(&failed.id), at(2, 10)).await,
            Err(StoreError::ProfileRetired { .. })
        ));
    }

    async fn active_session(store: &Store, initiator: &Identity, profile_id: &str, suffix: &str) -> Session {
        let reserved = store.reserve_session("org-1", initiator, &session_create(profile_id), at(2, 10)).await.unwrap();
        store.activate_session("org-1", &reserved.id, &provider(suffix), &secrets(), at(2, 10)).await.unwrap()
    }

    /// Test group: profile ACLs are the owner/principal/tag union and lists do not leak.
    #[tokio::test]
    async fn profile_access_is_normalized_and_scoped() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let profile = create_profile(&store, &owner, &["growth", "@carbon-1", "growth"], "acl").await;
        assert_eq!(profile.access.as_slice(), ["@owner-1", "growth", "@carbon-1"]);

        let tagged = identity("silicon-2", IdentityKind::Silicon, &["growth"]);
        let explicit = identity("carbon-1", IdentityKind::Carbon, &[]);
        let stranger = silicon("stranger");
        assert_eq!(store.profiles("org-1", &tagged).await.unwrap().len(), 1);
        assert_eq!(store.profiles("org-1", &explicit).await.unwrap().len(), 1);
        assert!(store.profiles("org-1", &stranger).await.unwrap().is_empty());
        assert!(matches!(store.profile("org-1", &stranger, &profile.id).await, Err(StoreError::NotFound { .. })));
    }

    /// Test group: only mutable profile fields change and access replacement retains the owner.
    #[tokio::test]
    async fn profile_updates_preserve_immutable_identity() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let profile = create_profile(&store, &owner, &["growth"], "immutable").await;
        let update = ProfileUpdate { name: Some("Renamed".into()), access: Some(AccessList::new(["sales"]).unwrap()) };
        let updated = store.update_profile("org-1", &owner, &profile.id, &update).await.unwrap();
        assert_eq!(updated.name, "Renamed");
        assert_eq!(updated.location, profile.location);
        assert_eq!(updated.fingerprint, profile.fingerprint);
        assert_eq!(updated.access.as_slice(), ["@owner-1", "sales"]);
        assert!(store.update_profile("org-1", &silicon("other"), &profile.id, &update).await.is_err());
    }

    /// Test group: retirement is durable, keeps history, and blocks future sessions.
    #[tokio::test]
    async fn retired_profiles_remain_visible_but_unusable() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let profile = create_profile(&store, &owner, &[], "retire").await;
        let retired = store
            .end_profile("org-1", &owner, &profile.id, &ProfileEnd { note: "rotated".into() }, at(3, 10))
            .await
            .unwrap();
        assert_eq!(retired.status, ProfileStatus::Retired);
        assert_eq!(retired.end_note.as_deref(), Some("rotated"));
        assert!(matches!(
            store.reserve_session("org-1", &owner, &session_create(&profile.id), at(4, 10)).await,
            Err(StoreError::ProfileRetired { .. })
        ));
        assert_eq!(store.profile("org-1", &owner, &profile.id).await.unwrap().status, ProfileStatus::Retired);
    }

    /// Test group: concurrent reservations have one winner and return the active owner/TTL to the loser.
    #[tokio::test]
    async fn profile_session_reservation_is_atomic() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let profile = create_profile(&store, &owner, &[], "race").await;
        let request_a = session_create(&profile.id);
        let request_b = session_create(&profile.id);
        let (a, b) = tokio::join!(
            store.reserve_session("org-1", &owner, &request_a, at(2, 10)),
            store.reserve_session("org-1", &owner, &request_b, at(2, 10))
        );
        let results = [a, b];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let busy = results.into_iter().find_map(Result::err).unwrap();
        assert!(matches!(busy, StoreError::ProfileBusy { actor_id, .. } if actor_id == owner.id));
    }

    /// Test group: TTL work is leased once, remains retryable, and releases the
    /// profile only after provider-stop finalization.
    #[tokio::test]
    async fn ttl_expiry_releases_profile() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let profile = create_profile(&store, &owner, &[], "ttl").await;
        let request = SessionCreate::with_profile(&profile.id, "short", "expires", SessionTtl::Minutes15);
        let reserved = store.reserve_session("org-1", &owner, &request, at(2, 10)).await.unwrap();
        store.activate_session("org-1", &reserved.id, &provider("ttl"), &secrets(), at(2, 10)).await.unwrap();
        let secret_box = secrets();
        let (first, competing) = tokio::join!(
            store.claim_expired_sessions(at(2, 11), at(2, 11) + ChronoDuration::minutes(2), 100, &secret_box),
            store.claim_expired_sessions(at(2, 11), at(2, 11) + ChronoDuration::minutes(2), 100, &secret_box)
        );
        let mut claims = [first.unwrap(), competing.unwrap()];
        assert_eq!(claims.iter().map(Vec::len).sum::<usize>(), 1);
        let claim = claims.iter_mut().find_map(Vec::pop).unwrap();
        assert!(matches!(
            store.reserve_session("org-1", &owner, &request, at(2, 11)).await,
            Err(StoreError::ProfileBusy { .. })
        ));
        store.release_expired_claim("org-1", &claim.session_id, &claim.lease_id).await.unwrap();
        let retry = store
            .claim_expired_sessions(at(2, 11), at(2, 11) + ChronoDuration::minutes(2), 100, &secret_box)
            .await
            .unwrap()
            .pop()
            .unwrap();
        store.mark_recording_pending("org-1", &retry.session_id, 60_000, 0).await.unwrap();
        store.finalize_expired_session("org-1", &retry.session_id, &retry.lease_id, at(2, 11)).await.unwrap();
        let expired = store.session("org-1", &owner, &reserved.id).await.unwrap();
        assert_eq!(expired.status, SessionStatus::Expired);
        assert_eq!(expired.end_note.as_deref(), Some("ttl reached"));
        assert!(store.reserve_session("org-1", &owner, &request, at(2, 11)).await.is_ok());
    }

    #[tokio::test]
    async fn expired_stop_claim_must_still_own_the_ending_session_and_live_lease() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let request = SessionCreate::incognito("short", "claim validation", SessionTtl::Minutes15);
        let reserved = store.reserve_session("org-1", &owner, &request, at(2, 10)).await.unwrap();
        store.activate_session("org-1", &reserved.id, &provider("claim"), &secrets(), at(2, 10)).await.unwrap();
        let claim = store
            .claim_expired_sessions(at(2, 11), at(2, 11) + ChronoDuration::minutes(2), 100, &secrets())
            .await
            .unwrap()
            .pop()
            .unwrap();

        assert!(store.expired_claim_is_current("org-1", &claim.session_id, &claim.lease_id, at(2, 11)).await.unwrap());
        assert!(
            !store
                .expired_claim_is_current(
                    "org-1",
                    &claim.session_id,
                    &claim.lease_id,
                    at(2, 11) + ChronoDuration::minutes(3)
                )
                .await
                .unwrap()
        );
        store.finalize_end_session("org-1", &claim.session_id, at(2, 11)).await.unwrap();
        assert!(!store.expired_claim_is_current("org-1", &claim.session_id, &claim.lease_id, at(2, 11)).await.unwrap());
    }

    #[tokio::test]
    async fn unactivated_start_expiry_does_not_enqueue_a_missing_recording_source() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let profile = create_profile(&store, &owner, &[], "unactivated-expiry").await;
        let request = session_create(&profile.id);
        let reserved = store.reserve_session("org-1", &owner, &request, at(2, 10)).await.unwrap();

        assert!(
            store
                .claim_expired_sessions(at(2, 11), at(2, 11) + ChronoDuration::minutes(2), 100, &secrets(),)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(store.session("org-1", &owner, &reserved.id).await.unwrap().status, SessionStatus::Expired);
        assert_eq!(
            store.recording("org-1", &owner, &reserved.id, &secrets()).await.unwrap().status,
            RecordingStatus::Failed
        );
        let queued: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM outbox WHERE event_type = 'recording.store' AND payload_json LIKE ?",
        )
        .bind(format!("%{}%", reserved.id))
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(queued, 0);
    }

    /// Test group: incognito sessions have no profile/location/proxy counters and do not serialize one another.
    #[tokio::test]
    async fn incognito_sessions_are_profileless_and_parallel() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let request = SessionCreate::incognito("private", "captcha", SessionTtl::Minutes15);
        let first = store.reserve_session("org-1", &owner, &request, at(2, 10)).await.unwrap();
        let second = store.reserve_session("org-1", &owner, &request, at(2, 10)).await.unwrap();
        assert_ne!(first.id, second.id);
        assert!(first.incognito);
        assert!(first.profile_id.is_none());
        assert!(first.location.is_none());
        assert_eq!(first.usage.proxy_bytes_in, 0);
        assert_eq!(first.usage.proxy_bytes_out, 0);
    }

    /// Test group: provider activation decrypts only at the live-link boundary and failure releases the lock.
    #[tokio::test]
    async fn activation_and_failure_are_compare_and_swap_transitions() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let profile = create_profile(&store, &owner, &[], "activate").await;
        let session = active_session(&store, &owner, &profile.id, "activate").await;
        let live = store.live_link("org-1", &owner, &session.id, &secrets()).await.unwrap();
        assert!(live.url.contains("secret=yes"));
        let runtime = store.provider_runtime("org-1", &owner, &session.id, &secrets()).await.unwrap();
        assert_eq!(runtime.provider_session_id, "activate");
        assert!(runtime.cdp_url.contains("secret=yes"));
        let debug = format!("{runtime:?}");
        assert!(!debug.contains("secret=yes"));
        assert!(debug.contains("[REDACTED]"));
        assert!(matches!(
            store.provider_runtime("org-1", &silicon("stranger"), &session.id, &secrets()).await,
            Err(StoreError::NotFound { .. })
        ));
        assert!(store.activate_session("org-1", &session.id, &provider("again"), &secrets(), at(2, 10)).await.is_err());
        let failed = store.fail_session("org-1", &session.id, "provider disconnected", at(2, 11)).await.unwrap();
        assert_eq!(failed.status, SessionStatus::Ended);
    }

    /// Test group: a provider-backed session keeps the profile lock throughout
    /// `ending`, and only a confirmed finalization releases it.
    #[tokio::test]
    async fn provider_stop_is_two_phase_and_retryable() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let profile = create_profile(&store, &owner, &[], "ending").await;
        let session = active_session(&store, &owner, &profile.id, "ending").await;
        let end = SessionEnd { note: "done".into() };
        let runtime = store.begin_end_session("org-1", &owner, &session.id, &end, &secrets()).await.unwrap();
        assert_eq!(runtime.provider_session_id, "ending");
        assert!(matches!(
            store.reserve_session("org-1", &owner, &session_create(&profile.id), at(2, 11)).await,
            Err(StoreError::ProfileBusy { .. })
        ));
        // Retrying an uncertain provider stop resolves the same encrypted runtime.
        assert_eq!(
            store.begin_end_session("org-1", &owner, &session.id, &end, &secrets()).await.unwrap().provider_session_id,
            "ending"
        );
        store.mark_recording_pending("org-1", &session.id, 1_000, 0).await.unwrap();
        let ended = store.finalize_end_session("org-1", &session.id, at(2, 11)).await.unwrap();
        assert_eq!(ended.status, SessionStatus::Ended);
        assert!(store.reserve_session("org-1", &owner, &session_create(&profile.id), at(2, 11)).await.is_ok());
    }

    #[tokio::test]
    async fn failed_activation_runtime_retains_profile_until_confirmed_stop_finalization() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let profile = create_profile(&store, &owner, &[], "activation-recovery").await;
        let request = session_create(&profile.id);
        let reserved = store.reserve_session("org-1", &owner, &request, at(2, 10)).await.unwrap();
        let runtime = provider("activation-recovery");
        store
            .retain_session_runtime("org-1", &reserved.id, &runtime, "local activation failed", &secrets())
            .await
            .unwrap();
        assert!(matches!(
            store.reserve_session("org-1", &owner, &request, at(2, 11)).await,
            Err(StoreError::ProfileBusy { .. })
        ));
        assert_eq!(
            store
                .begin_end_session("org-1", &owner, &reserved.id, &SessionEnd { note: "retry".into() }, &secrets(),)
                .await
                .unwrap()
                .provider_session_id,
            runtime.id
        );
        store
            .remember_provider_recording_url(
                "org-1",
                &reserved.id,
                runtime.recording_url.as_deref().unwrap(),
                &secrets(),
            )
            .await
            .unwrap();
        store
            .finalize_failed_session_after_stop(
                "org-1",
                &reserved.id,
                FailedSessionFinalization {
                    provider_session_id: &runtime.id,
                    reason: "local activation failed",
                    duration_ms: 1_000,
                    has_recording_source: true,
                    at: at(2, 11),
                },
            )
            .await
            .unwrap();
        assert!(store.reserve_session("org-1", &owner, &request, at(2, 11)).await.is_ok());
        assert_eq!(
            store.recording("org-1", &owner, &reserved.id, &secrets()).await.unwrap().status,
            RecordingStatus::Pending
        );
    }

    #[tokio::test]
    async fn failed_activation_stop_without_url_queues_recording_resolution() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let profile = create_profile(&store, &owner, &[], "activation-delayed-recording").await;
        let request = session_create(&profile.id);
        let reserved = store.reserve_session("org-1", &owner, &request, at(2, 10)).await.unwrap();
        let runtime = provider("activation-delayed-recording");
        store
            .finalize_failed_session_after_stop(
                "org-1",
                &reserved.id,
                FailedSessionFinalization {
                    provider_session_id: &runtime.id,
                    reason: "local activation failed",
                    duration_ms: 1_000,
                    has_recording_source: false,
                    at: at(2, 11),
                },
            )
            .await
            .unwrap();

        assert_eq!(
            store.recording("org-1", &owner, &reserved.id, &secrets()).await.unwrap().status,
            RecordingStatus::Pending
        );
        let claim = store
            .claim_recording_source_resolutions(
                at(2, 11) + ChronoDuration::seconds(15),
                at(2, 11) + ChronoDuration::minutes(3),
                1,
            )
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(claim.provider_session_id, runtime.id);
        assert_eq!(claim.attempt, 1);
    }

    /// Test group: silicon command logs are encrypted, sequenced, participant-aware, and replayed in order.
    #[tokio::test]
    async fn command_logs_are_ordered_and_encrypted() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let profile = create_profile(&store, &owner, &["@runner-1"], "commands").await;
        let session = active_session(&store, &owner, &profile.id, "commands").await;
        let box_ = secrets();
        let runner = silicon("runner-1");
        let one = store
            .report_command(
                "org-1",
                &runner,
                "runner-principal",
                &session.id,
                &report("open https://secret.test", 0),
                &box_,
                at(2, 10),
            )
            .await
            .unwrap();
        let two = store
            .report_command(
                "org-1",
                &runner,
                "runner-principal",
                &session.id,
                &report("snapshot -i", 2),
                &box_,
                at(2, 10),
            )
            .await
            .unwrap();
        assert_eq!(one.sequence, Some(1));
        assert_eq!(two.sequence, Some(2));

        let encrypted: Vec<String> = sqlx::query_scalar("SELECT command_enc FROM commands ORDER BY sequence")
            .fetch_all(&store.pool)
            .await
            .unwrap();
        assert!(encrypted.iter().all(|value| !value.contains("secret.test") && !value.contains("snapshot")));
        let logs = store.session_logs("org-1", &owner, &session.id, Some(at(2, 10).date_naive()), &box_).await.unwrap();
        assert_eq!(logs.iter().map(|entry| entry.sequence).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(logs[0].command, "open https://secret.test");
        assert_eq!(logs[1].exit_code, Some(2));
        let participants = store.session("org-1", &owner, &session.id).await.unwrap().participant_ids;
        assert!(participants.contains(&"runner-1".into()));

        sqlx::query("UPDATE commands SET command_enc = CASE sequence WHEN 1 THEN ? WHEN 2 THEN ? END")
            .bind(&encrypted[1])
            .bind(&encrypted[0])
            .execute(&store.pool)
            .await
            .unwrap();
        assert!(matches!(
            store.session_logs("org-1", &owner, &session.id, None, &box_).await,
            Err(StoreError::Crypto(_))
        ));
    }

    /// Test group: direct connection renewal requires the current profile ACL.
    #[tokio::test]
    async fn connection_renewal_rechecks_current_profile_access() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let runner = silicon("runner-1");
        let profile = create_profile(&store, &owner, &["@runner-1"], "command-auth").await;
        let session = active_session(&store, &owner, &profile.id, "command-auth").await;
        store
            .update_profile(
                "org-1",
                &owner,
                &profile.id,
                &ProfileUpdate { name: None, access: Some(AccessList::default()) },
            )
            .await
            .unwrap();

        assert!(matches!(
            store.connection_runtime("org-1", &runner, &session.id, &secrets(), at(2, 10)).await,
            Err(StoreError::NotFound { kind: "session", .. })
        ));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM commands WHERE session_id = ?")
            .bind(&session.id)
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    /// Test group: carbon-initiated sessions execute without retaining replayable commands.
    #[tokio::test]
    async fn carbon_sessions_do_not_persist_command_logs() {
        let store = Store::in_memory().await.unwrap();
        let owner = identity("carbon-1", IdentityKind::Carbon, &[]);
        let profile = create_profile(&store, &owner, &[], "carbon-log").await;
        let session = active_session(&store, &owner, &profile.id, "carbon-log").await;
        let ticket = store
            .report_command(
                "org-1",
                &owner,
                "carbon-principal",
                &session.id,
                &report("open https://example.com", 0),
                &secrets(),
                at(2, 10),
            )
            .await
            .unwrap();
        assert_eq!(ticket.sequence, None);
        assert!(store.session_logs("org-1", &owner, &session.id, None, &secrets()).await.unwrap().is_empty());
    }

    /// Test group: recording discovery uses source-session metadata, includes
    /// the initiating owner as an actor, and treats profile-ACL visibility as
    /// shared when the viewer does not own the recording.
    #[tokio::test]
    async fn recording_discovery_filters_metadata_actors_and_acl_shares() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let acl_viewer = silicon("acl-viewer");
        let runner = identity("carbon-1", IdentityKind::Carbon, &[]);
        let stranger = silicon("stranger");
        let profile = create_profile(&store, &owner, &["@acl-viewer"], "recording-discovery").await;
        let session = active_session(&store, &owner, &profile.id, "recording-discovery").await;
        store
            .associate_participant(
                "org-1",
                &session.id,
                &runner.id,
                ParticipantRole::Runner,
                at(2, 10) + ChronoDuration::minutes(1),
            )
            .await
            .unwrap();

        let visible = store.recording("org-1", &acl_viewer, &session.id, &secrets()).await.unwrap();
        assert_eq!(visible.profile_id.as_deref(), Some(profile.id.as_str()));
        assert!(!visible.incognito);
        assert_eq!(visible.participant_ids, ["owner-1", "carbon-1"]);

        let metadata = RecordingFilter::parse(&format!(
            "profile:{} -> for:@owner-1 -> for:@carbon-1 -> name:market* -> description:^research -> is:shared",
            profile.id
        ))
        .unwrap();
        assert_eq!(store.recordings("org-1", &acl_viewer, Some(&metadata), &secrets()).await.unwrap().len(), 1);
        assert!(store.recordings("org-1", &owner, Some(&metadata), &secrets()).await.unwrap().is_empty());
        assert!(store.recordings("org-1", &stranger, Some(&metadata), &secrets()).await.unwrap().is_empty());

        let mine = RecordingFilter::parse("is:mine -> for:@owner-1").unwrap();
        let mut owner_alias = silicon("canonical-owner");
        owner_alias.verified_aliases.push(owner.id.clone());
        assert_eq!(store.recordings("org-1", &owner_alias, Some(&mine), &secrets()).await.unwrap().len(), 1);

        let incognito_request =
            SessionCreate::incognito("Private market scan", "Research without a profile", SessionTtl::Minutes15);
        let incognito = store.reserve_session("org-1", &owner, &incognito_request, at(2, 12)).await.unwrap();
        store
            .activate_session("org-1", &incognito.id, &provider("recording-incognito"), &secrets(), at(2, 12))
            .await
            .unwrap();
        let incognito_filter = RecordingFilter::parse("is:incognito -> for:@owner-1 -> name:private*").unwrap();
        let values = store.recordings("org-1", &owner, Some(&incognito_filter), &secrets()).await.unwrap();
        assert_eq!(values.len(), 1);
        assert!(values[0].incognito);
        assert!(values[0].profile_id.is_none());
        assert!(store.recordings("org-1", &acl_viewer, Some(&incognito_filter), &secrets()).await.unwrap().is_empty());
    }

    /// Test group: recording finalization remains pending and trash metadata retains exactly 45 days.
    #[tokio::test]
    async fn recordings_are_pending_then_recoverably_trashed() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let profile = create_profile(&store, &owner, &[], "recording").await;
        let session = active_session(&store, &owner, &profile.id, "recording").await;
        store.end_session("org-1", &session.id, &SessionEnd { note: "done".into() }, at(2, 11)).await.unwrap();
        let pending = store.mark_recording_pending("org-1", &session.id, 90_500, 1_024).await.unwrap();
        assert_eq!(pending.status, RecordingStatus::Pending);
        assert_eq!(pending.duration_seconds, 90);
        assert!(pending.briefcase_path.is_empty());
        let matching = RecordingFilter::parse("contains:market -> is:mine").unwrap();
        assert_eq!(store.recordings("org-1", &owner, Some(&matching), &secrets()).await.unwrap().len(), 1);
        let missing = RecordingFilter::parse("contains:checkout").unwrap();
        assert!(store.recordings("org-1", &owner, Some(&missing), &secrets()).await.unwrap().is_empty());

        let trashed = store.trash_recording("org-1", &owner, &session.id, at(3, 10), &secrets()).await.unwrap();
        assert_eq!(trashed.status, RecordingStatus::Trashed);
        assert!(trashed.purge_at.is_none());
        store.trash_recording("org-1", &owner, &session.id, at(3, 11), &secrets()).await.unwrap();
        let trash_events: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM outbox WHERE id = ? AND event_type = 'recording.trash' AND done_at IS NULL",
        )
        .bind(format!("recording.trash:{}", session.id))
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(trash_events, 0);
        let store_done_at: Option<String> = sqlx::query_scalar("SELECT done_at FROM outbox WHERE id = ?")
            .bind(format!("recording.store:{}", session.id))
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert!(store_done_at.is_some());
    }

    /// Test group: a terminal browser without an immediate recording source is
    /// durably leased, retryable after crashes, and only creates Briefcase work
    /// once the encrypted URL is present.
    #[tokio::test]
    async fn delayed_recording_source_is_durably_resolved() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let profile = create_profile(&store, &owner, &[], "no-recording").await;
        let request = session_create(&profile.id);
        let reserved = store.reserve_session("org-1", &owner, &request, at(2, 10)).await.unwrap();
        let mut runtime = provider("no-recording");
        runtime.recording_url = None;
        store.activate_session("org-1", &reserved.id, &runtime, &secrets(), at(2, 10)).await.unwrap();
        store
            .begin_end_session("org-1", &owner, &reserved.id, &SessionEnd { note: "done".into() }, &secrets())
            .await
            .unwrap();
        let queued = store.queue_recording_source_resolution("org-1", &reserved.id, 1_000, at(2, 11)).await.unwrap();
        store.finalize_end_session("org-1", &reserved.id, at(2, 11)).await.unwrap();
        assert_eq!(queued.status, RecordingStatus::Pending);
        let store_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM outbox WHERE id = ?")
            .bind(format!("recording.store:{}", reserved.id))
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(store_events, 0);

        let first_due = at(2, 11) + ChronoDuration::seconds(15);
        assert!(
            store
                .claim_recording_source_resolutions(
                    first_due - ChronoDuration::milliseconds(1),
                    first_due + ChronoDuration::seconds(120),
                    10,
                )
                .await
                .unwrap()
                .is_empty()
        );
        let (first, competing) = tokio::join!(
            store.claim_recording_source_resolutions(first_due, first_due + ChronoDuration::seconds(120), 10,),
            store.claim_recording_source_resolutions(first_due, first_due + ChronoDuration::seconds(120), 10,),
        );
        let mut claims = [first.unwrap(), competing.unwrap()];
        assert_eq!(claims.iter().map(Vec::len).sum::<usize>(), 1);
        let stale = claims.iter_mut().find_map(Vec::pop).unwrap();
        assert_eq!(stale.attempt, 1);

        let retry_at = first_due + ChronoDuration::seconds(30);
        assert!(store.reschedule_recording_source_resolution(&stale, retry_at).await.unwrap());
        let current = store
            .claim_recording_source_resolutions(retry_at, retry_at + ChronoDuration::seconds(120), 10)
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(current.attempt, 2);
        let materialized = "https://provider.test/no-recording/recording?signature=secret";
        assert!(!store.complete_recording_source_resolution(&stale, materialized, &secrets(), retry_at).await.unwrap());
        assert!(
            store.complete_recording_source_resolution(&current, materialized, &secrets(), retry_at).await.unwrap()
        );

        let encrypted: String = sqlx::query_scalar("SELECT provider_recording_url_enc FROM sessions WHERE id = ?")
            .bind(&reserved.id)
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert!(!encrypted.contains("signature=secret"));
        let resolution_done: Option<String> = sqlx::query_scalar("SELECT done_at FROM outbox WHERE id = ?")
            .bind(format!("recording.resolve:{}", reserved.id))
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert!(resolution_done.is_some());
        let storage_pending: Option<String> = sqlx::query_scalar("SELECT done_at FROM outbox WHERE id = ?")
            .bind(format!("recording.store:{}", reserved.id))
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert!(storage_pending.is_none());
        assert_eq!(
            store.recording("org-1", &owner, &reserved.id, &secrets()).await.unwrap().status,
            RecordingStatus::Pending
        );
    }

    /// Test group: usage is monotonic, distinguishes browser/proxy units, filters, and aggregates exact costs.
    #[tokio::test]
    async fn usage_reads_filters_and_aggregates() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let profile = create_profile(&store, &owner, &[], "usage").await;
        let session = active_session(&store, &owner, &profile.id, "usage").await;
        let usage = store
            .record_usage(
                "org-1",
                &session.id,
                &UsageSample {
                    browser_millis: 90_000,
                    proxy_bytes_in: Some(1_000_000_000),
                    proxy_bytes_out: Some(1_000_000_000),
                    proxy_bytes_unclassified: None,
                    browser_cost: "0.25".into(),
                    proxy_cost: "0.50".into(),
                    currency: "usd".into(),
                    sampled_at: at(2, 11),
                },
            )
            .await
            .unwrap();
        assert_eq!(usage.browser_seconds, 90);
        assert_eq!(usage.cost.browser.micros, 250_000);
        assert_eq!(usage.cost.proxy_in.micros, 250_000);
        assert_eq!(usage.cost.proxy_out.micros, 250_000);
        assert_eq!(usage.cost.total.micros, 750_000);

        let filter = UsageFilter::parse("between:02-08-2026=02-08-2026 -> for:@owner-1").unwrap();
        let total = store.usage_total("org-1", &owner, Some(&filter)).await.unwrap();
        assert_eq!(total.sessions, 1);
        assert_eq!(total.browser_seconds, 90);
        assert_eq!(total.cost.total.micros, 750_000);

        // Organization billing is intentionally broader than the caller's
        // profile/session ACL. A second owner's private session is included,
        // while an otherwise identical session in another org is not.
        let other_owner = silicon("other-owner");
        let hidden_profile = create_profile(&store, &other_owner, &[], "hidden-usage").await;
        let hidden_session = active_session(&store, &other_owner, &hidden_profile.id, "hidden-usage").await;
        store
            .record_usage(
                "org-1",
                &hidden_session.id,
                &UsageSample {
                    browser_millis: 30_000,
                    proxy_bytes_in: Some(0),
                    proxy_bytes_out: Some(0),
                    proxy_bytes_unclassified: None,
                    browser_cost: "0.10".into(),
                    proxy_cost: "0".into(),
                    currency: "USD".into(),
                    sampled_at: at(2, 11),
                },
            )
            .await
            .unwrap();
        let other_org = store
            .reserve_session(
                "org-2",
                &other_owner,
                &SessionCreate::incognito("Other org", "Must not count", SessionTtl::Minutes15),
                at(2, 10),
            )
            .await
            .unwrap();
        store
            .activate_session("org-2", &other_org.id, &provider("other-org-usage"), &secrets(), at(2, 10))
            .await
            .unwrap();
        store
            .record_usage(
                "org-2",
                &other_org.id,
                &UsageSample {
                    browser_millis: 300_000,
                    proxy_bytes_in: None,
                    proxy_bytes_out: None,
                    proxy_bytes_unclassified: None,
                    browser_cost: "1".into(),
                    proxy_cost: "0".into(),
                    currency: "USD".into(),
                    sampled_at: at(2, 11),
                },
            )
            .await
            .unwrap();

        assert_eq!(store.usage_list("org-1", &owner, None).await.unwrap().len(), 1);
        let window = UsageFilter::parse("between:02-08-2026=02-08-2026").unwrap();
        let organization = store.org_usage_total("org-1", Some(&window)).await.unwrap();
        assert_eq!(organization.sessions, 2);
        assert_eq!(organization.browser_seconds, 120);
        assert_eq!(organization.cost.total.micros, 850_000);

        // The storage primitive remains exact for internal analytics; the HTTP
        // aggregate blocks this identity dimension for privacy.
        let owner_only = store.org_usage_total("org-1", Some(&filter)).await.unwrap();
        assert_eq!(owner_only.sessions, 1);
        assert_eq!(owner_only.cost.total.micros, 750_000);

        let decreasing = UsageSample {
            browser_millis: 89_000,
            proxy_bytes_in: Some(1_000_000_000),
            proxy_bytes_out: Some(1_000_000_000),
            proxy_bytes_unclassified: None,
            browser_cost: "0.25".into(),
            proxy_cost: "0.50".into(),
            currency: "USD".into(),
            sampled_at: at(2, 11),
        };
        assert!(store.record_usage("org-1", &session.id, &decreasing).await.is_err());
    }

    /// Test group: Browser Use's stop-time refund may lower costs, while a
    /// delayed active read must not overwrite the terminal bill and observed
    /// duration/traffic counters must never move backwards.
    #[tokio::test]
    async fn terminal_usage_accepts_refunds_and_ignores_stale_active_snapshots() {
        let directory = tempfile::tempdir().unwrap();
        let database_url = format!("sqlite://{}", directory.path().join("usage-race.db").display());
        let store = Store::connect(&database_url).await.unwrap();
        // WAL lets a second process commit the lifecycle/final-usage write
        // while the deliberately paused active sampler retains its old read
        // snapshot.
        sqlx::query("PRAGMA journal_mode = WAL").execute(&store.pool).await.unwrap();
        let owner = silicon("owner-1");
        let profile = create_profile(&store, &owner, &[], "usage-refund").await;
        let session = active_session(&store, &owner, &profile.id, "usage-refund").await;
        let active = UsageSample {
            browser_millis: 120_000,
            proxy_bytes_in: None,
            proxy_bytes_out: None,
            proxy_bytes_unclassified: Some(600_000),
            browser_cost: "0.24".into(),
            proxy_cost: "0.12".into(),
            currency: "USD".into(),
            sampled_at: at(2, 11),
        };
        store.record_usage("org-1", &session.id, &active).await.unwrap();

        let terminal = UsageSample {
            browser_millis: 90_000,
            proxy_bytes_unclassified: Some(500_000),
            browser_cost: "0.03".into(),
            proxy_cost: "0.10".into(),
            sampled_at: at(2, 12),
            ..active.clone()
        };

        // Pause a sampler from an independent connection pool immediately
        // after it observes `active`. This models another backend process
        // whose provider GET began before the stop but finishes afterward.
        let after_read = std::sync::Arc::new(tokio::sync::Notify::new());
        let resume = std::sync::Arc::new(tokio::sync::Notify::new());
        let mut stale_store = Store::connect(&database_url).await.unwrap();
        stale_store.usage_write_test_hook =
            Some(UsageWriteTestHook { after_read: after_read.clone(), resume: resume.clone() });
        let stale_active = UsageSample {
            browser_millis: 180_000,
            proxy_bytes_unclassified: Some(800_000),
            browser_cost: "0.24".into(),
            proxy_cost: "0.12".into(),
            sampled_at: at(2, 13),
            ..active
        };
        let stale_session_id = session.id.clone();
        let stale_task =
            tokio::spawn(async move { stale_store.record_usage("org-1", &stale_session_id, &stale_active).await });
        tokio::time::timeout(Duration::from_secs(1), after_read.notified()).await.unwrap();

        store
            .begin_end_session("org-1", &owner, &session.id, &SessionEnd { note: "done".into() }, &secrets())
            .await
            .unwrap();
        let refunded = store.record_terminal_usage("org-1", &session.id, &terminal).await.unwrap();
        assert_eq!(refunded.browser_seconds, 120);
        assert_eq!(refunded.proxy_bytes_unclassified, 600_000);
        assert_eq!(refunded.cost.browser.micros, 30_000);
        assert_eq!(refunded.cost.proxy_unclassified.micros, 100_000);

        resume.notify_one();
        match tokio::time::timeout(Duration::from_secs(1), stale_task).await.unwrap().unwrap() {
            Ok(retained) => assert_eq!(retained, refunded),
            // SQLite may reject upgrading the deliberately stale WAL snapshot.
            // That is also safe: the active refresh path is best-effort and no
            // final values were overwritten.
            Err(StoreError::Database(_)) => {}
            Err(error) => panic!("unexpected stale sampler failure: {error}"),
        }
        assert_eq!(store.usage("org-1", &owner, &session.id).await.unwrap(), refunded);

        store.finalize_end_session("org-1", &session.id, at(2, 13)).await.unwrap();
        assert_eq!(store.usage("org-1", &owner, &session.id).await.unwrap(), refunded);
    }

    /// Test group: accounting preserves measured traffic even when the provider
    /// unexpectedly meters an incognito session created with proxy disabled.
    #[tokio::test]
    async fn incognito_usage_preserves_unexpected_provider_proxy_measurements() {
        let store = Store::in_memory().await.unwrap();
        let owner = silicon("owner-1");
        let request = SessionCreate::incognito("private", "research", SessionTtl::Minutes15);
        let reserved = store.reserve_session("org-1", &owner, &request, at(2, 10)).await.unwrap();
        let session = store
            .activate_session("org-1", &reserved.id, &provider("incognito-usage"), &secrets(), at(2, 10))
            .await
            .unwrap();
        let sample = UsageSample {
            browser_millis: 1_000,
            proxy_bytes_in: None,
            proxy_bytes_out: None,
            proxy_bytes_unclassified: Some(1_160_191),
            browser_cost: "0".into(),
            proxy_cost: "0.00022659972310066222289062500".into(),
            currency: "USD".into(),
            sampled_at: at(2, 10),
        };
        let usage = store.record_usage("org-1", &session.id, &sample).await.unwrap();
        assert_eq!((usage.proxy_bytes_in, usage.proxy_bytes_out), (0, 0));
        assert_eq!(usage.proxy_bytes_unclassified, 1_160_191);
        assert_eq!(usage.cost.proxy_unclassified.micros, 227);
        assert_eq!(usage.cost.total.micros, 227);
    }
}

impl Store {
    pub async fn session_logs(
        &self,
        org_id: &str,
        viewer: &Identity,
        session_id: &str,
        date: Option<NaiveDate>,
        secrets: &SecretBox,
    ) -> StoreResult<Vec<SessionLog>> {
        let session = self.session(org_id, viewer, session_id).await?;
        let started_by_kind: String =
            sqlx::query_scalar("SELECT started_by_kind FROM sessions WHERE org_id = ? AND id = ?")
                .bind(org_id)
                .bind(session_id)
                .fetch_one(&self.pool)
                .await?;
        if started_by_kind == "carbon" {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT sequence, actor_id, command_enc, started_at, exit_code \
             FROM commands WHERE session_id = ? ORDER BY sequence",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        let mut entries = Vec::new();
        for row in rows {
            let at = parse_timestamp(row.try_get("started_at")?, "command.started_at")?;
            if date.is_some_and(|date| at.date_naive() != date) {
                continue;
            }
            let sequence: i64 = row.try_get("sequence")?;
            let encrypted: String = row.try_get("command_enc")?;
            let actor_id: String = row.try_get("actor_id")?;
            entries.push(SessionLog {
                sequence: nonnegative(sequence, "command.sequence")?,
                at,
                actor_id: actor_id.clone(),
                command: secrets
                    .open_for(&command_secret_context(org_id, session_id, sequence, &actor_id), &encrypted)
                    .map_err(StoreError::Crypto)?,
                exit_code: row.try_get("exit_code")?,
            });
        }
        debug_assert_eq!(session.id, session_id);
        Ok(entries)
    }

    /// Keep an ended visual recording pending while its provider URL is still
    /// being materialized. The outbox payload contains only local identifiers;
    /// signed provider URLs are never written there in plaintext.
    pub async fn queue_recording_source_resolution(
        &self,
        org_id: &str,
        session_id: &str,
        duration_ms: u64,
        now: DateTime<Utc>,
    ) -> StoreResult<Recording> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let result = sqlx::query(
            "UPDATE recordings SET status = 'pending', duration_ms = ?, size_bytes = COALESCE(size_bytes, 0) \
             WHERE session_id = ? AND status IN ('recording', 'pending') \
             AND EXISTS (SELECT 1 FROM sessions WHERE sessions.id = recordings.session_id \
             AND sessions.org_id = ? AND sessions.status IN ('ending', 'ended', 'expired', 'failed') \
             AND sessions.provider_session_id IS NOT NULL AND sessions.provider_recording_url_enc IS NULL)",
        )
        .bind(to_i64(duration_ms, "recording.duration_ms")?)
        .bind(session_id)
        .bind(org_id)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            return Err(not_found("recording", session_id));
        }
        let payload = serde_json::to_string(&serde_json::json!({
            "org_id": org_id,
            "session_id": session_id,
        }))
        .map_err(corrupt_json)?;
        let first_attempt = now
            + ChronoDuration::from_std(RECORDING_SOURCE_INITIAL_DELAY)
                .map_err(|_| StoreError::Invalid("recording retry delay is too large".into()))?;
        sqlx::query(
            "INSERT OR IGNORE INTO outbox \
             (id, event_type, payload_json, attempts, next_attempt_at, created_at) \
             VALUES (?, 'recording.resolve', ?, 0, ?, ?)",
        )
        .bind(format!("recording.resolve:{session_id}"))
        .bind(payload)
        .bind(timestamp(first_attempt))
        .bind(timestamp(now))
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.recording_unchecked(org_id, session_id, None).await
    }

    /// Atomically lease due recording URL lookups. Incrementing `attempts`
    /// while moving `next_attempt_at` forward makes a crashed worker retryable
    /// without allowing two service processes to own the same lookup.
    pub async fn claim_recording_source_resolutions(
        &self,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
        limit: u32,
    ) -> StoreResult<Vec<RecordingSourceClaim>> {
        if limit == 0 || limit > 100 {
            return Err(StoreError::Invalid("recording resolution claim limit must be between 1 and 100".into()));
        }
        if lease_until <= now {
            return Err(StoreError::Invalid("recording resolution lease must end in the future".into()));
        }
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let rows = sqlx::query(
            "UPDATE outbox SET attempts = attempts + 1, next_attempt_at = ? WHERE id IN ( \
               SELECT o.id FROM outbox o \
               JOIN sessions s ON o.id = 'recording.resolve:' || s.id \
               JOIN recordings r ON r.session_id = s.id \
               WHERE o.event_type = 'recording.resolve' AND o.done_at IS NULL AND o.next_attempt_at <= ? \
               AND s.provider_session_id IS NOT NULL AND s.provider_recording_url_enc IS NULL \
               AND s.status IN ('ended', 'expired', 'failed') AND r.status IN ('recording', 'pending', 'failed') \
               ORDER BY o.next_attempt_at, o.created_at, o.id LIMIT ? \
             ) RETURNING id, attempts",
        )
        .bind(timestamp(lease_until))
        .bind(timestamp(now))
        .bind(i64::from(limit))
        .fetch_all(&mut *transaction)
        .await?;

        let mut claims = Vec::with_capacity(rows.len());
        for row in rows {
            let event_id: String = row.try_get("id")?;
            let attempt = u32::try_from(row.try_get::<i64, _>("attempts")?)
                .map_err(|_| corrupt("outbox", "recording resolution attempt is outside the supported range"))?;
            let details = sqlx::query(
                "SELECT s.org_id, s.id AS session_id, s.provider_session_id \
                 FROM sessions s WHERE ? = 'recording.resolve:' || s.id",
            )
            .bind(&event_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| corrupt("outbox", "recording resolution has no matching session"))?;
            claims.push(RecordingSourceClaim {
                event_id,
                org_id: details.try_get("org_id")?,
                session_id: details.try_get("session_id")?,
                provider_session_id: details
                    .try_get::<Option<String>, _>("provider_session_id")?
                    .ok_or_else(|| corrupt("session", "recording resolution has no provider session id"))?,
                attempt,
            });
        }
        transaction.commit().await?;
        Ok(claims)
    }

    /// Move a still-unresolved lookup to its next due time. The attempt CAS
    /// makes a response from an expired lease harmless after another worker
    /// has already reclaimed the event.
    pub async fn reschedule_recording_source_resolution(
        &self,
        claim: &RecordingSourceClaim,
        next_attempt_at: DateTime<Utc>,
    ) -> StoreResult<bool> {
        let result = sqlx::query(
            "UPDATE outbox SET next_attempt_at = ? WHERE id = ? AND event_type = 'recording.resolve' \
             AND attempts = ? AND done_at IS NULL",
        )
        .bind(timestamp(next_attempt_at))
        .bind(&claim.event_id)
        .bind(i64::from(claim.attempt))
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() != 0)
    }

    /// Persist a materialized signed URL and atomically turn the resolution
    /// intent into the deferred Briefcase storage intent.
    pub async fn complete_recording_source_resolution(
        &self,
        claim: &RecordingSourceClaim,
        recording_url: &str,
        secrets: &SecretBox,
        now: DateTime<Utc>,
    ) -> StoreResult<bool> {
        required(recording_url, "recording_url")?;
        let encrypted = secrets
            .seal_for(
                &session_secret_context(&claim.org_id, &claim.session_id, "provider-recording-url"),
                recording_url,
            )
            .map_err(StoreError::Crypto)?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let owned = sqlx::query(
            "UPDATE outbox SET done_at = ? WHERE id = ? AND event_type = 'recording.resolve' \
             AND attempts = ? AND done_at IS NULL",
        )
        .bind(timestamp(now))
        .bind(&claim.event_id)
        .bind(i64::from(claim.attempt))
        .execute(&mut *transaction)
        .await?;
        if owned.rows_affected() == 0 {
            transaction.rollback().await?;
            return Ok(false);
        }

        let status: String = sqlx::query_scalar(
            "SELECT r.status FROM recordings r JOIN sessions s ON s.id = r.session_id \
             WHERE s.org_id = ? AND s.id = ? AND s.provider_session_id = ? \
             AND s.status IN ('ended', 'expired', 'failed')",
        )
        .bind(&claim.org_id)
        .bind(&claim.session_id)
        .bind(&claim.provider_session_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| corrupt("outbox", "recording resolution target changed"))?;
        if status == "trashed" {
            transaction.commit().await?;
            return Ok(false);
        }
        if !matches!(status.as_str(), "recording" | "pending" | "failed") {
            return Err(corrupt("recording", format!("cannot resolve recording in {status} state")));
        }
        sqlx::query(
            "UPDATE sessions SET provider_recording_url_enc = ? \
             WHERE org_id = ? AND id = ? AND provider_session_id = ? AND provider_recording_url_enc IS NULL",
        )
        .bind(encrypted)
        .bind(&claim.org_id)
        .bind(&claim.session_id)
        .bind(&claim.provider_session_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE recordings SET status = 'pending' WHERE session_id = ? AND status IN ('recording', 'pending')",
        )
        .bind(&claim.session_id)
        .execute(&mut *transaction)
        .await?;
        let payload = serde_json::to_string(&serde_json::json!({
            "org_id": claim.org_id,
            "session_id": claim.session_id,
        }))
        .map_err(corrupt_json)?;
        sqlx::query(
            "INSERT OR IGNORE INTO outbox \
             (id, event_type, payload_json, attempts, next_attempt_at, created_at) \
             VALUES (?, 'recording.store', ?, 0, ?, ?)",
        )
        .bind(format!("recording.store:{}", claim.session_id))
        .bind(payload)
        .bind(timestamp(now))
        .bind(timestamp(now))
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(true)
    }

    /// Exhaust a resolution intent without manufacturing an uploadable
    /// recording. This is terminal and CAS-protected against a stale worker.
    pub async fn fail_recording_source_resolution(
        &self,
        claim: &RecordingSourceClaim,
        now: DateTime<Utc>,
    ) -> StoreResult<bool> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let owned = sqlx::query(
            "UPDATE outbox SET done_at = ? WHERE id = ? AND event_type = 'recording.resolve' \
             AND attempts = ? AND done_at IS NULL",
        )
        .bind(timestamp(now))
        .bind(&claim.event_id)
        .bind(i64::from(claim.attempt))
        .execute(&mut *transaction)
        .await?;
        if owned.rows_affected() == 0 {
            transaction.rollback().await?;
            return Ok(false);
        }
        sqlx::query(
            "UPDATE recordings SET status = 'failed', size_bytes = COALESCE(size_bytes, 0) \
             WHERE session_id = ? AND status IN ('recording', 'pending') \
             AND EXISTS (SELECT 1 FROM sessions WHERE sessions.id = recordings.session_id \
             AND sessions.org_id = ? AND sessions.provider_session_id = ? \
             AND sessions.status IN ('ended', 'expired', 'failed'))",
        )
        .bind(&claim.session_id)
        .bind(&claim.org_id)
        .bind(&claim.provider_session_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(true)
    }

    pub async fn mark_recording_pending(
        &self,
        org_id: &str,
        session_id: &str,
        duration_ms: u64,
        size_bytes: u64,
    ) -> StoreResult<Recording> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let result = sqlx::query(
            "UPDATE recordings SET status = 'pending', duration_ms = ?, size_bytes = ? \
             WHERE session_id = ? AND status IN ('recording', 'pending') \
             AND EXISTS (SELECT 1 FROM sessions WHERE sessions.id = recordings.session_id \
             AND sessions.org_id = ? AND sessions.status IN ('ending', 'ended', 'expired', 'failed') \
             AND sessions.provider_recording_url_enc IS NOT NULL)",
        )
        .bind(to_i64(duration_ms, "recording.duration_ms")?)
        .bind(to_i64(size_bytes, "recording.size_bytes")?)
        .bind(session_id)
        .bind(org_id)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            return Err(not_found("recording", session_id));
        }
        let payload = serde_json::to_string(&serde_json::json!({
            "org_id": org_id,
            "session_id": session_id,
        }))
        .map_err(corrupt_json)?;
        sqlx::query(
            "INSERT OR IGNORE INTO outbox \
             (id, event_type, payload_json, attempts, next_attempt_at, created_at) \
             VALUES (?, 'recording.store', ?, 0, ?, ?)",
        )
        .bind(format!("recording.store:{session_id}"))
        .bind(payload)
        .bind(timestamp(Utc::now()))
        .bind(timestamp(Utc::now()))
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.recording_unchecked(org_id, session_id, None).await
    }

    /// Terminalize a visual recording only when it is known to be
    /// irrecoverable (for example, no provider browser was ever activated).
    pub async fn mark_recording_failed(
        &self,
        org_id: &str,
        session_id: &str,
        duration_ms: u64,
    ) -> StoreResult<Recording> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let result = sqlx::query(
            "UPDATE recordings SET status = 'failed', duration_ms = ?, size_bytes = COALESCE(size_bytes, 0) \
             WHERE session_id = ? AND status IN ('recording', 'pending') \
             AND EXISTS (SELECT 1 FROM sessions WHERE sessions.id = recordings.session_id \
             AND sessions.org_id = ? AND sessions.status IN ('ending', 'ended', 'expired', 'failed'))",
        )
        .bind(to_i64(duration_ms, "recording.duration_ms")?)
        .bind(session_id)
        .bind(org_id)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            return Err(not_found("recording", session_id));
        }
        sqlx::query("UPDATE outbox SET done_at = COALESCE(done_at, ?) WHERE id = ?")
            .bind(timestamp(Utc::now()))
            .bind(format!("recording.store:{session_id}"))
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE outbox SET done_at = COALESCE(done_at, ?) WHERE id = ?")
            .bind(timestamp(Utc::now()))
            .bind(format!("recording.resolve:{session_id}"))
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        self.recording_unchecked(org_id, session_id, None).await
    }

    /// Persist the provider's post-stop recording source encrypted. The outbox
    /// carries only org/session references; the signed URL never enters JSON
    /// logs or a plaintext queue payload.
    pub async fn remember_provider_recording_url(
        &self,
        org_id: &str,
        session_id: &str,
        recording_url: &str,
        secrets: &SecretBox,
    ) -> StoreResult<()> {
        required(recording_url, "recording_url")?;
        let encrypted = secrets
            .seal_for(&session_secret_context(org_id, session_id, "provider-recording-url"), recording_url)
            .map_err(StoreError::Crypto)?;
        let result = sqlx::query("UPDATE sessions SET provider_recording_url_enc = ? WHERE org_id = ? AND id = ?")
            .bind(encrypted)
            .bind(org_id)
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(not_found("session", session_id));
        }
        Ok(())
    }

    pub async fn recording(
        &self,
        org_id: &str,
        viewer: &Identity,
        session_id: &str,
        secrets: &SecretBox,
    ) -> StoreResult<Recording> {
        self.session(org_id, viewer, session_id).await?;
        self.recording_unchecked(org_id, session_id, Some(secrets)).await
    }

    pub async fn recordings(
        &self,
        org_id: &str,
        viewer: &Identity,
        filter: Option<&RecordingFilter>,
        secrets: &SecretBox,
    ) -> StoreResult<Vec<Recording>> {
        let sessions = self.sessions(org_id, viewer).await?;
        let mut recordings = Vec::with_capacity(sessions.len());
        for session in sessions {
            let recording = self.recording_unchecked(org_id, &session.id, Some(secrets)).await?;
            if filter.is_none_or(|filter| filter.matches(&recording, viewer)) {
                recordings.push(recording);
            }
        }
        Ok(recordings)
    }

    pub async fn trash_recording(
        &self,
        org_id: &str,
        actor: &Identity,
        session_id: &str,
        now: DateTime<Utc>,
        secrets: &SecretBox,
    ) -> StoreResult<Recording> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let row = sqlx::query(
            "SELECT r.owner_id, r.status AS recording_status, s.status AS session_status \
             FROM recordings r JOIN sessions s ON s.id = r.session_id \
             WHERE s.org_id = ? AND r.session_id = ?",
        )
        .bind(org_id)
        .bind(session_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| not_found("recording", session_id))?;
        let owner_id: String = row.try_get("owner_id")?;
        if !actor.matches_principal(&owner_id) {
            return Err(forbidden("recording", session_id));
        }
        let recording_status: String = row.try_get("recording_status")?;
        let session_status: String = row.try_get("session_status")?;
        if matches!(session_status.as_str(), "starting" | "active" | "ending") {
            return Err(StoreError::SessionState { session_id: session_id.into(), status: session_status });
        }
        if !matches!(recording_status.as_str(), "pending" | "available" | "failed" | "trashed") {
            return Err(StoreError::SessionState { session_id: session_id.into(), status: recording_status });
        }
        sqlx::query(
            "UPDATE recordings SET status = 'trashed', trashed_at = COALESCE(trashed_at, ?), \
             purge_after = NULL WHERE session_id = ? AND status <> 'trashed'",
        )
        .bind(timestamp(now))
        .bind(session_id)
        .execute(&mut *transaction)
        .await?;
        // This is a local hide only. Briefcase does not expose OBO deletion;
        // retain its existing files and do not promise a remote purge deadline.
        sqlx::query("UPDATE outbox SET done_at = COALESCE(done_at, ?) WHERE id = ?")
            .bind(timestamp(now))
            .bind(format!("recording.store:{session_id}"))
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE outbox SET done_at = COALESCE(done_at, ?) WHERE id = ?")
            .bind(timestamp(now))
            .bind(format!("recording.resolve:{session_id}"))
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE outbox SET done_at = COALESCE(done_at, ?) WHERE id = ? AND event_type = 'recording.trash'")
            .bind(timestamp(now))
            .bind(format!("recording.trash:{session_id}"))
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        self.recording_unchecked(org_id, session_id, Some(secrets)).await
    }

    async fn recording_unchecked(
        &self,
        org_id: &str,
        session_id: &str,
        secrets: Option<&SecretBox>,
    ) -> StoreResult<Recording> {
        let row = sqlx::query(
            "SELECT r.*, s.profile_id, s.name, s.description, s.started_at FROM recordings r \
             JOIN sessions s ON s.id = r.session_id WHERE s.org_id = ? AND r.session_id = ?",
        )
        .bind(org_id)
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| not_found("recording", session_id))?;
        let participant_ids: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT actor_id FROM session_participants WHERE session_id = ? ORDER BY first_seen_at, actor_id",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        let profile_id: Option<String> = row.try_get("profile_id")?;
        let status: String = row.try_get("status")?;
        let link = if status == "trashed" {
            None
        } else {
            row.try_get::<Option<String>, _>("briefcase_url_enc")?
                .map(|encrypted| {
                    secrets
                        .ok_or_else(|| corrupt("recording", "encrypted Briefcase link requires SecretBox"))?
                        .open_for(&session_secret_context(org_id, session_id, "briefcase-url"), &encrypted)
                        .map_err(StoreError::Crypto)
                })
                .transpose()?
        };
        let duration_ms: Option<i64> = row.try_get("duration_ms")?;
        let size_bytes: Option<i64> = row.try_get("size_bytes")?;
        let log = sqlx::query("SELECT artifact_path, receipt_url_enc FROM recording_artifacts WHERE session_id = ? AND kind = 'commands' AND state = 'complete'")
            .bind(session_id).fetch_optional(&self.pool).await?;
        let (command_log_path, command_log_link) = if status == "trashed" {
            (None, None)
        } else if let Some(log) = log {
            let path: Option<String> = log.try_get("artifact_path")?;
            let link = log
                .try_get::<Option<String>, _>("receipt_url_enc")?
                .map(|encrypted| {
                    secrets
                        .ok_or_else(|| corrupt("recording", "encrypted log link requires SecretBox"))?
                        .open_for(&format!("recording-artifact:{org_id}:{session_id}:commands"), &encrypted)
                        .map_err(StoreError::Crypto)
                })
                .transpose()?;
            (path, link)
        } else {
            (None, None)
        };
        let delivery_error: Option<String> = if status == "trashed" {
            None
        } else {
            sqlx::query_scalar("SELECT last_error FROM recording_artifacts WHERE session_id = ? AND last_error IS NOT NULL ORDER BY CASE state WHEN 'failed' THEN 0 ELSE 1 END, kind LIMIT 1")
                .bind(session_id).fetch_optional(&self.pool).await?.flatten()
        };
        Ok(Recording {
            session_id: session_id.into(),
            incognito: profile_id.is_none(),
            profile_id,
            session_name: row.try_get("name")?,
            session_description: row.try_get("description")?,
            owner_id: row.try_get("owner_id")?,
            participant_ids,
            // The old placeholder path was never evidence of a Briefcase write.
            // Mask it for legacy pending rows too, without rewriting real receipts.
            briefcase_path: if row.try_get::<Option<String>, _>("briefcase_url_enc")?.is_some() {
                row.try_get("artifact_path")?
            } else {
                String::new()
            },
            briefcase_link: link,
            command_log_path,
            command_log_link,
            delivery_error,
            duration_seconds: nonnegative(duration_ms.unwrap_or(0), "recording.duration_ms")? / 1_000,
            size_bytes: nonnegative(size_bytes.unwrap_or(0), "recording.size_bytes")?,
            status: match status.as_str() {
                "recording" | "pending" => RecordingStatus::Pending,
                "available" => RecordingStatus::Available,
                "trashed" => RecordingStatus::Trashed,
                "failed" => RecordingStatus::Failed,
                value => return Err(corrupt("recording", format!("unknown status {value}"))),
            },
            created_at: parse_timestamp(row.try_get("started_at")?, "recording.created_at")?,
            trashed_at: optional_timestamp(row.try_get::<Option<&str>, _>("trashed_at")?, "recording.trashed_at")?,
            purge_at: optional_timestamp(row.try_get::<Option<&str>, _>("purge_after")?, "recording.purge_after")?,
        })
    }

    /// Persist a cost-so-far snapshot while the local session is still active.
    ///
    /// A provider read may have started before a concurrent end operation and
    /// complete after its terminal bill was stored. Rechecking the internal
    /// lifecycle in the write transaction makes that stale active response a
    /// no-op instead of allowing it to overwrite the refunded final cost.
    pub async fn record_usage(&self, org_id: &str, session_id: &str, sample: &UsageSample) -> StoreResult<Usage> {
        self.record_usage_snapshot(org_id, session_id, sample, false).await
    }

    /// Persist the authoritative snapshot returned after the provider has
    /// confirmed a stop. Browser Use charges the configured browser timeout up
    /// front and refunds unused time at this boundary, so terminal costs may be
    /// lower than an earlier active snapshot. Duration and traffic counters
    /// remain monotonic even if the provider's clocks round differently.
    pub async fn record_terminal_usage(
        &self,
        org_id: &str,
        session_id: &str,
        sample: &UsageSample,
    ) -> StoreResult<Usage> {
        self.record_usage_snapshot(org_id, session_id, sample, true).await
    }

    async fn record_usage_snapshot(
        &self,
        org_id: &str,
        session_id: &str,
        sample: &UsageSample,
        terminal: bool,
    ) -> StoreResult<Usage> {
        let browser_millis = to_i64(sample.browser_millis, "usage.browser_millis")?;
        let proxy_bytes_in = optional_i64(sample.proxy_bytes_in, "usage.proxy_bytes_in")?;
        let proxy_bytes_out = optional_i64(sample.proxy_bytes_out, "usage.proxy_bytes_out")?;
        let proxy_bytes_unclassified = optional_i64(sample.proxy_bytes_unclassified, "usage.proxy_bytes_unclassified")?;
        parse_cost_micros(&sample.browser_cost)?;
        parse_cost_micros(&sample.proxy_cost)?;
        validate_currency(&sample.currency)?;

        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT s.profile_id, s.status, u.browser_millis, u.proxy_bytes_in, u.proxy_bytes_out, u.proxy_bytes_unclassified, \
             u.browser_cost, u.proxy_cost, u.currency \
             FROM sessions s JOIN usage u ON u.session_id = s.id WHERE s.org_id = ? AND s.id = ?",
        )
        .bind(org_id)
        .bind(session_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| not_found("usage", session_id))?;
        let session_status: String = row.try_get("status")?;
        #[cfg(test)]
        if !terminal && let Some(hook) = &self.usage_write_test_hook {
            hook.after_read.notify_one();
            hook.resume.notified().await;
        }
        if terminal {
            if matches!(session_status.as_str(), "ended" | "expired" | "failed") {
                transaction.rollback().await?;
                return self.usage_unchecked(session_id).await;
            }
            if !matches!(session_status.as_str(), "starting" | "ending") {
                return Err(StoreError::SessionState { session_id: session_id.into(), status: session_status });
            }
        } else if session_status != "active" {
            transaction.rollback().await?;
            return self.usage_unchecked(session_id).await;
        }
        let profile_id: Option<String> = row.try_get("profile_id")?;
        if profile_id.is_some()
            && sample.proxy_bytes_in.is_none()
            && sample.proxy_bytes_out.is_none()
            && sample.proxy_bytes_unclassified.is_none()
        {
            return Err(StoreError::Invalid("profile sessions must report proxy counters".into()));
        }
        let current_browser: i64 = row.try_get("browser_millis")?;
        let current_in: Option<i64> = row.try_get("proxy_bytes_in")?;
        let current_out: Option<i64> = row.try_get("proxy_bytes_out")?;
        let current_unclassified: Option<i64> = row.try_get("proxy_bytes_unclassified")?;
        let current_browser_cost = parse_cost_micros(row.try_get("browser_cost")?)?;
        let current_proxy_cost = parse_cost_micros(row.try_get("proxy_cost")?)?;
        let next_browser_cost = parse_cost_micros(&sample.browser_cost)?;
        let next_proxy_cost = parse_cost_micros(&sample.proxy_cost)?;
        let current_currency: String = row.try_get("currency")?;
        let counters_decreased = browser_millis < current_browser
            || proxy_bytes_in.zip(current_in).is_some_and(|(next, current)| next < current)
            || proxy_bytes_out.zip(current_out).is_some_and(|(next, current)| next < current)
            || proxy_bytes_unclassified.zip(current_unclassified).is_some_and(|(next, current)| next < current);
        let costs_decreased = next_browser_cost < current_browser_cost || next_proxy_cost < current_proxy_cost;
        if !terminal && (counters_decreased || costs_decreased) {
            return Err(StoreError::Invalid("usage counters cannot decrease".into()));
        }
        if !current_currency.eq_ignore_ascii_case(&sample.currency) {
            return Err(StoreError::Invalid("usage currency cannot change within a session".into()));
        }
        let browser_millis = if terminal { browser_millis.max(current_browser) } else { browser_millis };
        let proxy_bytes_in = monotonic_terminal_counter(proxy_bytes_in, current_in, terminal);
        let proxy_bytes_out = monotonic_terminal_counter(proxy_bytes_out, current_out, terminal);
        let proxy_bytes_unclassified =
            monotonic_terminal_counter(proxy_bytes_unclassified, current_unclassified, terminal);
        let (allowed_status_a, allowed_status_b) = if terminal { ("starting", "ending") } else { ("active", "active") };
        let result = sqlx::query(
            "UPDATE usage SET browser_millis = ?, proxy_bytes_in = COALESCE(?, proxy_bytes_in), \
             proxy_bytes_out = COALESCE(?, proxy_bytes_out), \
             proxy_bytes_unclassified = COALESCE(?, proxy_bytes_unclassified), \
             browser_cost = ?, proxy_cost = ?, currency = ?, sampled_at = ? \
             WHERE session_id = ? AND EXISTS ( \
               SELECT 1 FROM sessions s WHERE s.id = usage.session_id AND s.org_id = ? AND s.status IN (?, ?) \
             )",
        )
        .bind(browser_millis)
        .bind(proxy_bytes_in)
        .bind(proxy_bytes_out)
        .bind(proxy_bytes_unclassified)
        .bind(&sample.browser_cost)
        .bind(&sample.proxy_cost)
        .bind(sample.currency.to_ascii_uppercase())
        .bind(timestamp(sample.sampled_at))
        .bind(session_id)
        .bind(org_id)
        .bind(allowed_status_a)
        .bind(allowed_status_b)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            transaction.rollback().await?;
            let status: Option<String> = sqlx::query_scalar("SELECT status FROM sessions WHERE org_id = ? AND id = ?")
                .bind(org_id)
                .bind(session_id)
                .fetch_optional(&self.pool)
                .await?;
            return match status.as_deref() {
                Some(status) if !terminal || matches!(status, "ended" | "expired" | "failed") => {
                    self.usage_unchecked(session_id).await
                }
                Some(status) => Err(StoreError::SessionState { session_id: session_id.into(), status: status.into() }),
                None => Err(not_found("session", session_id)),
            };
        }
        transaction.commit().await?;
        self.usage_unchecked(session_id).await
    }

    pub async fn usage(&self, org_id: &str, viewer: &Identity, session_id: &str) -> StoreResult<Usage> {
        self.session(org_id, viewer, session_id).await?;
        self.usage_unchecked(session_id).await
    }

    pub async fn usage_list(
        &self,
        org_id: &str,
        viewer: &Identity,
        filter: Option<&UsageFilter>,
    ) -> StoreResult<Vec<Usage>> {
        let sessions = self.visible_sessions_with_usage(org_id, viewer).await?;
        let mut result = Vec::with_capacity(sessions.len());
        for (_, usage) in sessions {
            if filter.is_none_or(|filter| filter.matches(&usage)) {
                result.push(usage);
            }
        }
        Ok(result)
    }

    pub async fn usage_total(
        &self,
        org_id: &str,
        viewer: &Identity,
        filter: Option<&UsageFilter>,
    ) -> StoreResult<UsageTotal> {
        // This is deliberately a visible-session total, not an assertion that
        // the caller has organization-wide billing visibility. IAM does not
        // currently project such a permission into the service.
        usage_total(&self.usage_list(org_id, viewer, filter).await?)
    }

    /// Aggregate every session in an organization, including sessions hidden
    /// from the requesting identity by profile/session ACLs.
    ///
    /// This deliberately has no `viewer` parameter: callers must establish an
    /// organization-wide authorization policy before invoking it. The HTTP
    /// boundary currently permits only an aggregate response to an active IAM
    /// member of this exact organization; per-session reads remain ACL-bound.
    pub async fn org_usage_total(&self, org_id: &str, filter: Option<&UsageFilter>) -> StoreResult<UsageTotal> {
        safe_id(org_id, "org_id")?;
        let session_ids: Vec<String> = sqlx::query_scalar(
            "SELECT s.id FROM sessions s JOIN usage u ON u.session_id = s.id \
             WHERE s.org_id = ? ORDER BY s.started_at, s.id",
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;
        let mut rows = Vec::with_capacity(session_ids.len());
        for session_id in session_ids {
            let usage = self.usage_unchecked(&session_id).await?;
            if filter.is_none_or(|filter| filter.matches(&usage)) {
                rows.push(usage);
            }
        }
        usage_total(&rows)
    }

    /// Privacy-minimal discovery audit. Queries, URLs and result bodies are
    /// deliberately excluded; operational accounting retains only purpose and
    /// item count.
    pub async fn record_discovery(
        &self,
        org_id: &str,
        actor_id: &str,
        kind: &str,
        purpose: &str,
        item_count: usize,
        now: DateTime<Utc>,
    ) -> StoreResult<()> {
        safe_id(org_id, "org_id")?;
        safe_id(actor_id, "actor_id")?;
        if !matches!(kind, "search" | "fetch") {
            return Err(StoreError::Invalid("discovery kind must be search or fetch".into()));
        }
        required(purpose, "purpose")?;
        sqlx::query(
            "INSERT INTO discovery_log (id, org_id, actor_id, kind, purpose, item_count, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(org_id)
        .bind(actor_id)
        .bind(kind)
        .bind(purpose.trim())
        .bind(i64::try_from(item_count).map_err(|_| StoreError::Invalid("item count is too large".into()))?)
        .bind(timestamp(now))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn usage_unchecked(&self, session_id: &str) -> StoreResult<Usage> {
        let row = sqlx::query(
            "SELECT u.*, s.started_at FROM usage u JOIN sessions s ON s.id = u.session_id WHERE u.session_id = ?",
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| not_found("usage", session_id))?;
        let principal_ids = sqlx::query_scalar(
            "SELECT DISTINCT actor_id FROM session_participants WHERE session_id = ? ORDER BY first_seen_at, actor_id",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        map_usage(&row, principal_ids)
    }
}
