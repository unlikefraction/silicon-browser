//! Briefcase recording publication using separately approved reusable OBO tokens.
//! Reservation binds exact metadata and bytes. The staging capability transfers
//! bytes without IAM credentials, and commit rechecks current provider authority.

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use reqwest::header::HeaderValue;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

use super::artifact::OnBehalfOfGrant;
use super::error::{ProviderError, ProviderResult, transport};
use crate::url_policy::is_https_or_loopback_http;

const PROVIDER: &str = "briefcase";
pub const BRIEFCASE_OBO_PATH: &str = "/api/v1/obo/files";
pub const BRIEFCASE_OBO_ENDPOINT_ID: &str = "briefcase.files.create";
/// Default bound for both byte and file uploads. Operators may explicitly
/// configure a different bound; Briefcase's own limit is independent of this one.
pub const DEFAULT_BRIEFCASE_UPLOAD_LIMIT: usize = 64 * 1024 * 1024;
const MAX_URL_BYTES: usize = 4096;

#[derive(Clone)]
pub struct BriefcaseClient {
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

/// Public entry metadata returned by a completed OBO upload. The permanent URL
/// is an authenticated entry link, not a signed object-download URL.
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
    /// Original creator provenance, preserved across later authorized uploads.
    /// A member-created file has no originating application.
    pub origin_app_id: Option<String>,
}

impl BriefcaseClient {
    /// `origin` must be a root HTTP(S) origin, not `/api/v1`. Plain HTTP is
    /// accepted only for loopback development. Testing requires the imported
    /// Briefcase IAM app secret (`ask_` plus 43 base64url characters), distinct
    /// from Browser's app secret and the 32-character IAM environment root key.
    pub fn new(origin: &str, testing_key: Option<&str>) -> ProviderResult<Self> {
        Self::with_upload_limit(origin, testing_key, DEFAULT_BRIEFCASE_UPLOAD_LIMIT)
    }

