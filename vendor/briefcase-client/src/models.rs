//! The shapes Briefcase sends and accepts.
//!
//! These mirror the published contract exactly. Anything Briefcase treats as
//! opaque — cursors, proofs, idempotency keys — stays opaque here too.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use crate::config::EnvironmentKey;

/// Kind of IAM principal.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorType {
    /// A human account.
    Carbon,
    /// An AI-agent account.
    Silicon,
    /// A registered application.
    Application,
}

impl ActorType {
    /// Returns the wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Carbon => "carbon",
            Self::Silicon => "silicon",
            Self::Application => "application",
        }
    }
}

impl std::fmt::Display for ActorType {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A reference to one Carbon or Silicon.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct ActorRef {
    /// Principal kind.
    #[serde(rename = "type")]
    pub actor_type: ActorType,
    /// IAM principal identifier.
    pub id: String,
}

impl ActorRef {
    /// References a Carbon by identifier.
    #[must_use]
    pub fn carbon(id: impl Into<String>) -> Self {
        Self {
            actor_type: ActorType::Carbon,
            id: id.into(),
        }
    }

    /// References a Silicon by identifier.
    #[must_use]
    pub fn silicon(id: impl Into<String>) -> Self {
        Self {
            actor_type: ActorType::Silicon,
            id: id.into(),
        }
    }
}

impl std::fmt::Display for ActorRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.id)
    }
}

/// Whether an entry stores bytes or contains other entries.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryType {
    /// A file.
    File,
    /// A folder.
    Folder,
}

/// The visibility boundary an entry inherits.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RootType {
    /// Readable by every current organization member.
    Public,
    /// Readable through ownership or an explicit grant only.
    Private,
    /// Readable by everyone carrying one IAM tag.
    Tag,
}

/// How much of an entry the caller can see.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryVisibility {
    /// Every field is populated.
    Full,
    /// A folder reachable only because something inside it was shared.
    Traversal,
}

/// The renderer a client should open for a file.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderKind {
    /// Still image.
    Image,
    /// Moving image.
    Video,
    /// Paginated or prose document.
    Document,
    /// Tabular data.
    Spreadsheet,
    /// Slide deck.
    Presentation,
    /// Sound.
    Audio,
    /// Container listed without extraction.
    Archive,
    /// Source or structured data.
    Code,
    /// No renderer applies.
    Unsupported,
}

/// What the caller may do with an entry.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectiveAccess {
    /// Read metadata and content.
    Read,
    /// Add content that does not exist yet.
    Write,
    /// Change content that already exists.
    Update,
    /// Move the entry to the recoverable bin.
    Delete,
    /// Grant and revoke access.
    ManagePermissions,
}

impl EffectiveAccess {
    /// Returns the wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::ManagePermissions => "manage_permissions",
        }
    }
}

impl std::fmt::Display for EffectiveAccess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One right a permission grant conveys.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessRight {
    /// View and download.
    Read,
    /// Add new content.
    Write,
    /// Change what already exists.
    Update,
    /// Move to the recoverable bin.
    Delete,
}

impl AccessRight {
    /// Returns the wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }

    /// Parses a right from its wire spelling.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            "update" => Some(Self::Update),
            "delete" => Some(Self::Delete),
            _ => None,
        }
    }
}

