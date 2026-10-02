//! Delegated operations authorized by a reusable IAM OBO access token.
//!
//! Each receiver verifies the selected endpoint and live authority online.
//! Immutable manifests retain logical idempotency and content integrity; their
//! digest is not a per-request IAM signature. No actor bearer is forwarded.

use std::{fmt, marker::PhantomData};

use reqwest::{Method, RequestBuilder, header::HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

mod uploads;
pub use uploads::{
    DelegatedCancelUpload, DelegatedCommitUpload, DelegatedReserveUpload, DelegatedUploadQuery,
    DelegatedUploadReservation, DelegatedUploadState, DelegatedUploadStatus, UploadCapability,
};

use crate::{
    ApplicationId, ByteRange, Client, ContentStream, Entry, EntryPage, Error, Result,
    client::json_body,
};

/// HTTP method used for every delegated JSON manifest.
pub const METHOD: &str = "POST";
/// IAM endpoint identifier for folder creation.
pub const CREATE_FOLDER_ENDPOINT_ID: &str = "briefcase.folders.create";
/// Exact IAM request-binding path for folder creation.
pub const CREATE_FOLDER_PATH: &str = "/api/v1/obo/folders/create";
/// IAM endpoint identifier for entry listing.
pub const LIST_ENTRIES_ENDPOINT_ID: &str = "briefcase.entries.list";
/// Exact IAM request-binding path for entry listing.
pub const LIST_ENTRIES_PATH: &str = "/api/v1/obo/entries/list";
/// IAM endpoint identifier for a current file read.
pub const READ_FILE_ENDPOINT_ID: &str = "briefcase.files.read";
/// Exact IAM request-binding path for a current file read.
pub const READ_FILE_PATH: &str = "/api/v1/obo/files/read";
/// IAM endpoint identifier for recoverable deletion.
pub const TRASH_ENTRY_ENDPOINT_ID: &str = "briefcase.entries.trash";
/// Exact IAM request-binding path for recoverable deletion.
pub const TRASH_ENTRY_PATH: &str = "/api/v1/obo/entries/trash";

/// A reusable IAM OBO access token supplied by the calling application.
/// The historical type name is retained for source compatibility. Receivers
/// accept only the current token class; a clone may authorize another request
/// within the approved graph while its live authority remains valid.
#[derive(Clone)]
pub struct OboProof(SecretString);

impl OboProof {
    /// Wraps a proof without interpreting or changing its opaque token format.
    ///
    /// # Errors
    ///
    /// Rejects an empty value or one that cannot safely be an HTTP header.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.trim().is_empty() || HeaderValue::from_str(&value).is_err() {
            return Err(Error::Configuration(
                "an OBO proof must be a nonempty HTTP header value".into(),
            ));
        }
        Ok(Self(SecretString::from(value)))
    }

    fn into_header(self) -> Result<HeaderValue> {
        let mut header = HeaderValue::from_str(self.0.expose_secret())
            .map_err(|_| Error::Configuration("invalid OBO proof header".into()))?;
        header.set_sensitive(true);
        Ok(header)
    }
}

impl fmt::Debug for OboProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OboProof(<redacted>)")
    }
}

mod sealed {
    pub trait Operation {
        fn validate(&self) -> crate::Result<()>;
    }
}

/// One of the fixed delegated JSON operations supported by this client.
///
/// Sealed to prevent a manifest for one endpoint being sent to another.
pub trait DelegatedOperation: sealed::Operation + Serialize {
    /// IAM endpoint identifier whose metadata schema is the empty object.
    const ENDPOINT_ID: &'static str;
    /// Exact, versioned request path verified by the receiving endpoint.
    const PATH: &'static str;
}

/// Immutable exact bytes to hash for integrity and logical idempotency, then transmit.
///
/// Prepared manifests can be cloned or retained for logical retries, but
/// tokens may be reused while valid. Changing the DTO cannot change these bytes.
#[derive(Clone)]
pub struct DelegatedManifest<T: DelegatedOperation> {
    bytes: Vec<u8>,
    sha256: String,
    operation: PhantomData<fn() -> T>,
}

