//! Private byte staging followed by a separate, freshly authorized publication.

use std::{fmt, path::Path};

use reqwest::{
    Method,
    header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderValue},
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize, de::Error as _};
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;
use tokio::io::AsyncReadExt as _;
use uuid::Uuid;

use super::sealed::Operation as _;
use super::{DelegatedManifest, DelegatedOperation, OboProof, non_nil, sealed};
use crate::{ApplicationId, Client, Error, Result, UploadSource, guess_content_type};

const MAX_UPLOAD_BYTES: u64 = 5 * 1024 * 1024 * 1024 * 1024;

/// Immutable destination and content identity for one logical upload.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedReserveUpload {
    /// Stable non-nil UUID retained across logical retries and status calls.
    pub operation_id: Uuid,
    /// Existing folder path; empty selects the represented member's app folder.
    pub parent_path: String,
    /// One file name, not a path.
    pub name: String,
    /// Media type for the resulting content.
    pub content_type: String,
    /// Exact raw-file size, including zero for an empty file.
    pub size: u64,
    /// Lowercase hexadecimal SHA-256 of the complete file, not its JSON manifest.
    pub sha256: String,
}

impl DelegatedReserveUpload {
    /// Hashes a local file with bounded memory before any proof is minted.
    ///
    /// The returned manifest contains no local path or credential. Keep the
    /// source immutable until transfer; the server checks its size and digest.
    /// The source must be a regular file, not a symlink or special file.
    ///
    /// # Errors
    ///
    /// Rejects unreadable files, invalid names, nil operation IDs and oversized content.
    pub async fn file(
        operation_id: Uuid,
        parent_path: impl Into<String>,
        path: impl AsRef<Path>,
    ) -> Result<Self> {
        non_nil(operation_id, "operation_id")?;
        let path = path.as_ref();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| Error::Configuration("the upload needs a UTF-8 file name".into()))?
            .to_owned();
        let (mut source, _) = open_regular_file(path).await?;
        let mut digest = Sha256::new();
        let mut size = 0_u64;
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let count = source
                .read(&mut buffer)
                .await
                .map_err(|error| local_io(path, error))?;
            if count == 0 {
                break;
            }
            size += count as u64;
            if size > MAX_UPLOAD_BYTES {
                return Err(Error::Configuration(
                    "delegated upload exceeds the 5 TiB limit".into(),
                ));
            }
            digest.update(&buffer[..count]);
        }
        let request = Self {
            operation_id,
            parent_path: parent_path.into(),
            content_type: guess_content_type(&name).to_owned(),
            name,
            size,
            sha256: format!("{:x}", digest.finalize()),
        };
        request.validate()?;
        Ok(request)
    }
}

/// Fresh authorization to publish one previously staged logical upload.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedCommitUpload {
    /// Stable logical UUID from the original reservation.
    pub operation_id: Uuid,
    /// Exact reservation UUID returned by Briefcase.
    pub upload_id: Uuid,
}

/// Fresh authorization to reconcile an upload, without resending its bytes.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedUploadQuery {
    /// Stable logical UUID from the original reservation.
    pub operation_id: Uuid,
}

/// Fresh authorization to cancel an unpublished logical upload.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedCancelUpload {
    /// Stable logical UUID from the original reservation.
    pub operation_id: Uuid,
}

macro_rules! upload_operation {
    ($request:ty, $endpoint:literal, $path:literal) => {
        impl DelegatedOperation for $request {
            const ENDPOINT_ID: &'static str = $endpoint;
            const PATH: &'static str = $path;
        }
        impl $request {
            /// Freezes exact JSON bytes before obtaining a valid IAM OBO token.
            ///
            /// # Errors
            ///
            /// Returns a local validation or serialization error.
            pub fn prepare(&self) -> Result<DelegatedManifest<Self>> {
                DelegatedManifest::new(self)
            }
        }
    };
}

