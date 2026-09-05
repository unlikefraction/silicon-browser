//! Prepare existing native recordings and command logs for request-bound Briefcase delivery.
//!
//! Durable claiming, cancellation, retry identity, and receipt persistence belong to the caller.
//! Preparation happens before proof issuance. Upload never retries a proof itself.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use silicon_browser_shared::SessionLog;
use tokio::io::AsyncWriteExt;
use url::Url;

use crate::{
    providers::{BriefcaseClient, BriefcaseEntry, BrowserProvider, OnBehalfOfGrant, ProviderError},
    url_policy::has_forbidden_host,
};

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_SOURCE_URL_BYTES: usize = 16 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    #[error("native recording is not ready yet")]
    NotReady,
    #[error("native recording is permanently unavailable")]
    Unavailable,
    #[error("native recording source failed validation")]
    InvalidSource,
    #[error("artifact exceeds the configured upload size limit")]
    TooLarge,
    #[error("could not stage recording delivery artifact")]
    Io,
    #[error(transparent)]
    Provider(#[from] ProviderError),
}

/// An anonymous private file; the only handle remains owned by this artifact until upload.
/// Persist the digest and size with the attempt before issuing a body-bound proof.
#[derive(Debug)]
pub struct StagedArtifact {
    pub name: String,
    pub content_type: &'static str,
    pub body_sha256: String,
    pub size: u64,
    file: tokio::fs::File,
}

pub struct RecordingDelivery {
    briefcase: BriefcaseClient,
    browser: Arc<dyn BrowserProvider>,
    #[cfg(test)]
    local_source_origin: Option<String>,
}

impl RecordingDelivery {
    pub fn new(briefcase: BriefcaseClient, browser: Arc<dyn BrowserProvider>) -> Self {
        Self {
            briefcase,
            browser,
            #[cfg(test)]
            local_source_origin: None,
        }
    }

    /// Explicit loopback fixture allowance, compiled only into unit tests.
    #[cfg(test)]
    pub(crate) fn with_test_source_origin(mut self, origin: &str) -> Self {
        let url = Url::parse(origin).expect("test source origin");
        assert!(url.host_str().is_some_and(|host| host == "127.0.0.1" || host == "localhost" || host == "[::1]"));
        self.local_source_origin = Some(url.origin().ascii_serialization());
        self
    }

    pub async fn prepare_video(&self, session_id: &str, provider_id: &str) -> Result<StagedArtifact, DeliveryError> {
        validate_session_id(session_id)?;
        let browser = self.browser.get_browser(provider_id).await?;
        if browser.id != provider_id {
            return Err(DeliveryError::InvalidSource);
        }
        if browser.status != "stopped" {
            return Err(DeliveryError::NotReady);
        }
        // A returned URL takes precedence over a contradictory terminal readiness flag.
        let Some(source) = browser.recording_url else {
            return Err(if browser.recording_available == Some(false) {
                DeliveryError::Unavailable
            } else {
                DeliveryError::NotReady
            });
        };
        let file = tokio::time::timeout(DOWNLOAD_TIMEOUT, self.download(&source)).await.map_err(|_| {
            DeliveryError::Provider(ProviderError::Transport {
                provider: "recording-download",
                message: "download deadline exceeded".into(),
            })
        })??;
        self.finish_staging(format!("{session_id}.mp4"), "video/mp4", file).await
    }

    pub async fn prepare_log(&self, session_id: &str, logs: &[SessionLog]) -> Result<StagedArtifact, DeliveryError> {
        let mut pages = logs.chunks(128);
        self.prepare_log_pages(session_id, |_| std::future::ready(Ok(pages.next().unwrap_or_default().to_vec()))).await
    }