impl std::fmt::Display for AccessRight {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A file or folder.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Entry {
    /// Stable identifier.
    pub id: Uuid,
    /// Organization the entry belongs to.
    pub org_id: String,
    /// File or folder.
    #[serde(rename = "type")]
    pub entry_type: EntryType,
    /// How much of this entry is disclosed.
    pub visibility: EntryVisibility,
    /// Display name within the parent.
    pub name: String,
    /// Organization-relative path, without a leading separator.
    pub path: String,
    /// Parent folder, absent at the organization base.
    pub parent_id: Option<Uuid>,
    /// Inherited visibility boundary.
    pub root_type: RootType,
    /// IAM tag backing a tag boundary.
    pub tag: Option<String>,
    /// Media type of a file.
    pub content_type: Option<String>,
    /// Size of a file in bytes.
    pub size: Option<u64>,
    /// Renderer a client should open for a file.
    pub render: Option<RenderKind>,
    /// Clean permanent URL that shows the folder structure.
    pub permanent_url: Url,
    /// Authenticated sandboxed content URL for a file.
    pub content_url: Option<Url>,
    /// Authenticated attachment URL for a file.
    pub download_url: Option<Url>,
    /// Owner, absent for a traversal folder and for a reserved container.
    pub owner: Option<ActorRef>,
    /// Application that originally created the entry.
    pub origin_app_id: Option<String>,
    /// What the caller may do here.
    pub effective_access: Vec<EffectiveAccess>,
    /// Creation time.
    #[serde(with = "time::serde::rfc3339::option")]
    pub created_at: Option<OffsetDateTime>,
    /// Last change time.
    #[serde(with = "time::serde::rfc3339::option")]
    pub updated_at: Option<OffsetDateTime>,
    /// Time the entry was moved to the bin.
    #[serde(with = "time::serde::rfc3339::option")]
    pub deleted_at: Option<OffsetDateTime>,
    /// When a self-destructing file is permanently deleted; absent for a
    /// permanent file, every folder, and servers that predate self destruct.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub self_destruct_at: Option<OffsetDateTime>,
}

impl Entry {
    /// Returns whether the entry is a folder.
    #[must_use]
    pub const fn is_folder(&self) -> bool {
        matches!(self.entry_type, EntryType::Folder)
    }

    /// Returns whether the caller holds a right on this entry.
    #[must_use]
    pub fn allows(&self, access: EffectiveAccess) -> bool {
        self.effective_access.contains(&access)
    }
}

/// One page of entries.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EntryPage {
    /// Visible entries, newest first.
    pub items: Vec<Entry>,
    /// Cursor for the following page, absent at the end.
    pub next_cursor: Option<String>,
}

/// An explicit permission grant.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PermissionGrant {
    /// Grant identifier.
    pub id: Uuid,
    /// Member the grant is for.
    pub principal: ActorRef,
    /// Rights it conveys.
    pub access: Vec<AccessRight>,
    /// Whether it reaches descendants.
    pub inherit: bool,
    /// Who created it.
    pub granted_by: ActorRef,
    /// When it was created.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// When this expiring share ends; absent for a permanent grant.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
}

/// Explicit grants on one entry.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PermissionGrantPage {
    /// The grants.
    pub items: Vec<PermissionGrant>,
}

/// What the caller may do on one inspected target.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EffectivePermission {
    /// Resolved entry identifier.
    pub entry_id: Uuid,
    /// Resolved path.
    pub path: String,
    /// File or folder.
    #[serde(rename = "type")]
    pub entry_type: EntryType,
    /// Everything the caller may do there.
    pub effective_access: Vec<EffectiveAccess>,
}

/// The answer to "what may I do with these?".
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PermissionInspection {
    /// One item per readable target.
    pub items: Vec<EffectivePermission>,
    /// Identifiers with no readable entry.
    pub unresolved_entry_ids: Vec<Uuid>,
    /// Paths with no readable entry.
    pub unresolved_paths: Vec<String>,
}

/// What a notification is about.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    /// The recipient received access.
    AccessGranted,
    /// The recipient's access was revoked.
    AccessRevoked,
    /// A request awaits the recipient's decision.
    AccessRequested,
    /// The recipient's own request was decided.
    AccessRequestDecided,
}

/// How a request was decided.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationDecision {
    /// Approved.
    Approved,
    /// Denied.
    Denied,
}

/// The entry a notification refers to, as it was at that moment.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NotificationSubject {
    /// Entry identifier.
    pub entry_id: Uuid,
    /// Name at the time.
    pub name: String,
    /// Path at the time.
    pub path: String,
    /// File or folder.
    #[serde(rename = "type")]
    pub entry_type: EntryType,
    /// Permanent URL of the entry.
    pub permanent_url: Url,
}

/// One notification.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Notification {
    /// Notification identifier.
    pub id: Uuid,
    /// What happened.
    pub kind: NotificationKind,
    /// Whether it has been read.
    pub read: bool,
    /// Who acted.
    pub actor: Option<ActorRef>,
    /// What it refers to.
    pub subject: Option<NotificationSubject>,
    /// Rights involved.
    pub access: Option<Vec<AccessRight>>,
    /// Access request it belongs to.
    pub access_request_id: Option<Uuid>,
    /// Outcome of a decided request.
    pub decision: Option<NotificationDecision>,
    /// When it was written.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// The notification inbox and its badge count.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NotificationInbox {
    /// Twenty newest notifications, newest first.
    pub items: Vec<Notification>,
    /// Unread count for a badge.
    pub unread_count: u64,
}

