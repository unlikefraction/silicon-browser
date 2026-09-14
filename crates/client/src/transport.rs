use std::time::Duration;

use serde_json::Value;
use silicon_browser_shared::{TestingCredentials, Validate};

use crate::Error;

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, Copy)]
pub enum Method {
    Get,
    Post,
    Patch,
}

#[derive(Clone)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub bearer: Option<String>,
    pub org: Option<String>,
    pub body: Option<Value>,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Request")
            .field("method", &self.method)
            .field("url", &self.url.split_once('?').map_or(self.url.as_str(), |(path, _)| path))
            .field("bearer", &self.bearer.as_ref().map(|_| "[REDACTED]"))
            .field("org", &self.org)
            .field("body", &self.body.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[derive(Clone)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl std::fmt::Debug for Response {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Response").field("status", &self.status).field("body_bytes", &self.body.len()).finish()
    }
}

/// Injectable transport keeps the package stateless and makes its exact HTTP contract testable.
pub trait Transport: Send + Sync {
    fn send(&self, request: Request) -> Result<Response, Error>;

    /// Send an operation with a caller-selected finite end-to-end timeout. Existing custom
    /// transports remain compatible; transports with their own timeout controls should override
    /// this method.
    fn send_with_timeout(&self, request: Request, _timeout: Duration) -> Result<Response, Error> {
        self.send(request)
    }
}

type UnauthorizedRecovery = dyn Fn(&Request) -> Result<Option<String>, Error> + Send + Sync;

struct RecoveredCredential {
    original: String,
    replacement: String,
    org: Option<String>,
    origin: String,
}

pub struct HttpTransport {
    agent: ureq::Agent,
    testing: Option<(url::Url, TestingCredentials)>,
    unauthorized_recovery: Option<std::sync::Arc<UnauthorizedRecovery>>,
    recovered: std::sync::Mutex<Option<RecoveredCredential>>,
}

impl Default for HttpTransport {
    fn default() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(DEFAULT_REQUEST_TIMEOUT))
            .http_status_as_error(false)
            // Auth exchange bodies and bearer headers must never be replayed
            // to a Location chosen by an intermediary or compromised host.
            .max_redirects(0)
            .build()
            .new_agent();
        Self { agent, testing: None, unauthorized_recovery: None, recovered: std::sync::Mutex::new(None) }
    }
}

impl HttpTransport {
    /// Pin developer credentials to one `/testing/<environment-id>` API base.
    /// Requests outside that origin and path fail locally; redirects remain disabled.
    pub fn with_testing(mut self, base: &str, credentials: TestingCredentials) -> Result<Self, Error> {
        credentials.validate().map_err(|error| Error::Local(error.to_string()))?;
        let base = url::Url::parse(&crate::normalize_backend_url(base)?)
            .map_err(|_| Error::Local("invalid test backend URL".into()))?;
        let (parent, id) = base.path().rsplit_once('/').unwrap_or_default();
        if !parent.ends_with("/testing") || uuid::Uuid::parse_str(id).is_err() {
            return Err(Error::Local("test transport requires a backend ending in /testing/<environment-uuid>".into()));
        }
        self.testing = Some((base, credentials));
        Ok(self)
    }

    /// Optional caller-owned credential recovery. Only a marked pre-handler HTTP 401 may
    /// trigger one retry. No refresh token or filesystem state is owned by this transport.
    pub fn with_unauthorized_recovery(
        mut self,
        recover: impl Fn(&Request) -> Result<Option<String>, Error> + Send + Sync + 'static,
    ) -> Self {
        self.unauthorized_recovery = Some(std::sync::Arc::new(recover));
        self
    }

    fn send_bounded(&self, request: Request, timeout: Duration) -> Result<Response, Error> {
        read_response(self.response_with_recovery(request, timeout)?)
    }