    /// Load at most 128 ordered entries per page; an empty page ends the snapshot.
    /// The caller must snapshot/freeze terminal commands across retries.
    pub async fn prepare_log_pages<F, Fut>(
        &self,
        session_id: &str,
        mut loader: F,
    ) -> Result<StagedArtifact, DeliveryError>
    where
        F: FnMut(u64) -> Fut,
        Fut: std::future::Future<Output = Result<Vec<SessionLog>, DeliveryError>>,
    {
        validate_session_id(session_id)?;
        let mut file = private_file()?;
        let mut total = 0usize;
        let mut after_sequence = 0u64;
        loop {
            let page = loader(after_sequence).await?;
            if page.is_empty() {
                break;
            }
            if page.len() > 128 {
                return Err(DeliveryError::InvalidSource);
            }
            for log in page {
                if log.sequence <= after_sequence {
                    return Err(DeliveryError::InvalidSource);
                }
                after_sequence = log.sequence;
                // Allocate one bounded log entry, never the complete session log.
                if log.command.len() > self.briefcase.max_upload_bytes() {
                    return Err(DeliveryError::TooLarge);
                }
                let line = serde_json::to_vec(&log).map_err(|_| DeliveryError::InvalidSource)?;
                total = total.checked_add(line.len()).and_then(|n| n.checked_add(1)).ok_or(DeliveryError::TooLarge)?;
                if total > self.briefcase.max_upload_bytes() {
                    return Err(DeliveryError::TooLarge);
                }
                file.write_all(&line).await.map_err(|_| DeliveryError::Io)?;
                file.write_all(b"\n").await.map_err(|_| DeliveryError::Io)?;
            }
        }
        self.finish_staging(format!("{session_id}-commands.jsonl"), "application/x-ndjson", file).await
    }

    pub async fn upload(
        &self,
        artifact: StagedArtifact,
        proof: &OnBehalfOfGrant,
        org_id: &str,
        app_id: &str,
    ) -> Result<BriefcaseEntry, DeliveryError> {
        Ok(self.briefcase.upload_file(org_id, app_id, proof, artifact.file, artifact.size).await?)
    }

    async fn finish_staging(
        &self,
        name: String,
        content_type: &'static str,
        mut file: tokio::fs::File,
    ) -> Result<StagedArtifact, DeliveryError> {
        file.flush().await.map_err(|_| DeliveryError::Io)?;
        let (body_sha256, size) = self.briefcase.hash_file(&mut file).await?;
        Ok(StagedArtifact { name, content_type, body_sha256, size, file })
    }

    async fn download(&self, raw: &str) -> Result<tokio::fs::File, DeliveryError> {
        let url = self.source_url(raw)?;
        let host = url.host_str().ok_or(DeliveryError::InvalidSource)?;
        let addresses = self.validate_resolved_addresses(
            tokio::net::lookup_host((
                host.trim_matches(['[', ']']),
                url.port_or_known_default().ok_or(DeliveryError::InvalidSource)?,
            ))
            .await
            .map(|addresses| addresses.collect()),
        )?;
        // Resolve exactly once, reject all private results, and pin the addresses used by the
        // connection. Redirects and ambient proxy configuration cannot bypass this check.
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .timeout(DOWNLOAD_TIMEOUT)
            .resolve_to_addrs(host, &addresses)
            .build()
            .map_err(download_error)?;
        let mut response = http.get(url).send().await.map_err(download_error)?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(DeliveryError::Provider(ProviderError::Http {
                provider: "recording-download",
                status: response.status().as_u16(),
                message: "response body redacted".into(),
                retry_after: None,
            }));
        }
        let max = self.briefcase.max_upload_bytes() as u64;
        if response.content_length().is_some_and(|size| size > max) {
            return Err(DeliveryError::TooLarge);
        }
        let mut file = private_file()?;
        let mut size = 0u64;
        let mut signature: Vec<u8> = Vec::with_capacity(12);
        while let Some(chunk) = response.chunk().await.map_err(download_error)? {
            size = size.checked_add(chunk.len() as u64).ok_or(DeliveryError::TooLarge)?;
            if size > max {
                return Err(DeliveryError::TooLarge);
            }
            signature.extend(chunk.iter().take(12usize.saturating_sub(signature.len())));
            file.write_all(&chunk).await.map_err(|_| DeliveryError::Io)?;
        }
        // Native MP4 begins with an ISO BMFF file-type box. Reject HTML/error payloads
        // masquerading as a successful download before binding or uploading them.
        if signature.len() < 12 || &signature[4..8] != b"ftyp" {
            return Err(DeliveryError::InvalidSource);
        }
        Ok(file)
    }

    fn source_url(&self, raw: &str) -> Result<Url, DeliveryError> {
        if raw.len() > MAX_SOURCE_URL_BYTES {
            return Err(DeliveryError::InvalidSource);
        }
        let url = Url::parse(raw).map_err(|_| DeliveryError::InvalidSource)?;
        if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() || url.host().is_none() {
            return Err(DeliveryError::InvalidSource);
        }
        #[cfg(test)]
        if self.local_source_origin.as_ref().is_some_and(|origin| url.origin().ascii_serialization() == *origin) {
            return Ok(url);
        }
        if url.scheme() != "https" || has_forbidden_host(&url) {
            return Err(DeliveryError::InvalidSource);
        }
        Ok(url)
    }

    fn validate_resolved_addresses(
        &self,
        result: std::io::Result<Vec<SocketAddr>>,
    ) -> Result<Vec<SocketAddr>, DeliveryError> {
        let resolution_failed = || {
            DeliveryError::Provider(ProviderError::Transport {
                provider: "recording-download",
                message: "source DNS resolution unavailable".into(),
            })
        };
        let addresses = result.map_err(|_| resolution_failed())?;
        if addresses.is_empty() {
            return Err(resolution_failed());
        }
        if addresses.iter().any(|address| !self.address_allowed(address)) {
            return Err(DeliveryError::InvalidSource);
        }
        Ok(addresses)
    }

    fn address_allowed(&self, address: &SocketAddr) -> bool {
        #[cfg(test)]
        if self.local_source_origin.is_some() && address.ip().is_loopback() {
            return true;
        }
        let Ok(url) = Url::parse(&format!("https://{address}/")) else { return false };
        !has_forbidden_host(&url)
    }
}

