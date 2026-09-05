use std::time::Duration;

use serde_json::Value;

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
        Self { agent, unauthorized_recovery: None, recovered: std::sync::Mutex::new(None) }
    }
}

impl HttpTransport {
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
        match request.method {
            Method::Get => {
                let built = self.agent.get(&request.url).config().timeout_global(Some(timeout)).build();
                match request.bearer.as_deref() {
                    Some(token) => {
                        let built = built.header("Authorization", &format!("Bearer {token}"));
                        match request.org.as_deref() {
                            Some(org) => built.header("X-Org-ID", org).call(),
                            None => built.call(),
                        }
                    }
                    None => match request.org.as_deref() {
                        Some(org) => built.header("X-Org-ID", org).call(),
                        None => built.call(),
                    },
                }
            }
            Method::Post => {
                let built = self.agent.post(&request.url).config().timeout_global(Some(timeout)).build();
                let built = match request.bearer.as_deref() {
                    Some(token) => built.header("Authorization", &format!("Bearer {token}")),
                    None => built,
                };
                let built = match request.org.as_deref() {
                    Some(org) => built.header("X-Org-ID", org),
                    None => built,
                };
                built.send_json(request.body.as_ref().unwrap_or(&Value::Null))
            }
            Method::Patch => {
                let built = self.agent.patch(&request.url).config().timeout_global(Some(timeout)).build();
                let built = match request.bearer.as_deref() {
                    Some(token) => built.header("Authorization", &format!("Bearer {token}")),
                    None => built,
                };
                let built = match request.org.as_deref() {
                    Some(org) => built.header("X-Org-ID", org),
                    None => built,
                };
                built.send_json(request.body.as_ref().unwrap_or(&Value::Null))
            }
        }
        .map_err(|error| Error::Transport(redact_url(error.to_string(), &request.url)))
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