    fn response_with_recovery(
        &self,
        mut request: Request,
        timeout: Duration,
    ) -> Result<ureq::http::Response<ureq::Body>, Error> {
        let original = request.bearer.clone();
        let origin = url::Url::parse(&request.url)
            .map_err(|_| Error::Local("invalid request URL".into()))?
            .origin()
            .ascii_serialization();
        {
            let recovered =
                self.recovered.lock().map_err(|_| Error::Local("credential recovery lock failed".into()))?;
            if let Some(recovered) = recovered.as_ref()
                && original.as_deref() == Some(recovered.original.as_str())
                && request.org == recovered.org
                && origin == recovered.origin
            {
                // Only reuse a credential returned by this transport's own recovery callback.
                // This never adopts another process's subsequently replaced CLI identity.
                request.bearer = Some(recovered.replacement.clone());
            }
        }
        let response = self.perform(&request, timeout)?;
        if response.status().as_u16() == 401
            && response.headers().get("x-sb-auth-rejected").is_some_and(|value| value == "1")
            && request.bearer.is_some()
            && let Some(recover) = &self.unauthorized_recovery
            && let Some(token) = recover(&request)?
        {
            crate::Auth::new(&token)?;
            drop(response);
            *self.recovered.lock().map_err(|_| Error::Local("credential recovery lock failed".into()))? =
                Some(RecoveredCredential {
                    original: original.expect("recovery requires a bearer"),
                    replacement: token.clone(),
                    org: request.org.clone(),
                    origin,
                });
            request.bearer = Some(token);
            return self.perform(&request, timeout);
        }
        Ok(response)
    }

    fn perform(&self, request: &Request, timeout: Duration) -> Result<ureq::http::Response<ureq::Body>, Error> {
        if let Some((base, _)) = &self.testing {
            let target = url::Url::parse(&request.url).map_err(|_| Error::Local("invalid request URL".into()))?;
            let path = target.path().to_ascii_lowercase();
            if target.origin() != base.origin()
                || !target.username().is_empty()
                || target.password().is_some()
                || !target.path().starts_with(&format!("{}/api/v1/", base.path()))
                || path.contains("%2f")
                || path.contains("%5c")
            {
                return Err(Error::Local(
                    "test credentials cannot be sent outside their enrolled environment API".into(),
                ));
            }
        }
        match request.method {
            Method::Get => {
                let built = self.agent.get(&request.url).config().timeout_global(Some(timeout)).build();
                self.headers(built, request).call()
            }
            Method::Post => {
                let built = self.agent.post(&request.url).config().timeout_global(Some(timeout)).build();
                self.headers(built, request).send_json(request.body.as_ref().unwrap_or(&Value::Null))
            }
            Method::Patch => {
                let built = self.agent.patch(&request.url).config().timeout_global(Some(timeout)).build();
                self.headers(built, request).send_json(request.body.as_ref().unwrap_or(&Value::Null))
            }
        }
        .map_err(|error| Error::Transport(redact_url(error.to_string(), &request.url)))
    }

    fn headers<B>(&self, mut built: ureq::RequestBuilder<B>, request: &Request) -> ureq::RequestBuilder<B> {
        if let Some(token) = &request.bearer {
            built = built.header("Authorization", format!("Bearer {token}"));
        }
        if let Some(org) = &request.org {
            built = built.header("X-Org-ID", org);
        }
        if let Some((_, credentials)) = &self.testing {
            built = built.header("x-sb-test-app-secret", &credentials.app_secret);
            if let Some(key) = &credentials.iam_test_key {
                built = built.header("x-testing-environment-key", key);
            }
            if let Some(key) = &credentials.briefcase_test_environment_key {
                built = built.header("x-sb-test-briefcase-key", key);
            }
        }
        built
    }
}

impl Transport for HttpTransport {
    fn send(&self, request: Request) -> Result<Response, Error> {
        self.send_bounded(request, DEFAULT_REQUEST_TIMEOUT)
    }

    fn send_with_timeout(&self, request: Request, timeout: Duration) -> Result<Response, Error> {
        self.send_bounded(request, timeout)
    }
}

fn read_response(mut response: ureq::http::Response<ureq::Body>) -> Result<Response, Error> {
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(64 << 20)
        .read_to_vec()
        .map_err(|error| Error::Transport(error.to_string()))?;
    Ok(Response { status, body })
}

