//! Briefcase's single-use, request-bound OBO upload endpoint.
//!
//! Destination path, name, and media type must already be bound into the IAM
//! proof. This adapter sends only the exact raw bytes; it cannot select folders,
//! obtain a proof, or replay an uncertain upload. It is deliberately separate
//! from `ArtifactStore`, whose asynchronous delegation contract is not yet wired.

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use reqwest::header::HeaderValue;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

use super::artifact::OnBehalfOfGrant;
use super::error::{ProviderError, ProviderResult, json_response_with_limit, transport};
use crate::url_policy::is_https_or_loopback_http;

const PROVIDER: &str = "briefcase";
pub const BRIEFCASE_OBO_PATH: &str = "/api/v1/obo/files";
pub const BRIEFCASE_OBO_ENDPOINT_ID: &str = "briefcase.files.create";
/// Default bound for both byte and file uploads. Operators may explicitly
/// configure a different bound; Briefcase's own limit is independent of this one.
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
    /// accepted only for loopback development. A test key selects Briefcase's
    /// own sandbox, and is distinct from the paired IAM environment key.
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
                if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
                    return Err(invalid("invalid Briefcase testing environment key"));
                }
                secret_header(value)
            })
            .transpose()?;
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

    pub fn max_upload_bytes(&self) -> usize {
        self.max_upload_bytes
    }

    /// Compute the lowercase hexadecimal digest IAM must bind before upload.
    /// Callers must bind POST, `BRIEFCASE_OBO_PATH`, and the destination metadata
    /// while exchanging the proof, then pass these same unmodified bytes.
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

    /// Stream the same immutable open file previously hashed for this proof.
    /// An open handle avoids pathname substitution; callers must prevent writes.
    pub async fn upload_file(
        &self,
        org_id: &str,
        app_id: &str,
        proof: &OnBehalfOfGrant,
        mut file: tokio::fs::File,
        size: u64,
    ) -> ProviderResult<BriefcaseEntry> {
        let metadata = file.metadata().await.map_err(|_| invalid("could not inspect staging file"))?;
        if !metadata.is_file() || metadata.len() != size || size > self.max_upload_bytes as u64 {
            return Err(invalid("staging file does not match the declared bounded upload size"));
        }
        file.rewind().await.map_err(|_| invalid("could not rewind staging file"))?;
        self.upload_body(org_id, app_id, proof, reqwest::Body::from(file), size).await
    }

    /// Make exactly one request with a supplied proof. A transport error or
    /// malformed success response can follow a committed write. Do not replay
    /// this single-use proof. A caller choosing a fresh-proof retry must accept
    /// that it may publish another version of an already committed file.
    pub async fn upload_raw(
        &self,
        org_id: &str,
        app_id: &str,
        proof: &OnBehalfOfGrant,
        bytes: Vec<u8>,
    ) -> ProviderResult<BriefcaseEntry> {
        self.validate_size(bytes.len())?;
        let size = bytes.len() as u64;
        self.upload_body(org_id, app_id, proof, reqwest::Body::from(bytes), size).await
    }

    async fn upload_body(
        &self,
        org_id: &str,
        app_id: &str,
        proof: &OnBehalfOfGrant,
        body: reqwest::Body,
        size: u64,
    ) -> ProviderResult<BriefcaseEntry> {
        let organization = identifier_header(org_id)?;
        let application = identifier_header(app_id)?;
        let Some((app_org, app_name)) = app_id.split_once('>') else {
            return Err(invalid("Briefcase OBO requires a canonical org>application ID"));
        };
        if app_org != org_id || app_name.is_empty() || app_name.contains('>') {
            return Err(invalid("Briefcase OBO application must belong to the requested organization"));
        }
        let proof_value = proof.expose();
        if !proof_value.starts_with("obo_")
            || proof_value.len() <= 4
            || proof_value.len() > 8192
            || !proof_value.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(invalid("invalid Briefcase OBO proof"));
        }
        let mut request = self
            .http
            .post(self.endpoint.clone())
            .header("x-org-id", organization)
            .header("x-app-id", application)
            .header("x-iam-obo-access-proof", secret_header(proof_value)?)
            .header("content-type", "application/octet-stream")
            .header(reqwest::header::CONTENT_LENGTH, size)
            .body(body);
        if let Some(key) = &self.testing_key {
            request = request.header("x-testing-environment-key", key.clone());
        }
        let response = request.send().await.map_err(|error| transport(PROVIDER, error))?;
        let status = response.status();
        let entry: BriefcaseEntry = json_response_with_limit(PROVIDER, response, MAX_RESPONSE_BYTES).await?;
        if status != reqwest::StatusCode::CREATED
            || entry.org_id != org_id
            || entry.entry_type != "file"
            || entry.size != size
            || entry.name.is_empty()
            || entry.name.len() > 255
            || entry.path.rsplit('/').next() != Some(entry.name.as_str())
            || entry.path.is_empty()
            || entry.path.len() > MAX_URL_BYTES
            || entry.path.starts_with('/')
            || entry.path.split('/').any(|part| matches!(part, "" | "." | ".."))
            || entry.name.chars().any(char::is_control)
            || entry.path.chars().any(char::is_control)
            || clean_url(&entry.permanent_url).is_err()
        {
            return Err(ProviderError::InvalidResponse {
                provider: PROVIDER,
                message: "created entry did not match the upload contract".into(),
            });
        }
        Ok(entry)
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

