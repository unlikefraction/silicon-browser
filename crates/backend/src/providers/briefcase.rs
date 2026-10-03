//! Briefcase 3 delegated uploads: reusable OBO controls and capability-only byte transfer.
//! The worker owns token refresh, immutable operation identity, retries and receipt persistence.

use std::{collections::HashSet, time::Duration};

use chrono::{DateTime, Utc};
use reqwest::header::HeaderValue;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use url::Url;
use uuid::Uuid;

use super::artifact::OnBehalfOfGrant;
use super::error::{ProviderError, ProviderResult, transport};
use crate::url_policy::is_https_or_loopback_http;

const PROVIDER: &str = "briefcase";
pub const BRIEFCASE_RECORDING_ENDPOINTS: [&str; 4] =
    ["briefcase.uploads.reserve", "briefcase.uploads.commit", "briefcase.uploads.status", "briefcase.entries.list"];
pub const DEFAULT_BRIEFCASE_UPLOAD_LIMIT: usize = 64 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_URL_BYTES: usize = 4096;

#[derive(Clone)]
pub struct BriefcaseClient {
    http: reqwest::Client,
    endpoint: Url,
    testing_key: Option<HeaderValue>,
    max_upload_bytes: usize,
}

impl std::fmt::Debug for BriefcaseClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BriefcaseClient")
            .field("endpoint", &self.endpoint)
            .field("testing_environment", &self.testing_key.is_some())
            .field("max_upload_bytes", &self.max_upload_bytes)
            .finish()
    }
}

/// Persist this intent before the first reserve; retries must retain every field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BriefcaseUploadManifest {
    pub operation_id: Uuid,
    pub parent_path: String,
    pub name: String,
    pub content_type: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BriefcaseUploadState {
    Reserved,
    Receiving,
    Staged,
    Committed,
    Cancelled,
    Expired,
    CleanupPending,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BriefcaseUploadStatus {
    pub operation_id: Uuid,
    pub upload_id: Uuid,
    pub state: BriefcaseUploadState,
    pub expires_at: DateTime<Utc>,
    pub published_entry_id: Option<Uuid>,
}

#[derive(Deserialize)]
pub struct BriefcaseUploadReservation {
    #[serde(flatten)]
    pub status: BriefcaseUploadStatus,
    capability: Option<String>,
}

impl std::fmt::Debug for BriefcaseUploadReservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BriefcaseUploadReservation")
            .field("status", &self.status)
            .field("has_capability", &self.capability.is_some())
            .finish()
    }
}

/// Metadata resolved after a committed upload; links require Briefcase authentication.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct BriefcaseEntry {
    pub id: Uuid,
    pub org_id: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub name: String,
    pub path: String,
    pub content_type: Option<String>,
    pub size: u64,
    pub permanent_url: String,
    pub origin_app_id: Option<String>,
}

impl BriefcaseClient {
    /// Use a root HTTPS origin (loopback HTTP is allowed for local fixtures).
    /// Testing uses the imported Briefcase IAM app secret, never an IAM root key.
    pub fn new(origin: &str, testing_key: Option<&str>) -> ProviderResult<Self> {
        Self::with_upload_limit(origin, testing_key, DEFAULT_BRIEFCASE_UPLOAD_LIMIT)
    }