impl<T: DelegatedOperation> DelegatedManifest<T> {
    /// Validates and serializes the request exactly once.
    ///
    /// # Errors
    ///
    /// Returns a local configuration error for invalid identifiers, ambiguous
    /// list parents, invalid limits, or an unserializable request.
    pub fn new(request: &T) -> Result<Self> {
        request.validate()?;
        let bytes = json_body(request)?;
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        Ok(Self {
            bytes,
            sha256,
            operation: PhantomData,
        })
    }

    /// The exact JSON bytes the SDK will send, without re-serialization.
    #[must_use]
    pub fn body_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Lowercase hexadecimal SHA-256 of `body_bytes()`, for local integrity and logical idempotency.
    #[must_use]
    pub fn body_sha256(&self) -> &str {
        &self.sha256
    }

    /// The HTTP method to bind into the OBO token.
    #[must_use]
    pub const fn method(&self) -> &'static str {
        METHOD
    }

    /// The full versioned path to bind into the OBO token.
    #[must_use]
    pub const fn path(&self) -> &'static str {
        T::PATH
    }

    /// The IAM endpoint to select when minting the OBO token.
    #[must_use]
    pub const fn endpoint_id(&self) -> &'static str {
        T::ENDPOINT_ID
    }
}

impl<T: DelegatedOperation> fmt::Debug for DelegatedManifest<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DelegatedManifest")
            .field("endpoint_id", &T::ENDPOINT_ID)
            .field("path", &T::PATH)
            .field("body_length", &self.bytes.len())
            .field("body_sha256", &self.sha256)
            .finish()
    }
}

/// Creates one child folder using a stable caller-owned logical operation UUID.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedCreateFolder {
    /// Non-nil UUID retained with this unchanged manifest across logical retries.
    pub operation_id: Uuid,
    /// Existing parent path; an empty string selects the private application folder.
    pub parent_path: String,
    /// A single child folder name, not a path or root declaration.
    pub name: String,
}

/// Lists the ordinary privacy-filtered entry view, with every input in an immutable manifest.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedListEntries {
    /// Existing parent UUID, mutually exclusive with `path`.
    pub parent_id: Option<Uuid>,
    /// Existing parent path; omitted with `parent_id` to browse organization roots.
    pub path: Option<String>,
    /// Optional filter expression; without a parent, searches the visible tree.
    pub filter: Option<String>,
    /// Opaque cursor from the preceding page; each page is verified online.
    pub cursor: Option<String>,
    /// Page size from 1 through 100, defaulting to 100.
    pub limit: Option<u16>,
}

/// Reads a current file with the range and disposition bound into its JSON body.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedReadFile {
    /// Non-nil UUID of the exact file inside the represented organization.
    pub entry_id: Uuid,
    /// Optional HTTP byte-range syntax, for example `bytes=0-1023`.
    pub range: Option<String>,
    /// `true` for attachment delivery; `false` for sandboxed inline content.
    #[serde(default)]
    pub download: bool,
}

impl DelegatedReadFile {
    /// Uses a typed byte range instead of constructing HTTP range syntax.
    #[must_use]
    pub fn with_range(mut self, range: ByteRange) -> Self {
        self.range = Some(range.header_value());
        self
    }
}

/// Moves an entry to the recoverable bin, not permanent deletion.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedTrashEntry {
    /// Non-nil UUID retained with this unchanged manifest across logical retries.
    pub operation_id: Uuid,
    /// Non-nil UUID of the entry to trash.
    pub entry_id: Uuid,
}

macro_rules! delegated_operation {
    ($request:ty, $endpoint:ident, $path:ident) => {
        impl DelegatedOperation for $request {
            const ENDPOINT_ID: &'static str = $endpoint;
            const PATH: &'static str = $path;
        }
        impl $request {
            /// Freezes the exact manifest bytes before transmitting the request.
            ///
            /// # Errors
            ///
            /// Returns a local error when validation or JSON serialization fails.
            pub fn prepare(&self) -> Result<DelegatedManifest<Self>> {
                DelegatedManifest::new(self)
            }
        }
    };
}