upload_operation!(
    DelegatedReserveUpload,
    "briefcase.uploads.reserve",
    "/api/v1/obo/uploads/reserve"
);
upload_operation!(
    DelegatedCommitUpload,
    "briefcase.uploads.commit",
    "/api/v1/obo/uploads/commit"
);
upload_operation!(
    DelegatedUploadQuery,
    "briefcase.uploads.status",
    "/api/v1/obo/uploads/status"
);
upload_operation!(
    DelegatedCancelUpload,
    "briefcase.uploads.cancel",
    "/api/v1/obo/uploads/cancel"
);

impl sealed::Operation for DelegatedReserveUpload {
    fn validate(&self) -> Result<()> {
        non_nil(self.operation_id, "operation_id")?;
        if self.size > MAX_UPLOAD_BYTES {
            return Err(Error::Configuration(
                "delegated upload exceeds the 5 TiB limit".into(),
            ));
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(Error::Configuration(
                "sha256 must contain 64 lowercase hexadecimal characters".into(),
            ));
        }
        if self.name.is_empty()
            || self.name.contains('/')
            || matches!(self.name.as_str(), "." | "..")
            || self.name.chars().any(char::is_control)
        {
            return Err(Error::Configuration(
                "delegated upload requires one valid file name".into(),
            ));
        }
        self.content_type
            .parse::<mime::Mime>()
            .map_err(|_| Error::Configuration("invalid delegated upload content_type".into()))?;
        Ok(())
    }
}

impl sealed::Operation for DelegatedCommitUpload {
    fn validate(&self) -> Result<()> {
        non_nil(self.operation_id, "operation_id")?;
        non_nil(self.upload_id, "upload_id")
    }
}

impl sealed::Operation for DelegatedUploadQuery {
    fn validate(&self) -> Result<()> {
        non_nil(self.operation_id, "operation_id")
    }
}

impl sealed::Operation for DelegatedCancelUpload {
    fn validate(&self) -> Result<()> {
        non_nil(self.operation_id, "operation_id")
    }
}

/// Public lifecycle state; none except `Committed` represents a visible file.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegatedUploadState {
    /// A capability may be freshly issued to start byte transfer.
    Reserved,
    /// A transfer is in progress or its lease has not yet been reconciled.
    Receiving,
    /// Exact bytes are privately stored, awaiting fresh commit authorization.
    Staged,
    /// Atomic publication succeeded; reconcile using `published_entry_id`.
    Committed,
    /// Cancelled and no longer eligible for publication.
    Cancelled,
    /// Reservation expired and cannot be published.
    Expired,
    /// Provider state must be cleaned before another transfer can proceed.
    CleanupPending,
}

/// Credential-free status safe for an application's durable recording outbox.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DelegatedUploadStatus {
    /// Caller-owned logical upload identity.
    pub operation_id: Uuid,
    /// Server-owned reservation identity.
    pub upload_id: Uuid,
    /// Current lifecycle stage.
    pub state: DelegatedUploadState,
    /// Absolute reservation deadline; a capability cannot extend it.
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    /// Published resource, present only after a successful atomic commit.
    pub published_entry_id: Option<Uuid>,
}

/// Fresh reservation answer. Debug output always redacts its upload capability.
#[derive(Debug, Deserialize)]
pub struct DelegatedUploadReservation {
    /// Credential-free state that may be retained for reconciliation.
    #[serde(flatten)]
    pub status: DelegatedUploadStatus,
    /// Narrow byte-staging capability, issued only when a transfer can start.
    ///
    /// Never a file-read or publication credential. Do not log or serialize it
    /// into a general-purpose outbox; obtain a new one with fresh authority.
    pub capability: Option<UploadCapability>,
}

/// An opaque capability for one exact private upload, never general authority.
pub struct UploadCapability(SecretString);