    pub fn with_upload_limit(origin: &str, testing_key: Option<&str>, max_upload_bytes: usize) -> ProviderResult<Self> {
        if max_upload_bytes == 0 {
            return Err(invalid("Briefcase upload limit must be positive"));
        }
        let mut endpoint = clean_url(origin)?;
        if endpoint.path() != "/" {
            return Err(invalid("Briefcase URL must be a root origin without an API path"));
        }
        endpoint.set_path(BRIEFCASE_OBO_PATH);
        let testing_key = testing_key
            .map(|value| {
                if value.len() != 47
                    || !value.starts_with("ask_")
                    || !value[4..].bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                {
                    return Err(invalid("invalid Briefcase testing application secret"));
                }
                secret_header(value)
            })
            .transpose()?;
        let _http = reqwest::Client::builder()
            .user_agent(concat!("silicon-browser/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map_err(|error| transport(PROVIDER, error))?;
        Ok(Self { endpoint, testing_key, max_upload_bytes })
    }

    pub fn max_upload_bytes(&self) -> usize {
        self.max_upload_bytes
    }

    /// Compute the digest bound by the Briefcase upload reservation.
    pub fn body_sha256(&self, bytes: &[u8]) -> ProviderResult<String> {
        self.validate_size(bytes.len())?;
        Ok(hex::encode(Sha256::digest(bytes)))
    }

    /// Hash a private immutable staging file with bounded memory, then rewind
    /// the same open handle for upload. Never modify the file after hashing.
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

    /// Publish exact staged bytes under a stable reservation, then read the committed receipt.
    #[allow(
        clippy::too_many_arguments,
        reason = "checks the exact immutable artifact fields and selected authority together"
    )]
    pub async fn upload_recording(
        &self,
        app_id: &str,
        operation_id: Uuid,
        tokens: &crate::delivery_auth::obo::RecordingTokens,
        name: &str,
        content_type: &str,
        digest: &str,
        file: tokio::fs::File,
        size: u64,
    ) -> ProviderResult<BriefcaseEntry> {
        use briefcase_client::{
            ApplicationId, Client, Config, DelegatedCommitUpload, DelegatedListEntries, DelegatedReserveUpload,
            DelegatedUploadState, EnvironmentKey, OboProof,
        };
        use secrecy::ExposeSecret as _;
        if size > self.max_upload_bytes as u64
            || silicon_browser_shared::app_id(app_id, "app_id").is_err()
            || tokens.expires_at <= chrono::Utc::now()
        {
            return Err(invalid("invalid recording upload authority or size"));
        }
        let metadata = file.metadata().await.map_err(|_| invalid("could not inspect staged recording"))?;
        if !metadata.is_file() || metadata.len() != size {
            return Err(invalid("staged recording length changed"));
        }
        let mut base = self.endpoint.clone();
        base.set_path("/api/v1/");
        let mut config = Config::new(base.as_str(), &tokens.org_id)
            .map_err(|_| invalid("invalid Briefcase configuration"))?
            .with_auto_update(false)
            .with_transfer_timeout(Duration::from_secs(120));
        if let Some(key) = &tokens.testing_secret {
            config = config.with_environment(
                EnvironmentKey::new(key.expose_secret()).map_err(|_| invalid("invalid selected testing context"))?,
            );
        } else if self.testing_key.is_some() {
            return Err(invalid("missing testing grant context"));
        }
        let client = Client::connect(config).await.map_err(sdk_error)?;
        let app = ApplicationId::new(app_id).map_err(sdk_error)?;
        let token = |token: &OnBehalfOfGrant| OboProof::new(token.expose()).map_err(sdk_error);
        let reserve = DelegatedReserveUpload {
            operation_id,
            parent_path: String::new(),
            name: name.into(),
            content_type: content_type.into(),
            size,
            sha256: digest.into(),
        }
        .prepare()
        .map_err(sdk_error)?;
        let reserved =
            client.reserve_delegated_upload(&app, token(&tokens.reserve)?, &reserve).await.map_err(sdk_error)?;
        if reserved.status.operation_id != operation_id || reserved.status.upload_id.is_nil() {
            return Err(invalid_receipt());
        }
        let upload_id = reserved.status.upload_id;
        let staged = match reserved.status.state {
            DelegatedUploadState::Reserved => client
                .transfer_delegated_upload_file(upload_id, reserved.capability.ok_or_else(invalid_receipt)?, file)
                .await
                .map_err(sdk_error)?,
            DelegatedUploadState::Staged | DelegatedUploadState::Committed => reserved.status,
            _ => return Err(invalid_receipt()),
        };
        if staged.operation_id != operation_id
            || staged.upload_id != upload_id
            || !matches!(staged.state, DelegatedUploadState::Staged | DelegatedUploadState::Committed)
        {
            return Err(invalid_receipt());
        }
        let committed = if staged.state == DelegatedUploadState::Committed {
            staged
        } else {
            client
                .commit_delegated_upload(
                    &app,
                    token(&tokens.commit)?,
                    &DelegatedCommitUpload { operation_id, upload_id }.prepare().map_err(sdk_error)?,
                )
                .await
                .map_err(sdk_error)?
        };
        if committed.operation_id != operation_id
            || committed.upload_id != upload_id
            || committed.state != DelegatedUploadState::Committed
        {
            return Err(invalid_receipt());
        }
        let published = committed.published_entry_id.ok_or_else(invalid_receipt)?;
        let parent = format!("apps/{app_id}/private/{}", tokens.actor_id);
        let mut cursor = None;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            let page = client
                .list_entries_on_behalf_of(
                    &app,
                    token(&tokens.list)?,
                    &DelegatedListEntries {
                        path: Some(parent.clone()),
                        cursor,
                        limit: Some(100),
                        ..Default::default()
                    }
                    .prepare()
                    .map_err(sdk_error)?,
                )
                .await
                .map_err(sdk_error)?;
            if let Some(entry) = page.items.into_iter().find(|entry| entry.id == published) {
                if entry.org_id != tokens.org_id
                    || entry.path != format!("{parent}/{name}")
                    || entry.name != name
                    || entry.origin_app_id.as_deref() != Some(app_id)
                    || entry.size != Some(size)
                    || entry.content_type.as_deref() != Some(content_type)
                    || clean_url(entry.permanent_url.as_str()).is_err()
                {
                    return Err(invalid_receipt());
                }
                return Ok(BriefcaseEntry {
                    id: entry.id,
                    org_id: entry.org_id,
                    entry_type: "file".into(),
                    name: entry.name,
                    path: entry.path,
                    content_type: entry.content_type,
                    size,
                    permanent_url: entry.permanent_url.to_string(),
                    origin_app_id: entry.origin_app_id,
                });
            }
            match page.next_cursor {
                Some(next) if seen.insert(next.clone()) => cursor = Some(next),
                None => break,
                _ => return Err(invalid_receipt()),
            }
        }
        Err(invalid_receipt())
    }

    /// Retired raw proof API. Call `upload_recording` with separately approved tokens.
    pub async fn upload_file(
        &self,
        _org_id: &str,
        _app_id: &str,
        _proof: &OnBehalfOfGrant,
        _file: tokio::fs::File,
        _size: u64,
    ) -> ProviderResult<BriefcaseEntry> {
        Err(invalid("raw OBO upload retired; reserve, transfer, then commit"))
    }
    /// Retired raw proof API. No credentials or bytes are sent.
    pub async fn upload_raw(
        &self,
        _org_id: &str,
        _app_id: &str,
        _proof: &OnBehalfOfGrant,
        _bytes: Vec<u8>,
    ) -> ProviderResult<BriefcaseEntry> {
        Err(invalid("raw OBO upload retired; reserve, transfer, then commit"))
    }

    fn validate_size(&self, size: usize) -> ProviderResult<()> {
        if size > self.max_upload_bytes {
            return Err(invalid(&format!(
                "Briefcase raw upload exceeds the configured {}-byte limit",
                self.max_upload_bytes
            )));
        }
        Ok(())
    }
}