delegated_operation!(
    DelegatedCreateFolder,
    CREATE_FOLDER_ENDPOINT_ID,
    CREATE_FOLDER_PATH
);
delegated_operation!(
    DelegatedListEntries,
    LIST_ENTRIES_ENDPOINT_ID,
    LIST_ENTRIES_PATH
);
delegated_operation!(DelegatedReadFile, READ_FILE_ENDPOINT_ID, READ_FILE_PATH);
delegated_operation!(
    DelegatedTrashEntry,
    TRASH_ENTRY_ENDPOINT_ID,
    TRASH_ENTRY_PATH
);

fn non_nil(id: Uuid, field: &str) -> Result<()> {
    if id.is_nil() {
        Err(Error::Configuration(format!(
            "delegated {field} must not be nil"
        )))
    } else {
        Ok(())
    }
}

impl sealed::Operation for DelegatedCreateFolder {
    fn validate(&self) -> Result<()> {
        non_nil(self.operation_id, "operation_id")
    }
}

impl sealed::Operation for DelegatedListEntries {
    fn validate(&self) -> Result<()> {
        if self.parent_id.is_some() && self.path.is_some() {
            return Err(Error::Configuration(
                "delegated listing accepts parent_id or path, not both".into(),
            ));
        }
        if let Some(parent) = self.parent_id {
            non_nil(parent, "parent_id")?;
        }
        if self.limit.is_some_and(|limit| !(1..=100).contains(&limit)) {
            return Err(Error::Configuration(
                "delegated listing limit must be from 1 through 100".into(),
            ));
        }
        Ok(())
    }
}

impl sealed::Operation for DelegatedReadFile {
    fn validate(&self) -> Result<()> {
        non_nil(self.entry_id, "entry_id")?;
        if self
            .range
            .as_ref()
            .is_some_and(|range| HeaderValue::from_str(range).is_err())
        {
            return Err(Error::Configuration(
                "invalid delegated range header".into(),
            ));
        }
        Ok(())
    }
}

impl sealed::Operation for DelegatedTrashEntry {
    fn validate(&self) -> Result<()> {
        non_nil(self.operation_id, "operation_id")?;
        non_nil(self.entry_id, "entry_id")
    }
}

impl Client {
    /// Creates a folder using current endpoint authority for the prepared manifest.
    ///
    /// # Errors
    ///
    /// Returns proof, authorization, conflict, or transport errors without retry.
    pub async fn create_folder_on_behalf_of(
        &self,
        application: &ApplicationId,
        proof: OboProof,
        manifest: &DelegatedManifest<DelegatedCreateFolder>,
    ) -> Result<Entry> {
        let request = self.delegated_request(application, proof, manifest)?;
        self.receive_json(request.timeout(self.request_timeout()))
            .await
    }

    /// Lists one page using the endpoint or shared-chain access token.
    ///
    /// # Errors
    ///
    /// Returns proof, visibility, pagination, or transport errors without retry.
    pub async fn list_entries_on_behalf_of(
        &self,
        application: &ApplicationId,
        proof: OboProof,
        manifest: &DelegatedManifest<DelegatedListEntries>,
    ) -> Result<EntryPage> {
        let request = self.delegated_request(application, proof, manifest)?;
        self.receive_json(request.timeout(self.request_timeout()))
            .await
    }

    /// Streams a file with no unbound Range header or disposition query.
    ///
    /// # Errors
    ///
    /// Returns proof, visibility, range, or transport errors without retry.
    /// Reading or dropping the returned stream never triggers maintenance.
    pub async fn read_file_on_behalf_of(
        &self,
        application: &ApplicationId,
        proof: OboProof,
        manifest: &DelegatedManifest<DelegatedReadFile>,
    ) -> Result<ContentStream> {
        let request = self.delegated_request(application, proof, manifest)?;
        let response = self
            .receive(request.timeout(self.transfer_timeout()))
            .await?;
        Ok(ContentStream::new(response))
    }