fn identifier_header(value: &str) -> ProviderResult<HeaderValue> {
    if value.is_empty() || value.len() > 255 || !value.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(invalid("invalid Briefcase organization or application identifier"));
    }
    HeaderValue::from_str(value).map_err(|_| invalid("invalid Briefcase identifier header"))
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
mod tests {
    use super::*;
    use crate::providers::test_http::{spawn_header_only_server, spawn_json_server};
    use serde_json::json;

    fn proof() -> OnBehalfOfGrant {
        OnBehalfOfGrant::new("obo_request-bound-secret").unwrap()
    }

    fn entry(size: usize) -> serde_json::Value {
        json!({
            "id":"018f156c-0276-7000-8000-000000000001", "org_id":"test-org", "type":"file",
            "name":"recording.bin", "path":"private/actor/apps/test-org>browser/recording.bin",
            "content_type":"video/mp4", "size":size,
            "permanent_url":"https://briefcase.example/test-org/private/actor/apps/test-org%3Ebrowser/recording.bin",
            "origin_app_id":"test-org>browser"
        })
    }

    #[tokio::test]
    async fn file_upload_streams_over_100_mib_from_the_same_hashed_handle() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let staged = tempfile::NamedTempFile::new().unwrap();
        let size = 101 * 1024 * 1024;
        staged.as_file().set_len(size).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = BriefcaseClient::with_upload_limit(
            &format!("http://{}", listener.local_addr().unwrap()),
            None,
            size as usize,
        )
        .unwrap();
        let mut file = tokio::fs::File::from_std(staged.reopen().unwrap());
        let (expected_digest, hashed_size) = client.hash_file(&mut file).await.unwrap();
        assert_eq!(hashed_size, size);
        // Removing the pathname cannot redirect the upload to another file.
        staged.close().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                head.push(socket.read_u8().await.unwrap());
                assert!(head.len() < 16384);
            }
            let head = String::from_utf8(head).unwrap().to_lowercase();
            assert!(head.contains(&format!("content-length: {size}\r\n")));
            let mut digest = Sha256::new();
            let mut remaining = size;
            let mut buffer = [0u8; 64 * 1024];
            while remaining > 0 {
                let bound = remaining.min(buffer.len() as u64) as usize;
                let count = socket.read(&mut buffer[..bound]).await.unwrap();
                assert!(count > 0);
                digest.update(&buffer[..count]);
                remaining -= count as u64;
            }
            assert_eq!(hex::encode(digest.finalize()), expected_digest);
            let body = entry(size as usize).to_string();
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 201 Created\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        assert_eq!(client.upload_file("test-org", "test-org>browser", &proof(), file, size).await.unwrap().size, size);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn file_upload_rejects_changed_size_before_network() {
        let staged = tempfile::NamedTempFile::new().unwrap();
        staged.as_file().set_len(3).unwrap();
        let client = BriefcaseClient::new("http://127.0.0.1:1", None).unwrap();
        let mut file = tokio::fs::File::from_std(staged.reopen().unwrap());
        let (_, size) = client.hash_file(&mut file).await.unwrap();
        staged.as_file().set_len(4).unwrap();
        assert!(matches!(
            client.upload_file("test-org", "test-org>browser", &proof(), file, size).await,
            Err(ProviderError::InvalidInput(_))
        ));
    }

    #[tokio::test]
    async fn upload_sends_bound_raw_bytes_and_only_obo_credentials_in_selected_plane() {
        for testing_key in [None, Some("0123456789abcdefghijklmnopqrstuv")] {
            let bytes = b"\0\xffraw\n".to_vec();
            let (base, mut requests, server) = spawn_json_server(vec![(201, entry(bytes.len()).to_string())]).await;
            let client = BriefcaseClient::new(&base, testing_key).unwrap();
            assert_eq!(
                client.body_sha256(b"abc").unwrap(),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            );
            let result = client.upload_raw("test-org", "test-org>browser", &proof(), bytes.clone()).await.unwrap();
            assert_eq!(result.size, bytes.len() as u64);
            let request = requests.recv().await.unwrap();
            server.await.unwrap();
            assert_eq!(request.method, "POST");
            assert_eq!(request.target, BRIEFCASE_OBO_PATH);
            assert_eq!(request.body, bytes);
            let headers = request.headers.to_ascii_lowercase();
            assert!(headers.contains(concat!("user-agent: silicon-browser/", env!("CARGO_PKG_VERSION"))));
            for expected in [
                "x-app-id: test-org>browser",
                "x-org-id: test-org",
                "x-iam-obo-access-proof: obo_request-bound-secret",
                "content-type: application/octet-stream",
            ] {
                assert!(headers.contains(expected), "missing expected header");
            }
            for forbidden in ["authorization:", "idempotency-key:", "content-digest:", "x-path:"] {
                assert!(!headers.contains(forbidden));
            }
            assert_eq!(headers.contains("x-testing-environment-key:"), testing_key.is_some());
            if let Some(key) = testing_key {
                assert!(headers.contains(&format!("x-testing-environment-key: {key}")));
                assert!(!format!("{client:?}").contains(key));
            }
        }
    }

    #[tokio::test]
    async fn rejects_unsafe_configuration_credentials_and_oversized_bytes_before_upload() {
        for origin in [
            "http://remote.example",
            "https://user:secret@example.com",
            "https://example.com?secret=yes",
            "https://example.com/#fragment",
            "https://example.com:0",
            "https://example.com/api/v1",
            &format!("https://example.com/{}", "x".repeat(MAX_URL_BYTES)),
        ] {
            assert!(BriefcaseClient::new(origin, None).is_err());
        }
        for key in ["", "short", "0123456789abcdefghijklmnopqrstu\n"] {
            assert!(BriefcaseClient::new("http://127.0.0.1:1", Some(key)).is_err());
        }
        assert!(BriefcaseClient::with_upload_limit("http://127.0.0.1:1", None, 0).is_err());
        let client = BriefcaseClient::with_upload_limit("http://127.0.0.1:1", None, 3).unwrap();
        assert_eq!(client.max_upload_bytes(), 3);
        assert!(client.body_sha256(b"four").is_err());
        let error = client.upload_raw("test-org", "test-org>browser", &proof(), b"four".to_vec()).await.unwrap_err();
        assert!(matches!(error, ProviderError::InvalidInput(_)));
        for (org, app, grant) in [
            ("test-org", "other-org>browser", proof()),
            ("test-org", "browser", proof()),
            ("test-org\r\n", "test-org>browser", proof()),
            ("test-org", "test-org>browser", OnBehalfOfGrant::new("invalid-proof-secret").unwrap()),
        ] {
            assert!(matches!(client.upload_raw(org, app, &grant, vec![]).await, Err(ProviderError::InvalidInput(_))));
        }
    }

    #[tokio::test]
    async fn errors_are_redacted_and_redirects_never_replay_single_use_proofs() {
        let (base, _requests, server) =
            spawn_json_server(vec![(403, "obo_request-bound-secret upstream-private-detail".into())]).await;
        let client = BriefcaseClient::new(&base, None).unwrap();
        let error = client.upload_raw("test-org", "test-org>browser", &proof(), vec![]).await.unwrap_err();
        server.await.unwrap();
        assert!(matches!(error, ProviderError::Http { status: 403, .. }));
        assert!(!format!("{error:?}").contains("upstream-private-detail"));
        assert!(!error.to_string().contains("obo_request-bound-secret"));
        let (base, server) =
            spawn_header_only_server(307, vec![("Location".into(), "http://127.0.0.1:1/proof-must-not-follow".into())])
                .await;
        let client = BriefcaseClient::new(&base, None).unwrap();
        let error = client.upload_raw("test-org", "test-org>browser", &proof(), vec![]).await.unwrap_err();
        server.await.unwrap();
        assert!(matches!(error, ProviderError::Http { status: 307, .. }));
    }

    #[tokio::test]
    async fn authorized_overwrite_preserves_original_creator_metadata() {
        for origin in [None, Some("test-org>other")] {
            let bytes = b"replacement content".to_vec();
            let mut response = entry(bytes.len());
            response["origin_app_id"] = json!(origin);
            let (base, mut requests, server) = spawn_json_server(vec![(201, response.to_string())]).await;
            let client = BriefcaseClient::new(&base, None).unwrap();
            let uploaded = client.upload_raw("test-org", "test-org>browser", &proof(), bytes.clone()).await.unwrap();
            let request = requests.recv().await.unwrap();
            server.await.unwrap();
            assert_eq!(uploaded.origin_app_id.as_deref(), origin);
            assert_eq!(uploaded.org_id, "test-org");
            assert_eq!(uploaded.size, bytes.len() as u64);
            assert_eq!(request.body, bytes);
            assert!(request.headers.contains("x-app-id: test-org>browser"));
        }
    }

    #[tokio::test]
    async fn responses_are_bounded_and_must_match_created_file_identity_and_size() {
        let (base, server) =
            spawn_header_only_server(201, vec![("Content-Length".into(), (MAX_RESPONSE_BYTES + 1).to_string())]).await;
        let client = BriefcaseClient::new(&base, None).unwrap();
        let error = client.upload_raw("test-org", "test-org>browser", &proof(), vec![]).await.unwrap_err();
        server.await.unwrap();
        assert!(matches!(error, ProviderError::InvalidResponse { .. }));
        for (field, value) in [
            ("org_id", json!("another-org")),
            ("type", json!("folder")),
            ("size", json!(99)),
            ("permanent_url", json!("https://briefcase.example/entry?token=hidden")),
        ] {
            let mut response = entry(0);
            response[field] = value;
            let (base, _requests, server) = spawn_json_server(vec![(201, response.to_string())]).await;
            let client = BriefcaseClient::new(&base, None).unwrap();
            let error = client.upload_raw("test-org", "test-org>browser", &proof(), vec![]).await.unwrap_err();
            server.await.unwrap();
            assert!(matches!(error, ProviderError::InvalidResponse { .. }), "field {field}");
            assert!(!error.to_string().contains("hidden"));
        }
    }
}