/// One consumption figure against its limit, in bytes.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct UsageMeasure {
    /// Bytes consumed.
    pub used_bytes: u64,
    /// Bytes allowed.
    pub limit_bytes: u64,
    /// Bytes still available.
    pub remaining_bytes: u64,
}

/// The day's upload consumption and when it returns.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct DailyUsageMeasure {
    /// Bytes uploaded today.
    pub used_bytes: u64,
    /// Bytes allowed in one UTC day.
    pub limit_bytes: u64,
    /// Bytes still available today.
    pub remaining_bytes: u64,
    /// Next midnight UTC.
    #[serde(with = "time::serde::rfc3339")]
    pub resets_at: OffsetDateTime,
}

/// What an organization is consuming.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct OrganizationUsage {
    /// Storage consumption and ceiling.
    pub storage: UsageMeasure,
    /// Upload volume within the current UTC day.
    pub daily_uploads: DailyUsageMeasure,
}

/// One search hit.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SearchResult {
    /// The entry found.
    pub entry: Entry,
    /// Ranking score within this response.
    pub score: f64,
    /// Whether the filename matched.
    pub filename_match: bool,
    /// Number of content matches.
    pub content_hits: u32,
    /// Excerpts, when extraction supports them.
    pub snippets: Vec<String>,
}

/// Search results.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SearchPage {
    /// Ranked hits.
    pub items: Vec<SearchResult>,
}

/// One recorded action in an entry's history.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ActivityEvent {
    /// Stable action name, such as `entry.file_created.v1`.
    pub action: String,
    /// Who performed it.
    pub actor: ActorRef,
    /// Application that acted on their behalf.
    pub app_id: Option<String>,
    /// When it happened.
    #[serde(with = "time::serde::rfc3339")]
    pub occurred_at: OffsetDateTime,
}

/// An entry's retained history.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ActivityPage {
    /// Newest-first history, capped at a hundred records.
    pub items: Vec<ActivityEvent>,
}

/// One retained version of a file.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FileVersion {
    /// Version identifier.
    pub id: Uuid,
    /// Monotonic version number.
    pub number: u64,
    /// Size in bytes.
    pub size: u64,
    /// SHA-256 of the complete file bytes.
    pub sha256: Option<String>,
    /// How the immutable version was created.
    pub source: String,
    /// Who created it.
    pub created_by: ActorRef,
    /// When it was created.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// A file's retained versions.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FileVersionPage {
    /// Newest-first versions, up to 100 per page.
    pub items: Vec<FileVersion>,
    /// Continue without losing access to older retained versions.
    pub next_cursor: Option<String>,
}

/// Server-side encryption for an organization bucket.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EncryptionMode {
    /// S3-managed keys.
    SseS3,
    /// A customer-selected KMS key.
    SseKms,
}

/// An organization-owned S3 bucket to store its files in.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BucketConfiguration {
    /// Bucket name.
    pub bucket_name: String,
    /// AWS region.
    pub region: String,
    /// Cross-account role Briefcase assumes.
    pub role_arn: String,
    /// Organization-owned prefix.
    pub prefix: String,
    /// Expected AWS account.
    pub aws_account_id: String,
    /// Required encryption behavior.
    pub encryption_mode: EncryptionMode,
    /// KMS key, required when the mode is `sse_kms`.
    pub kms_key_arn: Option<String>,
}

/// Whether a storage configuration probe succeeded.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BucketConfigurationState {
    /// The probe succeeded and the configuration is active.
    Configured,
    /// The probe failed and the previous configuration remains.
    Failed,
}

/// The outcome of configuring organization storage.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BucketConfigurationStatus {
    /// Outcome.
    pub status: BucketConfigurationState,
    /// When the probe completed.
    #[serde(with = "time::serde::rfc3339")]
    pub tested_at: OffsetDateTime,
    /// Redacted failure category, when it failed.
    pub failure_reason: Option<String>,
}