fn redact_url(mut message: String, url: &str) -> String {
    if let Some(query) = url.split_once('?').map(|(_, query)| query) {
        message = message.replace(query, "[REDACTED]");
    }
    message
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn test_transport_sends_credentials_on_all_methods_and_rejects_other_scopes() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", server.local_addr().unwrap());
        let base = format!("{origin}/testing/{}", uuid::Uuid::new_v4());
        let worker = std::thread::spawn(move || {
            for method in ["GET", "POST", "PATCH"] {
                let (mut connection, _) = server.accept().unwrap();
                connection.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut reader = std::io::BufReader::new(&mut connection);
                let mut first = String::new();
                std::io::BufRead::read_line(&mut reader, &mut first).unwrap();
                assert!(first.starts_with(method));
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
                    assert!(!line.is_empty());
                    if line == "\r\n" {
                        break;
                    }
                    headers.push_str(&line.to_ascii_lowercase());
                }
                assert!(headers.contains("x-sb-test-app-secret: ask_private\r\n"));
                assert!(headers.contains(&format!("x-testing-environment-key: {}\r\n", "a".repeat(32))));
                assert!(headers.contains(&format!("x-sb-test-briefcase-key: ask_{}\r\n", "b".repeat(43))));
                assert!(headers.contains("authorization: bearer oat_private\r\n"));
                assert!(headers.contains("x-org-id: tos\r\n"));
                let body_length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .map(|v| v.trim().parse::<usize>().unwrap())
                    .unwrap_or(0);
                let mut body = vec![0; body_length];
                reader.read_exact(&mut body).unwrap();
                connection.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").unwrap();
            }
        });
        let transport = HttpTransport::default()
            .with_testing(
                &base,
                TestingCredentials {
                    app_secret: "ask_private".into(),
                    iam_test_key: Some("a".repeat(32)),
                    briefcase_test_environment_key: Some(format!("ask_{}", "b".repeat(43))),
                },
            )
            .unwrap();
        let request = |method, url| Request {
            method,
            url,
            bearer: Some("oat_private".into()),
            org: Some("tos".into()),
            body: None,
        };
        for path in [
            format!("{origin}/api/v1/profiles"),
            format!("{base}-other/api/v1/profiles"),
            format!("{base}/api/v1/../../profiles"),
            format!("{base}/api/v1/%2f..%2fprofiles"),
            "https://other.example/api/v1/profiles".into(),
        ] {
            assert!(matches!(transport.send(request(Method::Get, path)), Err(Error::Local(_))));
        }
        for method in [Method::Get, Method::Post, Method::Patch] {
            assert_eq!(transport.send(request(method, format!("{base}/api/v1/profiles"))).unwrap().status, 200);
        }
        worker.join().unwrap();
    }

    /// Redirects cannot replay bearer credentials to another origin.

    #[test]
    fn authenticated_requests_never_follow_redirects() {
        let attacker = TcpListener::bind("127.0.0.1:0").unwrap();
        attacker.set_nonblocking(true).unwrap();
        let redirect = TcpListener::bind("127.0.0.1:0").unwrap();
        let destination = attacker.local_addr().unwrap();
        let redirect_address = redirect.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut connection, _) = redirect.accept().unwrap();
            connection.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            connection.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut request = Vec::new();
            let header_end = loop {
                let mut chunk = [0_u8; 1024];
                let count = connection.read(&mut chunk).unwrap();
                assert_ne!(count, 0, "request closed before the HTTP headers completed");
                request.extend_from_slice(&chunk[..count]);
                assert!(request.len() <= 16 * 1024, "request headers exceeded the test bound");
                if let Some(index) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    break index + 4;
                }
            };
            let headers = std::str::from_utf8(&request[..header_end]).unwrap().to_ascii_lowercase();
            assert!(headers.contains("authorization: bearer oat_private\r\n"));
            let content_length = headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| *name == "content-length")
                .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                .expect("JSON request must declare Content-Length");
            assert!(content_length <= 4096, "request body exceeded the test bound");
            let received = request.len();
            let complete_length = header_end + content_length;
            assert!(received <= complete_length);
            request.resize(complete_length, 0);
            connection.read_exact(&mut request[received..]).unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&request[header_end..]).unwrap(),
                serde_json::json!({"short_lived_token":"slt_private"})
            );
            write!(
                connection,
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{destination}/stolen\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        });
        let response = HttpTransport::default()
            .send(Request {
                method: Method::Post,
                url: format!("http://{redirect_address}/exchange"),
                bearer: Some("oat_private".into()),
                org: Some("org-1".into()),
                body: Some(serde_json::json!({"short_lived_token":"slt_private"})),
            })
            .unwrap();
        server.join().unwrap();
        assert_eq!(response.status, 307);
        assert!(matches!(attacker.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock));
    }
}