    pub fn with_upload_limit(origin: &str, testing_key: Option<&str>, max_upload_bytes: usize) -> ProviderResult<Self> {
        if max_upload_bytes == 0 {
            return Err(invalid("Briefcase upload limit must be positive"));
        }
        let endpoint = clean_url(origin)?;
        if endpoint.path() != "/" {
            return Err(invalid("Briefcase URL must be a root origin without an API path"));
        }
        let testing_key = testing_key.map(testing_header).transpose()?;
        let http = reqwest::Client::builder()
            .user_agent(concat!("silicon-browser/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map_err(|error| transport(PROVIDER, error))?;
        Ok(Self { http, endpoint, testing_key, max_upload_bytes })
    }

    /// Select the credential returned for this token's recipient context, including clearing it
    /// for production. A missing or invalid testing credential must be rejected by the caller.
    pub fn with_testing_secret(&self, testing_key: Option<&str>) -> ProviderResult<Self> {
        let mut client = self.clone();
        client.testing_key = testing_key.map(testing_header).transpose()?;
        Ok(client)
    }

    pub fn max_upload_bytes(&self) -> usize {
        self.max_upload_bytes
    }

    /// SHA-256 is a Briefcase manifest/integrity field, not an IAM proof signature.
    pub fn body_sha256(&self, bytes: &[u8]) -> ProviderResult<String> {
        if bytes.len() > self.max_upload_bytes {
            return Err(invalid("Briefcase upload exceeds the configured size limit"));
        }
        Ok(hex::encode(Sha256::digest(bytes)))
    }

    /// Hash and rewind the same immutable file handle using bounded memory.
    pub async fn hash_file(&self, file: &mut tokio::fs::File) -> ProviderResult<(String, u64)> {
        let metadata = file.metadata().await.map_err(|_| invalid("could not inspect staging file"))?;
        if !metadata.is_file() || metadata.len() > self.max_upload_bytes as u64 {
            return Err(invalid("staging file exceeds the configured upload limit or is not a regular file"));
        }
        file.rewind().await.map_err(|_| invalid("could not rewind staging file"))?;
        let mut digest = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        let mut size = 0u64;
        loop {
            let count = file.read(&mut buffer).await.map_err(|_| invalid("could not read staging file"))?;
            if count == 0 {
                break;
            }
            size += count as u64;
            if size > self.max_upload_bytes as u64 {
                return Err(invalid("staging file grew beyond upload limit"));
            }
            digest.update(&buffer[..count]);
        }
        if size != metadata.len() {
            return Err(invalid("staging file size changed while hashing"));
        }
        file.rewind().await.map_err(|_| invalid("could not rewind staging file"))?;
        Ok((hex::encode(digest.finalize()), size))
    }

    pub async fn reserve_upload(
        &self,
        org_id: &str,
        app_id: &str,
        access: &OnBehalfOfGrant,
        manifest: &BriefcaseUploadManifest,
    ) -> ProviderResult<BriefcaseUploadReservation> {
        self.validate_manifest(manifest)?;
        let request = self.control("/api/v1/obo/uploads/reserve", org_id, app_id, access)?.json(manifest);
        let result: BriefcaseUploadReservation = self.response(request).await?;
        validate_status(&result.status, manifest.operation_id, None)?;
        if let Some(capability) = &result.capability
            && (result.status.state != BriefcaseUploadState::Reserved || secret_header(capability).is_err())
        {
            return Err(invalid_response("invalid upload reservation capability"));
        }
        Ok(result)
    }

    pub async fn upload_status(
        &self,
        org_id: &str,
        app_id: &str,
        access: &OnBehalfOfGrant,
        operation_id: Uuid,
    ) -> ProviderResult<BriefcaseUploadStatus> {
        require_uuid(operation_id)?;
        let request = self
            .control("/api/v1/obo/uploads/status", org_id, app_id, access)?
            .json(&json!({"operation_id": operation_id}));
        let result = self.response(request).await?;
        validate_status(&result, operation_id, None)?;
        Ok(result)
    }

    /// Transfer credentials cannot publish. Never send an OBO token or application ID here.
    pub async fn transfer_upload(
        &self,
        org_id: &str,
        reservation: &BriefcaseUploadReservation,
        mut file: tokio::fs::File,
        size: u64,
    ) -> ProviderResult<BriefcaseUploadStatus> {
        validate_status(&reservation.status, reservation.status.operation_id, None)?;
        if reservation.status.state != BriefcaseUploadState::Reserved || reservation.status.expires_at <= Utc::now() {
            return Err(invalid("upload reservation is not available for transfer"));
        }
        let capability = reservation.capability.as_deref().ok_or_else(|| invalid("upload capability is absent"))?;
        let metadata = file.metadata().await.map_err(|_| invalid("could not inspect staging file"))?;
        if !metadata.is_file() || metadata.len() != size || size > self.max_upload_bytes as u64 {
            return Err(invalid("staging file does not match the declared bounded upload size"));
        }
        file.rewind().await.map_err(|_| invalid("could not rewind staging file"))?;
        let request = self
            .plane(self.http.put(self.url(&format!("/api/v1/obo/uploads/{}/content", reservation.status.upload_id))))
            .header("x-org-id", identifier_header(org_id)?)
            .header("x-briefcase-upload-capability", secret_header(capability)?)
            .header("content-type", "application/octet-stream")
            .header(reqwest::header::CONTENT_LENGTH, size)
            .body(reqwest::Body::from(file));
        let result = self.response(request).await?;
        validate_status(&result, reservation.status.operation_id, Some(reservation.status.upload_id))?;
        if result.state != BriefcaseUploadState::Staged {
            return Err(invalid_response("byte transfer did not reach staged state"));
        }
        Ok(result)
    }

    /// Obtain a current commit token after transfer; transfer can outlive the reserve token.
    pub async fn commit_upload(
        &self,
        org_id: &str,
        app_id: &str,
        access: &OnBehalfOfGrant,
        operation_id: Uuid,
        upload_id: Uuid,
    ) -> ProviderResult<BriefcaseUploadStatus> {
        require_uuid(operation_id)?;
        require_uuid(upload_id)?;
        let request = self
            .control("/api/v1/obo/uploads/commit", org_id, app_id, access)?
            .json(&json!({"operation_id": operation_id, "upload_id": upload_id}));
        let result = self.response(request).await?;
        validate_status(&result, operation_id, Some(upload_id))?;
        Ok(result)
    }

    /// Status returns an entry UUID only. Resolve real provider metadata without inventing a path.
    pub async fn resolve_upload_entry(
        &self,
        org_id: &str,
        app_id: &str,
        access: &OnBehalfOfGrant,
        manifest: &BriefcaseUploadManifest,
        entry_id: Uuid,
    ) -> ProviderResult<BriefcaseEntry> {
        self.validate_manifest(manifest)?;
        require_uuid(entry_id)?;
        #[derive(Deserialize)]
        struct Page {
            items: Vec<serde_json::Value>,
            next_cursor: Option<String>,
        }
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        for _ in 0..100 {
            let mut body = json!({"limit": 100});
            if !manifest.parent_path.is_empty() {
                body["path"] = json!(manifest.parent_path);
            }
            if let Some(value) = &cursor {
                body["cursor"] = json!(value);
            }
            let page: Page =
                self.response(self.control("/api/v1/obo/entries/list", org_id, app_id, access)?.json(&body)).await?;
            if page.items.len() > 100 {
                return Err(invalid_response("entry page exceeds requested limit"));
            }
            for item in page.items {
                if item.get("id").and_then(|id| id.as_str()).and_then(|id| Uuid::parse_str(id).ok()) == Some(entry_id) {
                    let entry: BriefcaseEntry =
                        serde_json::from_value(item).map_err(|_| invalid_response("invalid entry metadata"))?;
                    validate_entry(&entry, org_id, manifest, entry_id)?;
                    return Ok(entry);
                }
            }
            cursor = page.next_cursor;
            let Some(next) = &cursor else {
                return Err(invalid_response("published upload entry was not found"));
            };
            if next.is_empty() || next.len() > 2048 || !seen.insert(next.clone()) {
                return Err(invalid_response("invalid entry pagination cursor"));
            }
        }
        Err(invalid_response("published entry exceeded the lookup page limit"))
    }

    fn validate_manifest(&self, manifest: &BriefcaseUploadManifest) -> ProviderResult<()> {
        require_uuid(manifest.operation_id)?;
        if manifest.size > self.max_upload_bytes as u64
            || manifest.name.is_empty()
            || manifest.name.len() > 255
            || manifest.name.contains(['/', '\\'])
            || matches!(manifest.name.as_str(), "." | "..")
            || manifest.name.chars().any(char::is_control)
            || manifest.content_type.is_empty()
            || manifest.content_type.len() > 255
            || manifest.content_type.chars().any(char::is_control)
            || manifest.sha256.len() != 64
            || !manifest.sha256.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || (!manifest.parent_path.is_empty() && !safe_path(&manifest.parent_path))
        {
            return Err(invalid("invalid Briefcase upload manifest"));
        }
        Ok(())
    }

    fn control(
        &self,
        path: &str,
        org: &str,
        app: &str,
        access: &OnBehalfOfGrant,
    ) -> ProviderResult<reqwest::RequestBuilder> {
        if silicon_browser_shared::app_id(app, "app_id").is_err()
            || access.expose().starts_with("obo_")
            || access.expose().starts_with("ort_")
        {
            return Err(invalid("Briefcase requires a canonical app ID and reusable OBO access token"));
        }
        Ok(self
            .plane(self.http.post(self.url(path)))
            .header("x-org-id", identifier_header(org)?)
            .header("x-app-id", identifier_header(app)?)
            .header("x-iam-obo-access-token", secret_header(access.expose())?))
    }

    fn plane(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.testing_key {
            Some(key) => request.header("x-briefcase-app-secret", key.clone()),
            None => request,
        }
    }

    fn url(&self, path: &str) -> Url {
        let mut url = self.endpoint.clone();
        url.set_path(path);
        url
    }

    async fn response<T: serde::de::DeserializeOwned>(&self, request: reqwest::RequestBuilder) -> ProviderResult<T> {
        let mut response = request.send().await.map_err(|error| transport(PROVIDER, error))?;
        let status = response.status();
        let retry_after = response.headers().get(reqwest::header::RETRY_AFTER).and_then(|header| {
            let value = header.to_str().ok()?;
            value.parse::<u64>().ok().map(Duration::from_secs).or_else(|| {
                let deadline = DateTime::parse_from_rfc2822(value).ok()?.with_timezone(&Utc);
                Some(Duration::from_secs((deadline - Utc::now()).num_seconds().max(0) as u64))
            })
        });
        if response.content_length().is_some_and(|length| length > MAX_RESPONSE_BYTES as u64) {
            return Err(invalid_response("provider response exceeded the safety limit"));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| transport(PROVIDER, error))? {
            if chunk.len() > MAX_RESPONSE_BYTES.saturating_sub(body.len()) {
                return Err(invalid_response("provider response exceeded the safety limit"));
            }
            body.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            // Preserve only an allowlisted authorization classification. ACL denials and
            // transient failures must not erase a still-valid grant; body text is never logged.
            let code = serde_json::from_slice::<serde_json::Value>(&body).ok();
            let code = code.as_ref().and_then(|value| value["error"]["code"].as_str());
            let authorization_required = matches!(status.as_u16(), 401 | 412)
                || (status.as_u16() == 403
                    && matches!(
                        code,
                        Some(
                            "obo_access_token_invalid"
                                | "obo_token_invalid"
                                | "obo_token_expired"
                                | "obo_token_revoked"
                                | "obo_consent_required"
                        )
                    ));
            return Err(ProviderError::Http {
                provider: PROVIDER,
                status: if authorization_required { 401 } else { status.as_u16() },
                message: "provider response body redacted".into(),
                retry_after,
            });
        }
        if status != reqwest::StatusCode::OK {
            return Err(invalid_response("unexpected delegated operation status"));
        }
        serde_json::from_slice(&body).map_err(|_| invalid_response("invalid delegated operation response"))
    }
}

fn validate_status(status: &BriefcaseUploadStatus, operation_id: Uuid, upload_id: Option<Uuid>) -> ProviderResult<()> {
    if status.operation_id != operation_id
        || status.operation_id.is_nil()
        || status.upload_id.is_nil()
        || upload_id.is_some_and(|expected| expected != status.upload_id)
        || status.published_entry_id.is_some_and(|id| id.is_nil())
        || (status.state == BriefcaseUploadState::Committed) != status.published_entry_id.is_some()
    {
        return Err(invalid_response("upload status did not match the logical operation"));
    }
    Ok(())
}

fn validate_entry(
    entry: &BriefcaseEntry,
    org: &str,
    manifest: &BriefcaseUploadManifest,
    entry_id: Uuid,
) -> ProviderResult<()> {
    if entry.id != entry_id
        || entry.org_id != org
        || entry.entry_type != "file"
        || entry.size != manifest.size
        || entry.name != manifest.name
        || entry.content_type.as_deref() != Some(manifest.content_type.as_str())
        || entry.path.rsplit('/').next() != Some(entry.name.as_str())
        || !safe_path(&entry.path)
        || (!manifest.parent_path.is_empty() && entry.path != format!("{}/{}", manifest.parent_path, manifest.name))
        || clean_url(&entry.permanent_url).is_err()
    {
        return Err(invalid_response("published entry did not match the upload contract"));
    }
    Ok(())
}

fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 2048
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && !path.split('/').any(|part| matches!(part, "" | "." | ".."))
}

fn clean_url(value: &str) -> ProviderResult<Url> {
    (value.len() <= MAX_URL_BYTES)
        .then(|| Url::parse(value).ok())
        .flatten()
        .filter(|url| {
            is_https_or_loopback_http(url)
                && url.host().is_some()
                && !url.cannot_be_a_base()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && url.port() != Some(0)
        })
        .ok_or_else(|| invalid("Briefcase URL must use HTTPS or loopback HTTP without credentials, query, or fragment"))
}

fn testing_header(value: &str) -> ProviderResult<HeaderValue> {
    if value.len() != 47
        || !value.starts_with("ask_")
        || !value[4..].bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(invalid("invalid Briefcase testing application secret"));
    }
    secret_header(value)
}

fn identifier_header(value: &str) -> ProviderResult<HeaderValue> {
    if value.is_empty() || value.len() > 255 || !value.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(invalid("invalid Briefcase organization or application identifier"));
    }
    HeaderValue::from_str(value).map_err(|_| invalid("invalid Briefcase identifier header"))
}

fn secret_header(value: &str) -> ProviderResult<HeaderValue> {
    if value.len() <= 4 || value.len() > 8192 || !value.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(invalid("invalid Briefcase credential"));
    }
    let mut header = HeaderValue::from_str(value).map_err(|_| invalid("invalid Briefcase secret header"))?;
    header.set_sensitive(true);
    Ok(header)
}

fn require_uuid(value: Uuid) -> ProviderResult<()> {
    if value.is_nil() { Err(invalid("logical operation and upload IDs must be non-nil")) } else { Ok(()) }
}

fn invalid(message: &str) -> ProviderError {
    ProviderError::InvalidInput(message.into())
}

fn invalid_response(message: &str) -> ProviderError {
    ProviderError::InvalidResponse { provider: PROVIDER, message: message.into() }
}

#[cfg(test)]
#[path = "briefcase_tests.rs"]
mod tests;
