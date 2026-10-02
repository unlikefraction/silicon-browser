//! Uploading bytes, reading them back, and file versions.

use std::path::Path;

use reqwest::{
    Method,
    multipart::{Form, Part},
};
use tokio::io::AsyncWriteExt as _;
use uuid::Uuid;

use crate::{
    client::{Client, IdempotencyKey},
    error::{Error, Result, io, transport},
    models::{Entry, FileVersionPage},
    requests::{ByteRange, Destination, OnBehalfOfUpload, Upload, UploadSource},
};

/// A file's bytes, arriving as they are read.
pub struct ContentStream {
    response: reqwest::Response,
    content_type: Option<String>,
    content_length: Option<u64>,
    content_range: Option<String>,
}

impl std::fmt::Debug for ContentStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ContentStream")
            .field("content_type", &self.content_type)
            .field("content_length", &self.content_length)
            .field("content_range", &self.content_range)
            .finish_non_exhaustive()
    }
}

impl ContentStream {
    /// Wraps a successful response for streaming.
    pub(crate) fn new(response: reqwest::Response) -> Self {
        let header = |name: reqwest::header::HeaderName| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned)
        };
        Self {
            content_type: header(reqwest::header::CONTENT_TYPE),
            content_length: response.content_length(),
            content_range: header(reqwest::header::CONTENT_RANGE),
            response,
        }
    }

    /// Returns the media type Briefcase served the bytes as.
    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// Returns the length of this response, when it is known.
    #[must_use]
    pub const fn content_length(&self) -> Option<u64> {
        self.content_length
    }

    /// Returns the `Content-Range` of a partial response.
    #[must_use]
    pub fn content_range(&self) -> Option<&str> {
        self.content_range.as_deref()
    }

    /// Reads the next chunk, or `None` at the end of the body.
    ///
    /// # Errors
    ///
    /// Returns a transport error when the stream breaks mid-file.
    pub async fn chunk(&mut self) -> Result<Option<Vec<u8>>> {
        self.response
            .chunk()
            .await
            .map(|chunk| chunk.map(|bytes| bytes.to_vec()))
            .map_err(transport)
    }

    /// Reads the whole body into memory.
    ///
    /// # Errors
    ///
    /// Returns a transport error when the stream breaks mid-file.
    pub async fn bytes(self) -> Result<Vec<u8>> {
        Ok(self.response.bytes().await.map_err(transport)?.to_vec())
    }

    /// Writes the whole body to a local file, returning the bytes written.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the file cannot be written, and a transport
    /// error when the stream breaks mid-file.
    pub async fn write_to_file(mut self, path: impl AsRef<Path>) -> Result<u64> {
        let path = path.as_ref();
        let display = path.display().to_string();
        let mut file = tokio::fs::File::create(path)
            .await
            .map_err(|error| io(display.clone(), error))?;
        let mut written = 0_u64;
        while let Some(chunk) = self.chunk().await? {
            file.write_all(&chunk)
                .await
                .map_err(|error| io(display.clone(), error))?;
            written += chunk.len() as u64;
        }
        file.flush()
            .await
            .map_err(|error| io(display.clone(), error))?;
        Ok(written)
    }
}

impl Client {
    /// Uploads a file of any supported size.
    ///
    /// Uploading a name an active file already carries publishes that file's
    /// next version and returns the same entry; the history keeps the complete
    /// immutable history.
    ///
    /// # Errors
    ///
    /// Returns an error when the destination is not writable, the file is not
    /// readable locally, the organization's daily upload allowance is spent
    /// (`daily_upload_limit_exhausted`, retryable after the reported delay), or
    /// its storage is full (`storage_limit_exhausted`).
    pub async fn upload(&self, upload: &Upload) -> Result<Entry> {
        let key = upload
            .idempotency_key
            .clone()
            .unwrap_or_else(IdempotencyKey::random);
        let content_type = upload
            .content_type
            .clone()
            .unwrap_or_else(|| "application/octet-stream".to_owned());
        let mut part = match &upload.source {
            UploadSource::Bytes(bytes) => Part::bytes(bytes.clone()),
            UploadSource::File(path) => {
                let file = tokio::fs::File::open(path)
                    .await
                    .map_err(|error| io(path.display().to_string(), error))?;
                Part::stream(reqwest::Body::wrap_stream(
                    tokio_util::io::ReaderStream::new(file),
                ))
            }
        };
        part = part
            .file_name(upload.file_name.clone())
            .mime_str(&content_type)
            .map_err(|error| Error::Configuration(format!("invalid content type: {error}")))?;

        let mut form = match &upload.destination {
            Destination::Id(id) => Form::new().text("parent_id", id.to_string()),
            Destination::Path(path) => Form::new().text("path", path.clone()),
        };
        if let Some(minutes) = upload.self_destruct_minutes {
            form = form.text("self_destruct_minutes", minutes.to_string());
        }
        let form = form.part("file", part);

        let request = self
            .request(Method::POST, self.api_url(&["uploads"])?)
            .header("idempotency-key", key.as_str())
            .multipart(form)
            .timeout(self.transfer_timeout());
        self.receive_json(request).await
    }