impl UploadCapability {
    /// Wraps a capability returned by Briefcase without interpreting its format.
    ///
    /// # Errors
    ///
    /// Rejects empty values or characters that cannot be safely placed in a header.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.trim().is_empty() || HeaderValue::from_str(&value).is_err() {
            return Err(Error::Configuration(
                "an upload capability must be a nonempty HTTP header value".into(),
            ));
        }
        Ok(Self(SecretString::from(value)))
    }

    /// Explicitly exposes a capability for caller-owned secure handoff only.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        self.0.expose_secret()
    }

    fn into_header(self) -> Result<HeaderValue> {
        let mut value = HeaderValue::from_str(self.expose_secret())
            .map_err(|_| Error::Configuration("invalid upload capability header".into()))?;
        value.set_sensitive(true);
        Ok(value)
    }
}

impl fmt::Debug for UploadCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("UploadCapability(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for UploadCapability {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

impl Client {
    /// Reserves a private upload with a valid IAM OBO token over the exact manifest.
    ///
    /// # Errors
    ///
    /// Returns current-authorization, quota, conflicting-manifest or transport errors.
    /// An uncertain response needs a fresh status proof, never blind resubmission.
    pub async fn reserve_delegated_upload(
        &self,
        application: &ApplicationId,
        proof: OboProof,
        manifest: &DelegatedManifest<DelegatedReserveUpload>,
    ) -> Result<DelegatedUploadReservation> {
        let request = self.delegated_request(application, proof, manifest)?;
        self.receive_json(request.timeout(self.request_timeout()))
            .await
    }

    /// Publishes private staged bytes after a new, current authorization check.
    ///
    /// # Errors
    ///
    /// Returns authority, expiry, state, quota or transport errors. Reconcile
    /// uncertain outcomes by stable operation UUID before retrying with fresh proof.
    pub async fn commit_delegated_upload(
        &self,
        application: &ApplicationId,
        proof: OboProof,
        manifest: &DelegatedManifest<DelegatedCommitUpload>,
    ) -> Result<DelegatedUploadStatus> {
        let request = self.delegated_request(application, proof, manifest)?;
        self.receive_json(request.timeout(self.transfer_timeout()))
            .await
    }

    /// Reconciles one logical upload with freshly verified IAM authority.
    ///
    /// # Errors
    ///
    /// Returns proof, visibility, identity-binding or transport errors without retry.
    pub async fn delegated_upload_status(
        &self,
        application: &ApplicationId,
        proof: OboProof,
        manifest: &DelegatedManifest<DelegatedUploadQuery>,
    ) -> Result<DelegatedUploadStatus> {
        let request = self.delegated_request(application, proof, manifest)?;
        self.receive_json(request.timeout(self.request_timeout()))
            .await
    }

    /// Cancels unpublished staging without deleting an already published file.
    ///
    /// # Errors
    ///
    /// Returns current-authority, state or transport errors. Cleanup is durable
    /// and may remain pending after cancellation is accepted.
    pub async fn cancel_delegated_upload(
        &self,
        application: &ApplicationId,
        proof: OboProof,
        manifest: &DelegatedManifest<DelegatedCancelUpload>,
    ) -> Result<DelegatedUploadStatus> {
        let request = self.delegated_request(application, proof, manifest)?;
        self.receive_json(request.timeout(self.request_timeout()))
            .await
    }

    /// Streams exact bytes to private staging using only a narrow capability.
    ///
    /// Does not send an IAM bearer, parent token, proof or application secret.
    /// The selected organization and test plane still apply. A successful
    /// transfer is not publication; authorize publication with the current commit endpoint token separately.
    ///
    /// # Errors
    ///
    /// Returns local I/O, capability, length/digest, expiry, state or transport
    /// errors. Never retries automatically; reconcile through fresh IAM status.
    pub async fn transfer_delegated_upload(
        &self,
        upload_id: Uuid,
        capability: UploadCapability,
        source: &UploadSource,
    ) -> Result<DelegatedUploadStatus> {
        non_nil(upload_id, "upload_id")?;
        let (body, size) = match source {
            UploadSource::Bytes(bytes) => (reqwest::Body::from(bytes.clone()), bytes.len() as u64),
            UploadSource::File(path) => {
                let (file, size) = open_regular_file(path).await?;
                (
                    reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(file)),
                    size,
                )
            }
        };
        if size > MAX_UPLOAD_BYTES {
            return Err(Error::Configuration(
                "delegated upload exceeds the 5 TiB limit".into(),
            ));
        }
        let url = self.api_url(&["obo", "uploads", &upload_id.to_string(), "content"])?;
        let request = self
            .anonymous_request(Method::PUT, url)
            .header("x-org-id", self.organization())
            .header("x-briefcase-upload-capability", capability.into_header()?)
            .header(CONTENT_LENGTH, size)
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(body)
            .timeout(self.transfer_timeout());
        self.receive_json(request).await
    }

    /// Transfer an already-open immutable staging file with only its upload capability.
    /// This preserves descriptor ownership when the caller staged an anonymous file.
    ///
    /// # Errors
    /// Rejects non-regular or oversized files, local I/O, and provider errors.
    pub async fn transfer_delegated_upload_file(
        &self,
        upload_id: Uuid,
        capability: UploadCapability,
        mut file: tokio::fs::File,
    ) -> Result<DelegatedUploadStatus> {
        use tokio::io::AsyncSeekExt as _;
        non_nil(upload_id, "upload_id")?;
        let metadata = file
            .metadata()
            .await
            .map_err(|_| Error::Configuration("could not inspect staging file".into()))?;
        if !metadata.is_file() || metadata.len() > MAX_UPLOAD_BYTES {
            return Err(Error::Configuration(
                "staging source must be a bounded regular file".into(),
            ));
        }
        file.rewind()
            .await
            .map_err(|_| Error::Configuration("could not rewind staging file".into()))?;
        let url = self.api_url(&["obo", "uploads", &upload_id.to_string(), "content"])?;
        let request = self
            .anonymous_request(Method::PUT, url)
            .header("x-org-id", self.organization())
            .header("x-briefcase-upload-capability", capability.into_header()?)
            .header(CONTENT_LENGTH, metadata.len())
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(reqwest::Body::wrap_stream(
                tokio_util::io::ReaderStream::new(file),
            ))
            .timeout(self.transfer_timeout());
        self.receive_json(request).await
    }
}

