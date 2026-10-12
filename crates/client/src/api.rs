use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use silicon_browser_shared::*;
use url::{Host, Url};

use crate::transport::{HttpTransport, Method, Request, Transport};
use crate::{Auth, Error};

const FETCH_REQUEST_TIMEOUT: Duration = Duration::from_secs(4 * 60 * 60 + 30 * 60);

/// Silicon Browser's complete public interface: explicit identity and no local
/// state. Clone is cheap and safe to share between synchronous workers.
#[derive(Clone)]
pub struct Client {
    base: String,
    auth: Auth,
    transport: Arc<dyn Transport>,
}

impl Client {
    pub fn new(base: impl Into<String>, auth: Auth) -> Result<Self, Error> {
        Self::with_transport(base, auth, Arc::new(HttpTransport::default()))
    }

    pub fn with_transport(base: impl Into<String>, auth: Auth, transport: Arc<dyn Transport>) -> Result<Self, Error> {
        Ok(Self { base: normalize_backend_url(&base.into())?, auth, transport })
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    // Authentication and identity ---------------------------------------------------------

    /// Discover the backend's Silicon Accounts application without an existing login.
    pub fn accounts(base: impl Into<String>) -> Result<AccountsInfo, Error> {
        Self::accounts_with_transport(base, Arc::new(HttpTransport::default()))
    }

    pub fn accounts_with_transport(
        base: impl Into<String>,
        transport: Arc<dyn Transport>,
    ) -> Result<AccountsInfo, Error> {
        let client = Self::with_transport(base, Auth::new("pre-auth")?, transport)?;
        let info: AccountsInfo = client.send_without_auth(Method::Get, "/api/v1/accounts", None)?;
        app_id(&info.app_id, "app_id").map_err(protocol)?;
        Ok(info)
    }

    /// Exchange a single-use Silicon Accounts short-lived token. This is deliberately an associated
    /// function: there is no bearer with which to construct a normal Client yet.
    pub fn exchange(base: impl Into<String>, request: &AuthExchangeRequest) -> Result<AuthSession, Error> {
        Self::exchange_with_transport(base, request, Arc::new(HttpTransport::default()))
    }

    pub fn exchange_with_transport(
        base: impl Into<String>,
        request: &AuthExchangeRequest,
        transport: Arc<dyn Transport>,
    ) -> Result<AuthSession, Error> {
        request.validate().map_err(validation)?;
        let placeholder = Auth::new("pre-auth")?;
        let client = Self::with_transport(base, placeholder, transport)?;
        let mut body = json(request)?;
        body["contract_version"] = 2.into();
        let session: AuthSession = client.send_without_auth(Method::Post, "/api/v1/auth/exchange", Some(body))?;
        session.validate().map_err(protocol)?;
        Ok(session)
    }

    pub fn refresh(base: impl Into<String>, request: &AuthRefreshRequest) -> Result<AuthSession, Error> {
        Self::refresh_with_transport(base, request, Arc::new(HttpTransport::default()))
    }

    pub fn refresh_with_transport(
        base: impl Into<String>,
        request: &AuthRefreshRequest,
        transport: Arc<dyn Transport>,
    ) -> Result<AuthSession, Error> {
        request.validate().map_err(validation)?;
        let placeholder = Auth::new("pre-auth")?;
        let client = Self::with_transport(base, placeholder, transport)?;
        let mut body = json(request)?;
        body["contract_version"] = 2.into();
        let session: AuthSession = client.send_without_auth(Method::Post, "/api/v1/auth/refresh", Some(body))?;
        session.validate().map_err(protocol)?;
        Ok(session)
    }

    pub fn me(&self) -> Result<Identity, Error> {
        let identity: Identity = self.get("/api/v1/me")?;
        identity.validate().map_err(protocol)?;
        Ok(identity)
    }

    /// Return the services enabled by the backend right now. This is queried
    /// live instead of trusting capabilities cached at token-exchange time.
    pub fn services(&self) -> Result<Vec<String>, Error> {
        self.get("/api/v1/services")
    }

    // Profiles and proxy locations --------------------------------------------------------

    pub fn proxy_locations(&self) -> Result<Vec<ProxyLocation>, Error> {
        self.get("/api/v1/proxy-locations")
    }

    pub fn profiles(&self, filter: Option<&str>) -> Result<Vec<Profile>, Error> {
        self.get(&with_filter("/api/v1/profiles", filter))
    }

    pub fn profile(&self, profile_id: &str) -> Result<Profile, Error> {
        self.get(&format!("/api/v1/profiles/{}", segment(profile_id)?))
    }

    pub fn create_profile(&self, request: &ProfileCreate) -> Result<Profile, Error> {
        request.validate().map_err(validation)?;
        self.post("/api/v1/profiles", request)
    }

    pub fn update_profile(&self, profile_id: &str, request: &ProfileUpdate) -> Result<Profile, Error> {
        request.validate().map_err(validation)?;
        self.patch(&format!("/api/v1/profiles/{}", segment(profile_id)?), request)
    }

    pub fn end_profile(&self, profile_id: &str, request: &ProfileEnd) -> Result<Profile, Error> {
        request.validate().map_err(validation)?;
        self.post(&format!("/api/v1/profiles/{}/end", segment(profile_id)?), request)
    }

    // Sessions and execution --------------------------------------------------------------

    pub fn sessions(&self, filter: Option<&str>) -> Result<Vec<Session>, Error> {
        self.get(&with_filter("/api/v1/sessions", filter))
    }

    pub fn session(&self, session_id: &str) -> Result<Session, Error> {
        self.get(&format!("/api/v1/sessions/{}", segment(session_id)?))
    }

    pub fn create_session(&self, request: &SessionCreate) -> Result<Session, Error> {
        request.validate().map_err(validation)?;
        self.post("/api/v1/sessions", request)
    }

    pub fn end_session(&self, session_id: &str, request: &SessionEnd) -> Result<Session, Error> {
        request.validate().map_err(validation)?;
        self.post(&format!("/api/v1/sessions/{}/end", segment(session_id)?), request)
    }

    pub fn live(&self, session_id: &str) -> Result<LiveLink, Error> {
        self.post(&format!("/api/v1/sessions/{}/live", segment(session_id)?), &Value::Null)
    }

    /// Redeem a live-link fragment after the viewer authenticates. Successful
    /// redemption also associates that viewer with the managed session.
    pub fn redeem_live(&self, session_id: &str, request: &LiveRedeemRequest) -> Result<LiveLink, Error> {
        request.validate().map_err(validation)?;
        self.post(&format!("/api/v1/sessions/{}/live/redeem", segment(session_id)?), request)
    }

    /// Obtain a short-lived direct browser connection after authorization. Treat its URL as a
    /// credential. Browser control goes directly from the local controller to this endpoint.
    pub fn session_connection(&self, session_id: &str) -> Result<SessionConnection, Error> {
        let connection: SessionConnection =
            self.get(&format!("/api/v1/sessions/{}/connection", segment(session_id)?))?;
        if connection.session_id != session_id || connection.principal_id.trim().is_empty() {
            return Err(Error::Protocol("session connection identity did not match the request".into()));
        }
        account_id(&connection.principal_id, "principal_id").map_err(protocol)?;
        crate::controller::validate_connection(&connection)?;
        Ok(connection)
    }

    // Recordings and usage ---------------------------------------------------------------

    pub fn recordings(&self, filter: Option<&str>) -> Result<Vec<Recording>, Error> {
        self.get(&with_filter("/api/v1/recordings", filter))
    }

    pub fn recording(&self, session_id: &str) -> Result<Recording, Error> {
        self.get(&format!("/api/v1/recordings/{}", segment(session_id)?))
    }

    pub fn trash_recording(&self, session_id: &str) -> Result<Recording, Error> {
        self.post(&format!("/api/v1/recordings/{}/trash", segment(session_id)?), &Value::Null)
    }

    pub fn usage(&self, session_id: &str) -> Result<Usage, Error> {
        self.get(&format!("/api/v1/usage/{}", segment(session_id)?))
    }

    pub fn usage_list(&self, filter: Option<&str>) -> Result<Vec<Usage>, Error> {
        self.get(&with_filter("/api/v1/usage", filter))
    }

    /// Current capacity of the shared browser-provider account.
    /// The backend refreshes the authoritative account snapshot at most once per minute.
    pub fn usage_limits(&self) -> Result<UsageLimits, Error> {
        self.get("/api/v1/usage/limits")
    }

    // Read-heavy discovery ---------------------------------------------------------------

    pub fn search(&self, request: &SearchRequest) -> Result<SearchResponse, Error> {
        request.validate().map_err(validation)?;
        self.post("/api/v1/search", request)
    }

    pub fn fetch(&self, request: &FetchRequest) -> Result<FetchResponse, Error> {
        request.validate().map_err(validation)?;
        self.post_with_timeout("/api/v1/fetch", request, FETCH_REQUEST_TIMEOUT)
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        self.send(Method::Get, path, None)
    }

    fn post<I: Serialize + ?Sized, O: DeserializeOwned>(&self, path: &str, body: &I) -> Result<O, Error> {
        self.send(Method::Post, path, Some(json(body)?))
    }

    fn post_with_timeout<I: Serialize + ?Sized, O: DeserializeOwned>(
        &self,
        path: &str,
        body: &I,
        timeout: Duration,
    ) -> Result<O, Error> {
        let request = self.request(Method::Post, path, Some(json(body)?))?;
        let response = self.transport.send_with_timeout(request, timeout)?;
        decode_response(response)
    }

    fn patch<I: Serialize + ?Sized, O: DeserializeOwned>(&self, path: &str, body: &I) -> Result<O, Error> {
        self.send(Method::Patch, path, Some(json(body)?))
    }

    fn send<T: DeserializeOwned>(&self, method: Method, path: &str, body: Option<Value>) -> Result<T, Error> {
        let request = self.request(method, path, body)?;
        let response = self.transport.send(request)?;
        decode_response(response)
    }

    fn send_without_auth<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<T, Error> {
        let response = self.transport.send(Request {
            method,
            url: format!("{}{}", self.base, path),
            bearer: None,
            body,
            idempotency_key: None,
        })?;
        decode_response(response)
    }

    fn request(&self, method: Method, path: &str, body: Option<Value>) -> Result<Request, Error> {
        if !path.starts_with('/') {
            return Err(Error::Local("API path must be absolute".into()));
        }
        Ok(Request {
            method,
            url: format!("{}{}", self.base, path),
            bearer: Some(self.auth.expose().to_owned()),
            body,
            idempotency_key: None,
        })
    }
}

/// Canonical credential issuer key, shared by the stateless client and caller-owned storage.
pub fn normalize_backend_url(base: &str) -> Result<String, Error> {
    let parsed = Url::parse(base)
        .ok()
        .filter(|url| {
            (url.scheme() == "https" || (url.scheme() == "http" && backend_host_is_loopback(url)))
                && url.host().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        })
        .ok_or_else(|| {
            Error::Local("backend URL must use HTTPS, or HTTP with localhost/loopback for local development".into())
        })?;
    Ok(parsed.as_str().trim_end_matches('/').to_owned())
}

fn backend_host_is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

fn decode_response<T: DeserializeOwned>(response: crate::transport::Response) -> Result<T, Error> {
    if !(200..300).contains(&response.status) {
        return Err(crate::decode_error(response.status, &response.body));
    }
    let envelope: Envelope<T> = serde_json::from_slice(&response.body).map_err(protocol)?;
    Ok(envelope.data)
}

fn validation(error: ValidationError) -> Error {
    Error::Local(error.to_string())
}

fn protocol(error: impl std::fmt::Display) -> Error {
    Error::Protocol(error.to_string())
}

fn json(value: &(impl Serialize + ?Sized)) -> Result<Value, Error> {
    serde_json::to_value(value).map_err(protocol)
}

fn segment(value: &str) -> Result<String, Error> {
    validate_id(value, "id")?;
    Ok(encode(value))
}

fn validate_id(value: &str, field: &'static str) -> Result<(), Error> {
    if value.is_empty()
        || matches!(value, "." | "..")
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace() || matches!(character, '/' | '\\'))
    {
        return Err(Error::Local(format!("{field} is not a valid identifier")));
    }
    Ok(())
}

fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn with_filter(path: &str, filter: Option<&str>) -> String {
    match filter.filter(|value| !value.trim().is_empty()) {
        Some(filter) => format!("{path}?filter={}", encode(filter)),
        None => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::transport::Response;

    #[derive(Default)]
    struct FakeTransport(Mutex<Vec<Request>>);

    impl Transport for FakeTransport {
        fn send(&self, request: Request) -> Result<Response, Error> {
            self.0.lock().unwrap().push(request);
            Ok(Response { status: 200, body: br#"{"data":[]}"#.to_vec() })
        }
    }

    #[derive(Default)]
    struct TimedFetchTransport(Mutex<Vec<(Request, Duration)>>);

    impl Transport for TimedFetchTransport {
        fn send(&self, _request: Request) -> Result<Response, Error> {
            panic!("fetch must use the timeout-aware transport method");
        }

        fn send_with_timeout(&self, request: Request, timeout: Duration) -> Result<Response, Error> {
            self.0.lock().unwrap().push((request, timeout));
            Ok(Response { status: 200, body: br#"{"data":{"items":[],"queued_ms":0}}"#.to_vec() })
        }
    }

    /// Test group: the stateless package forms authenticated requests exactly once.
    #[test]
    fn list_profiles_carries_explicit_auth_and_encoded_filter() {
        let transport = Arc::new(FakeTransport::default());
        let client = Client::with_transport(
            "https://backend.example/",
            Auth::new("jwt.secret.signature").unwrap(),
            transport.clone(),
        )
        .unwrap();
        let profiles = client.profiles(Some("name:market research -> is:mine")).unwrap();
        assert!(profiles.is_empty());
        let requests = transport.0.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].bearer.as_deref(), Some("jwt.secret.signature"));
        assert!(requests[0].url.ends_with("filter=name%3Amarket+research+-%3E+is%3Amine"));
    }

    /// Test group: URL normalization cannot reinterpret a caller's ID as a parent endpoint.
    #[test]
    fn dot_segment_resource_ids_are_rejected_before_transport() {
        let transport = Arc::new(FakeTransport::default());
        let client = Client::with_transport(
            "https://backend.example",
            Auth::new("jwt.secret.signature").unwrap(),
            transport.clone(),
        )
        .unwrap();
        for id in [".", ".."] {
            assert!(matches!(client.profile(id), Err(Error::Local(_))));
            assert!(matches!(client.session(id), Err(Error::Local(_))));
            assert!(matches!(client.trash_recording(id), Err(Error::Local(_))));
        }
        assert!(transport.0.lock().unwrap().is_empty());
    }

    /// Test group: capability discovery is an authenticated live
    /// backend query rather than client-side credential metadata.
    #[test]
    fn services_uses_the_live_scoped_endpoint() {
        let transport = Arc::new(FakeTransport::default());
        let client = Client::with_transport(
            "https://backend.example",
            Auth::new("jwt.secret.signature").unwrap(),
            transport.clone(),
        )
        .unwrap();
        assert!(client.services().unwrap().is_empty());
        let requests = transport.0.lock().unwrap();
        assert!(requests[0].url.ends_with("/api/v1/services"));
        assert_eq!(requests[0].bearer.as_deref(), Some("jwt.secret.signature"));
    }

    /// Test group: live-link redemption remains an authenticated
    /// operation and carries its opaque grant only in the JSON body.
    #[test]
    fn redeem_live_uses_the_typed_scoped_endpoint() {
        struct LiveTransport(Mutex<Vec<Request>>);

        impl Transport for LiveTransport {
            fn send(&self, request: Request) -> Result<Response, Error> {
                self.0.lock().unwrap().push(request);
                Ok(Response {
                    status: 200,
                    body: br#"{"data":{"session_id":"session-1","url":"https://live.example/view","expires_at":"2030-03-17T17:46:40Z"}}"#.to_vec(),
                })
            }
        }

        let transport = Arc::new(LiveTransport(Mutex::new(Vec::new())));
        let client = Client::with_transport(
            "https://backend.example",
            Auth::new("jwt.secret.signature").unwrap(),
            transport.clone(),
        )
        .unwrap();
        let live = client.redeem_live("session-1", &LiveRedeemRequest { grant: "opaque-grant".into() }).unwrap();
        assert_eq!(live.session_id, "session-1");

        let requests = transport.0.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].url.ends_with("/api/v1/sessions/session-1/live/redeem"));
        assert_eq!(requests[0].bearer.as_deref(), Some("jwt.secret.signature"));
        assert_eq!(requests[0].body, Some(serde_json::json!({"grant":"opaque-grant"})));
    }

    /// Test group: backend URLs can never smuggle credentials into requests or diagnostics.
    #[test]
    fn backend_url_rejects_credentials_query_and_fragment() {
        for base in [
            "https://user:secret@backend.example",
            "https://backend.example?token=secret",
            "https://backend.example/#secret",
        ] {
            let error = Client::new(base, Auth::new("jwt.secret.signature").unwrap()).err().unwrap();
            assert!(matches!(error, Error::Local(_)));
            assert!(!error.to_string().contains("secret"));
        }
    }

    #[test]
    fn backend_url_requires_tls_except_for_explicit_loopback_development() {
        for base in ["https://backend.example", "http://localhost:8080", "http://127.0.0.1:8080", "http://[::1]:8080"] {
            assert!(Client::new(base, Auth::new("jwt.secret.signature").unwrap()).is_ok(), "{base}");
        }
        for base in ["http://backend.example", "http://10.0.0.1:8080", "http://[fd00::1]:8080"] {
            assert!(Client::new(base, Auth::new("jwt.secret.signature").unwrap()).is_err(), "{base}");
        }
    }

    #[test]
    fn refresh_rejects_invalid_credentials() {
        let mut session: AuthSession = serde_json::from_value(serde_json::json!({
            "access_token": "jwt.access.signature", "refresh_token": "sar_refresh",
            "expires_at": "2030-03-17T17:46:40Z",
            "identity": {"id":"0080c488-a9e9-4c22-aa91-1c7a639f1d7b","name":"Silicon","kind":"silicon"},
            "services": ["session"]
        }))
        .unwrap();
        assert!(session.validate().is_ok());
        session.access_token = "jwt.bad.signature\nheader".into();
        assert!(session.validate().is_err());
    }

    #[test]
    fn fetch_uses_the_long_timeout_transport_contract() {
        let transport = Arc::new(TimedFetchTransport::default());
        let client = Client::with_transport(
            "https://backend.example",
            Auth::new("jwt.secret.signature").unwrap(),
            transport.clone(),
        )
        .unwrap();
        let response = client
            .fetch(&FetchRequest {
                urls: vec!["https://example.com".into()],
                format: FetchFormat::Markdown,
                links: false,
                image_links: false,
                ttl_seconds: None,
                timeout_ms: None,
                include_selectors: vec![],
                exclude_selectors: vec![],
            })
            .unwrap();
        assert!(response.items.is_empty());
        let calls = transport.0.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1, Duration::from_secs(4 * 60 * 60 + 30 * 60));
        assert!(calls[0].0.url.ends_with("/api/v1/fetch"));
    }
}