    /// Opens a file's bytes for reading, optionally one range of them.
    ///
    /// Briefcase relays the bytes itself rather than signing a provider URL,
    /// so every read stays bound to the caller's current IAM identity.
    ///
    /// # Errors
    ///
    /// Returns a not-found error when the file is not visible to the caller,
    /// and a range error when the requested range lies past the end.
    pub async fn read_content(
        &self,
        entry_id: Uuid,
        range: Option<ByteRange>,
    ) -> Result<ContentStream> {
        let url = self.api_url(&["entries", &entry_id.to_string(), "content"])?;
        self.open_content(url, range).await
    }

    /// Opens a file's bytes as an attachment.
    ///
    /// # Errors
    ///
    /// Returns a not-found error when the file is not visible to the caller.
    pub async fn download(&self, entry_id: Uuid) -> Result<ContentStream> {
        let url = self.api_url(&["entries", &entry_id.to_string(), "download"])?;
        self.open_content(url, None).await
    }

    /// Downloads a file straight to a local path, returning the bytes written.
    ///
    /// # Errors
    ///
    /// Returns a not-found error when the file is not visible, and an I/O
    /// error when the local file cannot be written.
    pub async fn download_to_file(&self, entry_id: Uuid, path: impl AsRef<Path>) -> Result<u64> {
        self.download(entry_id).await?.write_to_file(path).await
    }

    /// Opens the bytes behind a permanent URL path.
    ///
    /// # Errors
    ///
    /// Returns a not-found error when nothing readable sits at that path, or
    /// when the path names a folder rather than a file.
    pub async fn read_content_at(&self, path: &str) -> Result<ContentStream> {
        let mut url = self.permanent_url(path)?;
        url.query_pairs_mut().append_pair("disposition", "inline");
        self.open_content(url, None).await
    }

    async fn open_content(&self, url: url::Url, range: Option<ByteRange>) -> Result<ContentStream> {
        let mut request = self
            .request(Method::GET, url)
            .timeout(self.transfer_timeout());
        if let Some(range) = range {
            request = request.header(reqwest::header::RANGE, range.header_value());
        }
        let response = self.receive(request).await?;
        let header = |name: reqwest::header::HeaderName| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned)
        };
        Ok(ContentStream {
            content_type: header(reqwest::header::CONTENT_TYPE),
            content_length: response.content_length(),
            content_range: header(reqwest::header::CONTENT_RANGE),
            response,
        })
    }

    /// Lists a file's retained versions, newest first.
    ///
    /// # Errors
    ///
    /// Returns an error when the file is not visible to the caller.
    pub async fn versions(&self, entry_id: Uuid) -> Result<FileVersionPage> {
        self.versions_page(entry_id, None).await
    }

    /// Continues through the complete retained version history.
    ///
    /// # Errors
    /// Returns visibility, cursor, or transport errors.
    pub async fn versions_page(
        &self,
        entry_id: Uuid,
        cursor: Option<&str>,
    ) -> Result<FileVersionPage> {
        let url = self.api_url(&["entries", &entry_id.to_string(), "versions"])?;
        let mut request = self
            .request(Method::GET, url)
            .timeout(self.request_timeout());
        if let Some(cursor) = cursor {
            request = request.query(&[("cursor", cursor)]);
        }
        self.receive_json(request).await
    }

    /// Restores an older version as the file's current content.
    ///
    /// Restoring adds to the history rather than erasing it, and stores a
    /// second copy of those bytes, so it answers to the storage ceiling.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller cannot change the file, or the version
    /// is no longer retained.
    pub async fn restore_version(&self, entry_id: Uuid, version_id: Uuid) -> Result<Entry> {
        self.restore_version_with_key(entry_id, version_id, &IdempotencyKey::random())
            .await
    }

    /// Restores a version with a caller-owned retry identity.
    ///
    /// Persist `idempotency_key` before the first attempt and reuse it after an
    /// uncertain transport failure so a retry returns the original result.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller cannot change the file, or the version
    /// is no longer retained.
    pub async fn restore_version_with_key(
        &self,
        entry_id: Uuid,
        version_id: Uuid,
        idempotency_key: &IdempotencyKey,
    ) -> Result<Entry> {
        let url = self.api_url(&[
            "entries",
            &entry_id.to_string(),
            "versions",
            &version_id.to_string(),
            "restore",
        ])?;
        let request = self
            .request(Method::POST, url)
            .header("idempotency-key", idempotency_key.as_str())
            .timeout(self.transfer_timeout());
        self.receive_json(request).await
    }

    /// Retired raw upload retained only as a source-compatible migration error.
    /// Use `reserve_delegated_upload`, `transfer_delegated_upload` and
    /// `commit_delegated_upload` with reusable endpoint access tokens.
    ///
    /// # Errors
    /// Always returns a configuration error without transmitting credentials or bytes.
    #[allow(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "retain the public async signature while refusing the retired operation locally"
    )]
    pub async fn create_file_on_behalf_of(&self, upload: &OnBehalfOfUpload) -> Result<Entry> {
        crate::ApplicationId::new(upload.app_id.clone())?;
        Err(crate::Error::Configuration(
            "Raw OBO uploads are retired; use reserve, content and commit".into(),
        ))
    }
}
