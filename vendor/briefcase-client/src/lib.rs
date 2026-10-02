//! The official Rust client for [Silicon Briefcase], the organization-scoped
//! file service used by Carbons, Silicons, and IAM-authorized applications.
//!
//! # What this package is
//!
//! Everything Briefcase exposes to a client, and nothing it does internally.
//! The package holds no login session or API cache: a [`Config`] is passed in,
//! a [`Client`] comes out, and credentials remembered between runs belong to
//! the caller. API requests never query package registries or run Cargo.
//! Update this dependency through the consuming project and rebuild.
//! [`Config::with_auto_update`] remains a compatibility no-op.
//!
//! # Getting started
//!
//! ```no_run
//! use briefcase_client::{Client, Config, Destination, ListEntries, Upload};
//!
//! # async fn example() -> briefcase_client::Result<()> {
//! let client = Client::connect(
//!     Config::new("https://backend.briefcase.teamofsilicons.com/api/v1/", "tos")?
//!         .with_token(std::env::var("BRIEFCASE_TOKEN").unwrap_or_default()),
//! )
//! .await?;
//!
//! // Everything a member can reach, one page at a time.
//! let page = client.list_entries(&ListEntries::default()).await?;
//! for entry in &page.items {
//!     println!("{}  {}", entry.path, entry.name);
//! }
//!
//! // One upload operation, whatever the file weighs.
//! let entry = client
//!     .upload(&Upload::file(Destination::path("private/cos:tos/notes"), "./report.pdf")?)
//!     .await?;
//! println!("stored at {}", entry.permanent_url);
//! # Ok(())
//! # }
//! ```
//!
//! [`Client::connect`] reads `GET /api/version` first and verifies the service
//! identity, negotiated API major, and every operation this build calls by
//! exact ID/revision/method/path. Unknown operation IDs are additive, while
//! duplicate IDs are refused, so a changed operation fails at startup rather
//! than mid-request.
//! [`Client::new_unchecked`] skips that when a caller has already decided the
//! pairing is fine.
//!
//! # Reading the answers
//!
//! Briefcase reports an entry the caller may not read exactly as one that does
//! not exist, so [`Error::is_not_found`] covers both and nothing confirms that
//! a hidden entry is there. [`Error::code`] carries the stable error code, and
//! [`Error::retry_after`] carries the wait a spent upload allowance names.
//!
//! # Authentication
//!
//! The contracted API is a bearer surface: an IAM access token for a Carbon or
//! Silicon, plus the organization the client was built for. Login exchanges an
//! IAM short-lived token with [`Client::login_with_slt`]; no client method ever
//! accepts Briefcase's IAM Application secret. Applications use the exact
//! [`delegated`] operations or the compatible one-shot upload, each carrying
//! a fresh IAM proof instead of a bearer. Private staged transfers use only a
//! narrow capability; publication requires another fresh proof. A typed
//! [`EnvironmentKey`] independently selects an isolated testing plane.
//!
//! [Silicon Briefcase]: https://briefcase.teamofsilicons.com

pub mod honeycomb;

mod api;
mod client;
mod config;
mod contract;
pub mod daemon;
mod error;
mod media;
pub mod models;
mod requests;
pub mod update;

pub use api::delegated;
pub use api::delegated::{
    DelegatedCancelUpload, DelegatedCommitUpload, DelegatedCreateFolder, DelegatedInvite,
    DelegatedLinkAccess, DelegatedListEntries, DelegatedManifest, DelegatedOperation,
    DelegatedReadFile, DelegatedReserveUpload, DelegatedTrashEntry, DelegatedUploadQuery,
    DelegatedUploadReservation, DelegatedUploadState, DelegatedUploadStatus, OboProof,
    UploadCapability,
};
pub use api::{
    BugReport, ContentStream, ExpiryChange, Invitation, InvitationPage, Invite, LinkAccess,
    LogEvent, LogPage, PublicEntry, PublicPage, Recipient, ReportReceipt,
};
pub use client::{Client, IdempotencyKey};
pub use config::{
    ApplicationId, Config, Credential, DEFAULT_CONNECT_TIMEOUT, DEFAULT_REQUEST_TIMEOUT,
    DEFAULT_TRANSFER_TIMEOUT, EnvironmentKey, IamApplicationSecret, IamEnvironmentKey,
};
pub use contract::{API_VERSION, OPERATIONS, OperationRevision, ServedOperation, ServiceVersion};
pub use error::{ApiError, Error, IncompatibleContract, OperationMismatch, Result};
pub use media::{DEFAULT_CONTENT_TYPE, guess_content_type};
pub use models::{
    AccessRight, ActivityEvent, ActorRef, ActorType, BucketConfiguration, BucketConfigurationState,
    BucketConfigurationStatus, DailyUsageMeasure, EffectiveAccess, EffectivePermission,
    EncryptionMode, Entry, EntryPage, EntryType, EntryVisibility, FileVersion, FileVersionPage,
    IamInfo, LoginActor, LoginStatus, Notification, NotificationDecision, NotificationInbox,
    NotificationKind, NotificationSubject, OrganizationUsage, PermissionGrant,
    PermissionInspection, RenderKind, RootType, SearchResult, ServiceStatus, SessionActor,
    SessionTokens, TestingEnvironment, TestingEnvironmentCleaning, TestingEnvironmentCreator,
    TestingEnvironmentKey, TestingEnvironmentPage, TestingEnvironmentSelf,
    TestingEnvironmentStatus, TestingEnvironmentWithKey, UsageMeasure,
};
pub use requests::{
    ByteRange, Destination, EntryUpdate, ListEntries, NewFolder, NewGrant, OnBehalfOfUpload,
    PermissionQuery, TestingEnvironmentCreate, TestingEnvironmentIamPairing,
    TestingEnvironmentUpdate, Upload, UploadSource,
};
pub use update::UpdateStatus;

/// Operational telemetry and explicit opt-out controls.
pub mod telemetry;