fn invalid_receipt() -> ProviderError {
    ProviderError::InvalidResponse {
        provider: PROVIDER,
        message: "recording receipt disagreed with approved upload".into(),
    }
}
fn sdk_error(error: briefcase_client::Error) -> ProviderError {
    // A resource ACL denial is not a revoked grant. Only explicit consent/token
    // failures (or a changed consent graph) invite the user to authorize again.
    if matches!(&error, briefcase_client::Error::Api(api) if matches!(api.status, 401 | 412)
        || (api.status == 403 && matches!(api.code.as_str(),
            "obo_access_token_invalid" | "obo_token_invalid" | "obo_token_expired" | "obo_token_revoked" | "obo_consent_required")))
    {
        ProviderError::Http {
            provider: PROVIDER,
            status: 401,
            message: "recording authorization required".into(),
            retry_after: None,
        }
    } else {
        ProviderError::Transport { provider: PROVIDER, message: "Briefcase request could not be confirmed".into() }
    }
}

fn clean_url(value: &str) -> ProviderResult<Url> {
    let url = (value.len() <= MAX_URL_BYTES)
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
        .ok_or_else(|| {
            invalid("Briefcase URL must use HTTPS or loopback HTTP without credentials, query, or fragment")
        })?;
    Ok(url)
}

fn secret_header(value: &str) -> ProviderResult<HeaderValue> {
    let mut header = HeaderValue::from_str(value).map_err(|_| invalid("invalid Briefcase secret header"))?;
    header.set_sensitive(true);
    Ok(header)
}

fn invalid(message: &str) -> ProviderError {
    ProviderError::InvalidInput(message.into())
}

#[cfg(test)]
#[path = "briefcase_tests.rs"]
mod tests;