/// Liveness or readiness of a deployment.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServiceStatus {
    /// `ok` for liveness, `ready` for readiness.
    pub status: String,
}

/// Lifecycle state of a disposable Briefcase testing environment.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TestingEnvironmentStatus {
    /// The key selects an active environment.
    Active,
    /// The key is disabled while the environment remains recoverable.
    Deleted,
}

/// One organization-owned Briefcase testing environment.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TestingEnvironment {
    /// Public selector safe to store in build metadata and pass to `--test`.
    pub id: Uuid,
    /// Owning IAM organization handle.
    pub org_id: String,
    /// Human-readable environment name.
    pub name: String,
    /// Optional purpose or run description.
    pub description: Option<String>,
    /// Current lifecycle state.
    pub status: TestingEnvironmentStatus,
    /// Public IAM environment paired with this Briefcase plane.
    pub iam_environment_id: Uuid,
    /// Canonical test-only IAM Application used by Briefcase in this plane.
    pub iam_app_id: crate::ApplicationId,
    /// Public actor that created the environment.
    pub created_by: TestingEnvironmentCreator,
    /// Increments whenever the Briefcase root key rotates.
    pub key_generation: i64,
    /// Most recent root-key rotation.
    #[serde(with = "time::serde::rfc3339::option")]
    pub key_rotated_at: Option<OffsetDateTime>,
    /// Last accepted request within this environment.
    #[serde(with = "time::serde::rfc3339")]
    pub last_activity_at: OffsetDateTime,
    /// Last time all disposable contents were erased.
    #[serde(with = "time::serde::rfc3339::option")]
    pub cleaned_at: Option<OffsetDateTime>,
    /// Soft-deletion time.
    #[serde(with = "time::serde::rfc3339::option")]
    pub deleted_at: Option<OffsetDateTime>,
    /// Final purge deadline for a retired environment.
    #[serde(with = "time::serde::rfc3339::option")]
    pub purge_after: Option<OffsetDateTime>,
    /// Optimistic-concurrency version.
    pub version: i64,
    /// Creation time.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Last lifecycle change.
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// Public identity retained as a testing environment's creator.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TestingEnvironmentCreator {
    /// Carbon or Silicon.
    #[serde(rename = "type")]
    pub actor_type: ActorType,
    /// Public actor handle.
    pub id: String,
}

/// Page of organization testing environments.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TestingEnvironmentPage {
    /// Environments in stable newest-first order.
    pub items: Vec<TestingEnvironment>,
}

/// Environment returned together with its current one-time secret value.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TestingEnvironmentWithKey {
    /// Environment metadata.
    #[serde(flatten)]
    pub environment: TestingEnvironment,
    /// Current Briefcase root key.
    pub key: EnvironmentKey,
}

/// Audited retrieval of an environment's current root key.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TestingEnvironmentKey {
    /// Environment whose key this is.
    pub environment_id: Uuid,
    /// Current key generation.
    pub key_generation: i64,
    /// Most recent rotation, when it has rotated.
    #[serde(with = "time::serde::rfc3339::option")]
    pub key_rotated_at: Option<OffsetDateTime>,
    /// Current Briefcase root key.
    pub key: EnvironmentKey,
}

/// Limited description available to a caller holding only the root key.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TestingEnvironmentSelf {
    /// Public environment selector.
    pub id: Uuid,
    /// Environment name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Current root-key generation.
    pub key_generation: i64,
    /// Creation time.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// Result of erasing one test environment's disposable data.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TestingEnvironmentCleaning {
    /// Cleaned environment.
    pub environment_id: Uuid,
    /// Database records erased, excluding the retained control record.
    pub erased_rows: u64,
    /// Completion time.
    #[serde(with = "time::serde::rfc3339")]
    pub cleaned_at: OffsetDateTime,
}