fn private_file() -> Result<tokio::fs::File, DeliveryError> {
    tempfile::tempfile().map(tokio::fs::File::from_std).map_err(|_| DeliveryError::Io)
}

fn validate_session_id(session_id: &str) -> Result<(), DeliveryError> {
    if session_id.is_empty()
        || session_id.len() > 128
        || !session_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(DeliveryError::InvalidSource);
    }
    Ok(())
}

fn download_error(error: reqwest::Error) -> DeliveryError {
    DeliveryError::Provider(ProviderError::Transport {
        provider: "recording-download",
        message: error.without_url().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{
        CreateBrowserProfile, ProviderBrowserSession, ProviderProfile, ProviderResult, StartBrowser,
        UpdateBrowserProfile,
    };
    use async_trait::async_trait;
    use sha2::{Digest, Sha256};
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    struct Browser(ProviderBrowserSession);
    #[async_trait]
    impl BrowserProvider for Browser {
        async fn get_browser(&self, _: &str) -> ProviderResult<ProviderBrowserSession> {
            Ok(self.0.clone())
        }
        async fn create_profile(&self, _: CreateBrowserProfile) -> ProviderResult<ProviderProfile> {
            unreachable!()
        }
        async fn update_profile(&self, _: &str, _: UpdateBrowserProfile) -> ProviderResult<ProviderProfile> {
            unreachable!()
        }
        async fn start_browser(&self, _: StartBrowser) -> ProviderResult<ProviderBrowserSession> {
            unreachable!()
        }
        async fn stop_browser(&self, _: &str) -> ProviderResult<ProviderBrowserSession> {
            unreachable!()
        }
        async fn find_profile_by_user_id(&self, _: &str) -> ProviderResult<Option<ProviderProfile>> {
            unreachable!()
        }
        async fn find_browser_by_session_id(&self, _: &str) -> ProviderResult<Option<ProviderBrowserSession>> {
            unreachable!()
        }
    }

    fn worker(value: serde_json::Value, limit: usize) -> RecordingDelivery {
        RecordingDelivery::new(
            BriefcaseClient::with_upload_limit("http://127.0.0.1:1", None, limit).unwrap(),
            Arc::new(Browser(serde_json::from_value(value).unwrap())),
        )
    }

    fn idle_worker(limit: usize) -> RecordingDelivery {
        worker(serde_json::json!({"id":"remote", "status":"stopped"}), limit)
    }

    async fn source(body: &[u8], status: u16, extra: &str, chunked: bool) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let mut response = format!("HTTP/1.1 {status} Test\r\nConnection: close\r\n{extra}");
        if chunked {
            response.push_str("Transfer-Encoding: chunked\r\n\r\n");
        } else {
            response.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
        }
        let mut bytes = response.into_bytes();
        if chunked {
            for chunk in body.chunks(5) {
                bytes.extend(format!("{:x}\r\n", chunk.len()).bytes());
                bytes.extend(chunk);
                bytes.extend(b"\r\n");
            }
            bytes.extend(b"0\r\n\r\n");
        } else {
            bytes.extend(body);
        }
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut head = vec![];
            loop {
                let b = stream.read_u8().await.unwrap();
                head.push(b);
                if head.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let _ = stream.write_all(&bytes).await;
            String::from_utf8(head).unwrap()
        });
        (origin, task)
    }

    #[tokio::test]
    async fn native_video_staging_preserves_bytes_and_does_not_send_credentials() {
        let bytes = b"\0\0\0\x18ftypmp42native-provider-video";
        let (origin, request) = source(bytes, 200, "Content-Type: video/mp4\r\n", true).await;
        let mut worker = worker(
            serde_json::json!({"id":"remote", "status":"stopped",
            "recordingUrl":format!("{origin}/recording?signature=private"), "recordingAvailable":false}),
            1024,
        );
        worker = worker.with_test_source_origin(&origin);
        let mut staged = worker.prepare_video("local", "remote").await.unwrap();
        assert_eq!(staged.name, "local.mp4");
        assert_eq!(staged.content_type, "video/mp4");
        assert_eq!(staged.size, bytes.len() as u64);
        assert_eq!(staged.body_sha256, hex::encode(Sha256::digest(bytes)));
        let mut downloaded = vec![];
        staged.file.read_to_end(&mut downloaded).await.unwrap();
        assert_eq!(downloaded, bytes);
        let head = request.await.unwrap().to_ascii_lowercase();
        for forbidden in
            ["authorization:", "x-browser-use-api-key:", "x-iam-obo-access-proof:", "x-testing-environment-key:"]
        {
            assert!(!head.contains(forbidden));
        }
    }

    #[tokio::test]
    async fn download_rejects_oversize_unknown_length_html_and_redirects() {
        for (body, status, chunked, limit) in [
            (b"\0\0\0\x18ftypmp42large-video".as_slice(), 200, true, 15),
            (b"\0\0\0\x18ftypmp42large-video".as_slice(), 200, false, 15),
            (b"<html>provider error private-secret</html>".as_slice(), 200, false, 1024),
            (b"private-secret".as_slice(), 302, false, 1024),
        ] {
            let (origin, request) = source(body, status, "Location: http://127.0.0.1:1/secret\r\n", chunked).await;
            let mut worker = idle_worker(limit);
            worker.local_source_origin = Some(origin.clone());
            let error = worker.download(&format!("{origin}/?signature=private-secret")).await.unwrap_err();
            assert!(!error.to_string().contains("private-secret"));
            assert!(!error.to_string().contains(&origin));
            if limit == 15 {
                assert!(matches!(error, DeliveryError::TooLarge));
            }
            request.await.unwrap();
        }
    }

    #[test]
    fn source_policy_rejects_literal_and_resolved_private_targets() {
        let worker = idle_worker(1024);
        for raw in [
            "http://public.example/video",
            "https://user:secret@public.example/video",
            "https://public.example/video#secret",
            "https://localhost/video",
            "https://127.1/video",
            "https://169.254.169.254/video",
            "https://[::ffff:127.0.0.1]/video",
        ] {
            assert!(worker.source_url(raw).is_err(), "{raw}");
        }
        for address in
            ["127.0.0.1:443", "10.0.0.1:443", "169.254.169.254:443", "100.64.0.1:443", "[::1]:443", "[fd00::1]:443"]
        {
            assert!(!worker.address_allowed(&address.parse().unwrap()), "{address}");
        }
        assert!(worker.source_url("https://public.example/video?signature=allowed").is_ok());
        assert!(worker.address_allowed(&"8.8.8.8:443".parse().unwrap()));
    }

    #[test]
    fn dns_failures_are_retryable_but_private_resolutions_are_terminal() {
        let worker = idle_worker(1024);
        for result in [Err(std::io::Error::other("signed-url-secret")), Ok(vec![])] {
            let error = worker.validate_resolved_addresses(result).unwrap_err();
            assert!(matches!(error, DeliveryError::Provider(ProviderError::Transport { .. })));
            assert!(!error.to_string().contains("signed-url-secret"));
        }
        assert!(matches!(
            worker.validate_resolved_addresses(Ok(vec!["10.0.0.1:443".parse().unwrap()])),
            Err(DeliveryError::InvalidSource)
        ));
        let public = vec!["8.8.8.8:443".parse().unwrap()];
        assert_eq!(worker.validate_resolved_addresses(Ok(public.clone())).unwrap(), public);
    }

    #[tokio::test]
    async fn recording_readiness_and_identity_gate_downloads() {
        for (value, expected) in [
            (serde_json::json!({"id":"remote","status":"stopped","recordingAvailable":false}), "unavailable"),
            (serde_json::json!({"id":"remote","status":"stopped"}), "not ready"),
            (
                serde_json::json!({"id":"remote","status":"active","recordingUrl":"https://public.example/video"}),
                "not ready",
            ),
            (serde_json::json!({"id":"different","status":"stopped"}), "validation"),
        ] {
            let error = worker(value, 1024).prepare_video("local", "remote").await.unwrap_err();
            assert!(error.to_string().contains(expected));
        }
    }

    fn log(sequence: u64) -> SessionLog {
        SessionLog {
            sequence,
            at: chrono::DateTime::parse_from_rfc3339("2026-09-05T01:02:03Z").unwrap().to_utc(),
            actor_id: "@silicon:org".into(),
            command: "evaluate 'a  b' --flag \"quoted\"\nnext".into(),
            exit_code: Some(0),
        }
    }

    #[tokio::test]
    async fn paged_logs_preserve_commands_metadata_and_stable_digest() {
        let worker = idle_worker(4096);
        let mut cursors = vec![];
        let mut staged = worker
            .prepare_log_pages("local", |after| {
                cursors.push(after);
                std::future::ready(Ok(match after {
                    0 => vec![log(1)],
                    1 => vec![log(3)],
                    _ => vec![],
                }))
            })
            .await
            .unwrap();
        assert_eq!(cursors, [0, 1, 3]);
        assert_eq!(staged.name, "local-commands.jsonl");
        let mut body = String::new();
        staged.file.read_to_string(&mut body).await.unwrap();
        let decoded = body.lines().map(|line| serde_json::from_str::<SessionLog>(line).unwrap()).collect::<Vec<_>>();
        assert_eq!(decoded, [log(1), log(3)]);
        let equivalent = worker.prepare_log("local", &[log(1), log(3)]).await.unwrap();
        assert_eq!(equivalent.body_sha256, staged.body_sha256);
        assert_eq!(equivalent.size, staged.size);
        staged.file.rewind().await.unwrap();
        assert!(!format!("{staged:?}").contains("evaluate"));
    }

    #[tokio::test]
    async fn staged_artifact_upload_uses_exact_hashed_bytes_and_supplied_proof() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let mut worker = idle_worker(4096);
        worker.briefcase = BriefcaseClient::with_upload_limit(&origin, None, 4096).unwrap();
        let staged = worker.prepare_log("local", &[log(1)]).await.unwrap();
        let digest = staged.body_sha256.clone();
        let expected_size = staged.size;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut head = vec![];
            loop {
                head.push(socket.read_u8().await.unwrap());
                if head.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let head = String::from_utf8(head).unwrap().to_ascii_lowercase();
            assert!(head.starts_with("post /api/v1/obo/files "));
            assert!(head.contains("x-iam-obo-access-proof: obo_supplied-proof\r\n"));
            assert!(!head.contains("authorization:"));
            let size =
                head.lines().find_map(|line| line.strip_prefix("content-length: ")).unwrap().parse::<usize>().unwrap();
            assert_eq!(size as u64, expected_size);
            let mut body = vec![0; size];
            socket.read_exact(&mut body).await.unwrap();
            assert_eq!(hex::encode(Sha256::digest(&body)), digest);
            let reply = serde_json::json!({"id":"00000000-0000-0000-0000-000000000001", "org_id":"org", "type":"file", "name":"local-commands.jsonl",
                "path":"private/actor/browser/local-commands.jsonl", "size":size,
                "permanent_url":"https://briefcase.example/entry", "origin_app_id":"org>browser"})
            .to_string();
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 201 Created\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                        reply.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let receipt = worker
            .upload(staged, &OnBehalfOfGrant::new("obo_supplied-proof").unwrap(), "org", "org>browser")
            .await
            .unwrap();
        assert_eq!(receipt.size, expected_size);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn log_staging_rejects_repeated_pages_and_bounds_total_bytes() {
        let worker = idle_worker(4096);
        assert!(matches!(
            worker.prepare_log_pages("local", |_| std::future::ready(Ok(vec![log(1)]))).await,
            Err(DeliveryError::InvalidSource)
        ));
        assert!(matches!(
            worker.prepare_log_pages("local", |_| std::future::ready(Ok(vec![log(1); 129]))).await,
            Err(DeliveryError::InvalidSource)
        ));
        assert!(matches!(idle_worker(15).prepare_log("local", &[log(1)]).await, Err(DeliveryError::TooLarge)));
        assert!(worker.prepare_log("../escape", &[]).await.is_err());
    }
}