    /// Trashes an entry using a stable operation UUID and a valid OBO token.
    ///
    /// # Errors
    ///
    /// Returns proof, subtree-delete authorization, or transport errors without retry.
    pub async fn trash_entry_on_behalf_of(
        &self,
        application: &ApplicationId,
        proof: OboProof,
        manifest: &DelegatedManifest<DelegatedTrashEntry>,
    ) -> Result<()> {
        let request = self.delegated_request(application, proof, manifest)?;
        self.receive(request.timeout(self.request_timeout()))
            .await
            .map(drop)
    }

    fn delegated_request<T: DelegatedOperation>(
        &self,
        application: &ApplicationId,
        proof: OboProof,
        manifest: &DelegatedManifest<T>,
    ) -> Result<RequestBuilder> {
        let segments: Vec<_> = manifest.path().trim_start_matches('/').split('/').collect();
        let url = self.origin_url(&segments)?;
        // An OBO request must not also carry the configured IAM bearer. Only
        // the Briefcase test-plane selector is inherited from client config.
        Ok(self
            .anonymous_request(Method::POST, url)
            .header("x-org-id", self.organization())
            .header("x-app-id", application.as_str())
            .header("x-iam-obo-access-token", proof.into_header()?)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(manifest.bytes.clone()))
    }
}

/// Critical invitation manifest; IAM must require user approval for this endpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedInvite {
    /// Stable UUID reused with a valid OBO token on retry.
    pub operation_id: Uuid,
    /// Entry inside the calling application's namespace.
    pub entry_id: Uuid,
    /// Recipient and granted rights.
    pub invitation: crate::Invite,
}
impl sealed::Operation for DelegatedInvite {
    fn validate(&self) -> Result<()> {
        non_nil(self.operation_id, "operation_id")?;
        non_nil(self.entry_id, "entry_id")
    }
}
impl DelegatedOperation for DelegatedInvite {
    const ENDPOINT_ID: &'static str = "briefcase.invitations.create";
    const PATH: &'static str = "/api/v1/obo/invitations";
}
/// Critical anyone-with-link manifest; requires user approval in IAM.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedLinkAccess {
    /// Stable logical operation ID.
    pub operation_id: Uuid,
    /// Entry inside the calling application's namespace.
    pub entry_id: Uuid,
    /// Desired explicit link setting.
    pub enabled: bool,
    /// With `enabled`, makes this an expiring link that ends after 1 to 43,200
    /// minutes. Part of the immutable request body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_minutes: Option<u32>,
}
impl sealed::Operation for DelegatedLinkAccess {
    fn validate(&self) -> Result<()> {
        non_nil(self.operation_id, "operation_id")?;
        non_nil(self.entry_id, "entry_id")
    }
}
impl DelegatedOperation for DelegatedLinkAccess {
    const ENDPOINT_ID: &'static str = "briefcase.link_access.update";
    const PATH: &'static str = "/api/v1/obo/link-access";
}
impl Client {
    /// Invites a member with a valid IAM OBO token for this critical endpoint.
    /// # Errors
    /// Returns proof, permission, recipient, or transport errors without retrying.
    pub async fn invite_on_behalf_of(
        &self,
        app: &ApplicationId,
        proof: OboProof,
        manifest: &DelegatedManifest<DelegatedInvite>,
    ) -> Result<crate::Invitation> {
        self.receive_json(
            self.delegated_request(app, proof, manifest)?
                .timeout(self.request_timeout()),
        )
        .await
    }
    /// Sets anonymous read access using a valid OBO token for this critical endpoint.
    /// # Errors
    /// Returns proof, permission, protected-folder, or transport errors without retrying.
    pub async fn set_link_access_on_behalf_of(
        &self,
        app: &ApplicationId,
        proof: OboProof,
        manifest: &DelegatedManifest<DelegatedLinkAccess>,
    ) -> Result<crate::LinkAccess> {
        self.receive_json(
            self.delegated_request(app, proof, manifest)?
                .timeout(self.request_timeout()),
        )
        .await
    }
}