/// Access and rotating refresh credentials returned after an IAM SLT exchange.
#[derive(Clone, Deserialize, Serialize)]
pub struct SessionTokens {
    /// Short-lived IAM access token used on Briefcase requests.
    pub access_token: String,
    /// Single-use rotating IAM refresh token.
    pub refresh_token: String,
    /// OAuth token type, normally `Bearer`.
    pub token_type: String,
    /// Access-token lifetime in seconds.
    pub expires_in: u64,
    /// Granted IAM scope catalogue.
    pub scope: String,
    /// Signed-in Carbon or Silicon.
    pub actor: SessionActor,
    /// Organization selected for this session.
    pub org_id: Option<String>,
    /// Active organizations explicitly selected by the user in IAM.
    /// Empty means reauthorisation is needed, never access to all memberships.
    #[serde(default)]
    pub organizations: Vec<String>,
}

/// IAM actor represented by a Briefcase Application session.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SessionActor {
    /// Stable IAM principal UUID.
    pub principal_id: Uuid,
    /// Carbon or Silicon.
    #[serde(rename = "type")]
    pub actor_type: ActorType,
    /// Public Carbon/Silicon identifier.
    pub public_id: String,
}

impl std::fmt::Debug for SessionTokens {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionTokens")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field("token_type", &self.token_type)
            .field("expires_in", &self.expires_in)
            .field("scope", &self.scope)
            .field("actor", &self.actor)
            .field("org_id", &self.org_id)
            .field("organizations", &self.organizations)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{AccessRight, ActorRef, ActorType, EffectiveAccess, Entry};

    #[test]
    fn an_actor_reads_as_kind_and_identifier() {
        let actor = ActorRef::carbon("c:cos");
        assert_eq!(actor.actor_type, ActorType::Carbon);
        assert_eq!(actor.to_string(), "c:cos");
    }

    #[test]
    fn rights_round_trip_through_their_wire_spelling() {
        for right in [
            AccessRight::Read,
            AccessRight::Write,
            AccessRight::Update,
            AccessRight::Delete,
        ] {
            assert_eq!(AccessRight::parse(right.as_str()), Some(right));
        }
        assert_eq!(AccessRight::parse(" READ "), Some(AccessRight::Read));
        assert_eq!(AccessRight::parse("manage_permissions"), None);
    }

    #[test]
    fn an_entry_answers_what_the_caller_may_do() {
        let entry: Entry = serde_json::from_str(
            r#"{
                "id": "01a067ce-7f19-7790-820a-0be6b3d4f828",
                "org_id": "tos",
                "type": "file",
                "visibility": "full",
                "name": "notes.md",
                "path": "private/cos:tos/notes.md",
                "parent_id": null,
                "root_type": "private",
                "tag": null,
                "content_type": "text/markdown",
                "size": 12,
                "render": "document",
                "permanent_url": "https://briefcase.example/org/tos/private/cos:tos/notes.md",
                "content_url": null,
                "download_url": null,
                "owner": {"type": "carbon", "id": "cos:tos"},
                "origin_app_id": null,
                "effective_access": ["read", "update"],
                "created_at": null,
                "updated_at": null,
                "deleted_at": null
            }"#,
        )
        .expect("the contract's entry shape must parse");

        assert!(!entry.is_folder());
        assert!(entry.allows(EffectiveAccess::Update));
        assert!(!entry.allows(EffectiveAccess::Delete));
    }
}

/// Public IAM application configuration for the selected Briefcase deployment.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct IamInfo {
    /// Canonical app ID for which IAM should mint a short-lived token.
    pub app_id: crate::ApplicationId,
    /// Briefcase test plane, or production when absent.
    pub test_environment_id: Option<Uuid>,
    /// Paired IAM test plane, or production when absent.
    pub iam_environment_id: Option<Uuid>,
}

/// Current authentication verified online by IAM; never contains credentials.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct LoginStatus {
    /// Whether IAM accepted the access token as active for this application.
    pub authenticated: bool,
    /// The authenticated Carbon or Silicon, absent when unauthenticated.
    pub actor: Option<LoginActor>,
    /// Currently granted organizations with active memberships.
    pub organizations: Vec<String>,
    /// Access-token expiry as a Unix timestamp, absent when unauthenticated.
    pub expires_at: Option<i64>,
}

/// Identity returned by login inspection, even without an organization grant.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LoginActor {
    /// Stable IAM principal UUID.
    pub principal_id: Uuid,
    /// Carbon or Silicon.
    #[serde(rename = "type")]
    pub actor_type: ActorType,
    /// Public identifier, if an active organization snapshot supplies it.
    pub public_id: Option<String>,
}