fn local_io(path: &Path, source: std::io::Error) -> Error {
    Error::Io {
        path: path.display().to_string(),
        source,
    }
}

/// Validate the opened descriptor before reading or constructing a request.
/// Unix no-follow/nonblocking flags also close the last-component symlink/FIFO
/// substitution window between the initial metadata check and opening.
async fn open_regular_file(path: &Path) -> Result<(tokio::fs::File, u64)> {
    let before = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|error| local_io(path, error))?;
    if !before.is_file() {
        return Err(Error::Configuration(
            "delegated upload source must be a regular, non-symlink file".into(),
        ));
    }
    let mut options = tokio::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let file = options
        .open(path)
        .await
        .map_err(|error| local_io(path, error))?;
    let opened = file
        .metadata()
        .await
        .map_err(|error| local_io(path, error))?;
    if !opened.is_file() {
        return Err(Error::Configuration(
            "delegated upload source changed while opening; nothing was read".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if before.dev() != opened.dev() || before.ino() != opened.ino() {
            return Err(Error::Configuration(
                "delegated upload source changed while opening; nothing was read".into(),
            ));
        }
    }
    if opened.len() > MAX_UPLOAD_BYTES {
        return Err(Error::Configuration(
            "delegated upload exceeds the 5 TiB limit".into(),
        ));
    }
    Ok((file, opened.len()))
}
