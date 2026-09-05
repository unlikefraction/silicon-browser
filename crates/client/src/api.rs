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

/// Silicon Browser's complete public interface: explicit identity, optional org scope, no local
/// state. Clone is cheap and safe to share between synchronous workers.
#[derive(Clone)]
pub struct Client {
    base: String,
    auth: Auth,
    org: Option<String>,
    transport: Arc<dyn Transport>,
}

impl Client {
    pub fn new(base: impl Into<String>, auth: Auth) -> Result<Self, Error> {
        Self::with_transport(base, auth, Arc::new(HttpTransport::default()))
    }

    pub fn with_transport(base: impl Into<String>, auth: Auth, transport: Arc<dyn Transport>) -> Result<Self, Error> {
        Ok(Self { base: normalize_backend_url(&base.into())?, auth, org: None, transport })
    }

    pub fn org(mut self, org: impl Into<String>) -> Result<Self, Error> {
        let org = org.into();
        validate_id(&org, "org")?;
        self.org = Some(org);
        Ok(self)
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    pub fn org_id(&self) -> Option<&str> {
        self.org.as_deref()
    }

    // Authentication and identity ---------------------------------------------------------

    /// Exchange a single-use IAM short-lived token. This is deliberately an associated
    /// function: there is no bearer with which to construct a normal Client yet.
    pub fn exchange(base: impl Into<String>, request: &AuthExchangeRequest) -> Result<AuthSession, Error> {
        request.validate().map_err(validation)?;
        let placeholder = Auth::new("pre-auth")?;
        let client = Self::new(base, placeholder)?;
        let session: AuthSession =
            client.send_without_auth(Method::Post, "/api/v1/auth/exchange", Some(json(request)?))?;
        validate_auth_session(&session, &request.org_id)?;
        Ok(session)
    }

    pub fn refresh(base: impl Into<String>, request: &AuthRefreshRequest) -> Result<AuthSession, Error> {
        request.validate().map_err(validation)?;
        let placeholder = Auth::new("pre-auth")?;
        let client = Self::new(base, placeholder)?;
        let session: AuthSession =
            client.send_without_auth(Method::Post, "/api/v1/auth/refresh", Some(json(request)?))?;
        validate_auth_session(&session, &request.org_id)?;
        Ok(session)
    }

    /// Inspect the backend-owned recording delivery grant for this actor and organization.
    pub fn delivery_authorization(&self) -> Result<DeliveryAuthorization, Error> {
        self.get_scoped("/api/v1/auth/delivery")
    }

    /// Enroll a fresh, separate IAM short-lived token for background recording delivery.
    pub fn authorize_delivery(&self, request: &DeliveryAuthorizationRequest) -> Result<DeliveryAuthorization, Error> {
        request.validate().map_err(validation)?;
        self.post_scoped("/api/v1/auth/delivery", request)
    }

    pub fn end_delivery_authorization(&self) -> Result<DeliveryAuthorization, Error> {
        self.post_scoped("/api/v1/auth/delivery/end", &Value::Null)
    }

    pub fn me(&self) -> Result<Identity, Error> {
        self.get_scoped("/api/v1/me")
    }

    /// Return the services enabled by the backend right now. This is queried
    /// live instead of trusting capabilities cached at token-exchange time.
    pub fn services(&self) -> Result<Vec<String>, Error> {
        self.get_scoped("/api/v1/services")
    }

    pub fn orgs(&self) -> Result<Vec<Org>, Error> {
        self.get("/api/v1/orgs")
    }

    // Profiles and proxy locations --------------------------------------------------------

    pub fn proxy_locations(&self) -> Result<Vec<ProxyLocation>, Error> {
        self.get_scoped("/api/v1/proxy-locations")
    }

    pub fn profiles(&self, filter: Option<&str>) -> Result<Vec<Profile>, Error> {
        self.get_scoped(&with_filter("/api/v1/profiles", filter))
    }

    pub fn profile(&self, profile_id: &str) -> Result<Profile, Error> {
        self.get_scoped(&format!("/api/v1/profiles/{}", segment(profile_id)?))
    }

    pub fn create_profile(&self, request: &ProfileCreate) -> Result<Profile, Error> {
        request.validate().map_err(validation)?;
        self.post_scoped("/api/v1/profiles", request)
    }

    pub fn update_profile(&self, profile_id: &str, request: &ProfileUpdate) -> Result<Profile, Error> {
        request.validate().map_err(validation)?;
        self.patch_scoped(&format!("/api/v1/profiles/{}", segment(profile_id)?), request)
    }

    pub fn end_profile(&self, profile_id: &str, request: &ProfileEnd) -> Result<Profile, Error> {
        request.validate().map_err(validation)?;
        self.post_scoped(&format!("/api/v1/profiles/{}/end", segment(profile_id)?), request)
    }

    // Sessions and execution --------------------------------------------------------------

    pub fn sessions(&self, filter: Option<&str>) -> Result<Vec<Session>, Error> {
        self.get_scoped(&with_filter("/api/v1/sessions", filter))
    }

    pub fn session(&self, session_id: &str) -> Result<Session, Error> {
        self.get_scoped(&format!("/api/v1/sessions/{}", segment(session_id)?))
    }

    pub fn create_session(&self, request: &SessionCreate) -> Result<Session, Error> {
        request.validate().map_err(validation)?;
        self.post_scoped("/api/v1/sessions", request)
    }

    pub fn end_session(&self, session_id: &str, request: &SessionEnd) -> Result<Session, Error> {
        request.validate().map_err(validation)?;
        self.post_scoped(&format!("/api/v1/sessions/{}/end", segment(session_id)?), request)
    }

    pub fn live(&self, session_id: &str) -> Result<LiveLink, Error> {
        self.post_scoped(&format!("/api/v1/sessions/{}/live", segment(session_id)?), &Value::Null)
    }

    /// Redeem a live-link fragment after the viewer authenticates. Successful
    /// redemption also associates that viewer with the managed session.
    pub fn redeem_live(&self, session_id: &str, request: &LiveRedeemRequest) -> Result<LiveLink, Error> {
        request.validate().map_err(validation)?;
        self.post_scoped(&format!("/api/v1/sessions/{}/live/redeem", segment(session_id)?), request)
    }

    pub fn session_logs(&self, session_id: &str, date: Option<&str>) -> Result<Vec<SessionLog>, Error> {
        let mut path = format!("/api/v1/sessions/{}/logs", segment(session_id)?);
        let date = match date {
            Some(date) => chrono::NaiveDate::parse_from_str(date, "%d-%m-%Y")
                .map_err(|_| Error::Local("date must be a valid DD-MM-YYYY value".into()))?,
            None => chrono::Utc::now().date_naive(),
        };
        path.push_str("?date=");
        path.push_str(&encode(&date.format("%Y-%m-%d").to_string()));
        self.get_scoped(&path)
    }

    /// Obtain a short-lived direct browser connection after authorization. Treat its URL as a
    /// credential. Browser control goes directly from the local controller to this endpoint.
    pub fn session_connection(&self, session_id: &str) -> Result<SessionConnection, Error> {
        let connection: SessionConnection =
            self.get_scoped(&format!("/api/v1/sessions/{}/connection", segment(session_id)?))?;
        if connection.session_id != session_id || connection.principal_id.trim().is_empty() {
            return Err(Error::Protocol("session connection identity did not match the request".into()));
        }
        crate::controller::validate_connection(&connection)?;
        Ok(connection)
    }

    /// Submit cooperative telemetry for an already executed local command. This endpoint never
    /// executes browser actions; callers can safely retry the exact report with the same UUID.
    pub fn report_command(&self, session_id: &str, report: &CommandReport) -> Result<CommandReportReceipt, Error> {
        report.validate().map_err(validation)?;
        let receipt: CommandReportReceipt = self.post_scoped_with_timeout(
            &format!("/api/v1/sessions/{}/commands", segment(session_id)?),
            report,
            Duration::from_secs(2),
        )?;
        if receipt.command_id != report.command_id {
            return Err(Error::Protocol("command receipt ID did not match its report".into()));
        }
        Ok(receipt)
    }

    // Recordings and usage ---------------------------------------------------------------

    pub fn recordings(&self, filter: Option<&str>) -> Result<Vec<Recording>, Error> {
        self.get_scoped(&with_filter("/api/v1/recordings", filter))
    }

    pub fn recording(&self, session_id: &str) -> Result<Recording, Error> {
        self.get_scoped(&format!("/api/v1/recordings/{}", segment(session_id)?))
    }

    /// Request another attempt for failed recording artifacts. The backend verifies the
    /// initiating identity's grant and retry eligibility; completed artifacts stay intact.
    pub fn retry_recording_delivery(&self, session_id: &str) -> Result<Recording, Error> {
        self.post_scoped(&format!("/api/v1/recordings/{}/retry", segment(session_id)?), &Value::Null)
    }

    pub fn trash_recording(&self, session_id: &str) -> Result<Recording, Error> {
        self.post_scoped(&format!("/api/v1/recordings/{}/trash", segment(session_id)?), &Value::Null)
    }

    pub fn usage(&self, session_id: &str) -> Result<Usage, Error> {
        self.get_scoped(&format!("/api/v1/usage/{}", segment(session_id)?))
    }

    pub fn usage_list(&self, filter: Option<&str>) -> Result<Vec<Usage>, Error> {
        self.get_scoped(&with_filter("/api/v1/usage", filter))
    }

    /// Current configured account capacity, shared across organizations.
    /// The backend refreshes the authoritative account snapshot at most once per minute.
    pub fn usage_limits(&self) -> Result<UsageLimits, Error> {
        self.get_scoped("/api/v1/usage/limits")
    }

    pub fn org_usage(&self, filter: Option<&str>) -> Result<UsageTotal, Error> {
        self.get_scoped(&with_filter("/api/v1/usage/org", filter))
    }

    // Read-heavy discovery ---------------------------------------------------------------

    pub fn search(&self, request: &SearchRequest) -> Result<SearchResponse, Error> {
        request.validate().map_err(validation)?;
        self.post_scoped("/api/v1/search", request)
    }

    pub fn fetch(&self, request: &FetchRequest) -> Result<FetchResponse, Error> {
        request.validate().map_err(validation)?;
        self.post_scoped_with_timeout("/api/v1/fetch", request, FETCH_REQUEST_TIMEOUT)
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        self.send(Method::Get, path, None, false)
    }

    fn get_scoped<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        self.require_org()?;
        self.send(Method::Get, path, None, true)
    }

    fn post_scoped<I: Serialize + ?Sized, O: DeserializeOwned>(&self, path: &str, body: &I) -> Result<O, Error> {
        self.require_org()?;
        self.send(Method::Post, path, Some(json(body)?), true)
    }

    fn post_scoped_with_timeout<I: Serialize + ?Sized, O: DeserializeOwned>(
        &self,
        path: &str,
        body: &I,
        timeout: Duration,
    ) -> Result<O, Error> {
        self.require_org()?;
        let request = self.request(Method::Post, path, Some(json(body)?), true)?;
        let response = self.transport.send_with_timeout(request, timeout)?;
        decode_response(response)
    }

    fn patch_scoped<I: Serialize + ?Sized, O: DeserializeOwned>(&self, path: &str, body: &I) -> Result<O, Error> {
        self.require_org()?;
        self.send(Method::Patch, path, Some(json(body)?), true)
    }

    fn send<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        scoped: bool,
    ) -> Result<T, Error> {
        let request = self.request(method, path, body, scoped)?;
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
            org: None,
            body,
        })?;
        decode_response(response)
    }

    fn request(&self, method: Method, path: &str, body: Option<Value>, scoped: bool) -> Result<Request, Error> {
        if !path.starts_with('/') {
            return Err(Error::Local("API path must be absolute".into()));
        }
        Ok(Request {
            method,
            url: format!("{}{}", self.base, path),
            bearer: Some(self.auth.expose().to_owned()),
            org: scoped.then(|| self.org.clone()).flatten(),
            body,
        })
    }

    fn require_org(&self) -> Result<&str, Error> {
        self.org.as_deref().ok_or_else(|| Error::Local("no organization selected; call Client::org first".into()))
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

fn validate_auth_session(session: &AuthSession, expected_org: &str) -> Result<(), Error> {
    session.validate().map_err(protocol)?;
    if expected_org != session.org.id {
        return Err(Error::Protocol("authentication response was bound to a different organization".into()));
    }
    Ok(())
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

    /// Test group: the stateless package forms scoped, authenticated requests exactly once.
    #[test]
    fn list_profiles_carries_explicit_auth_org_and_encoded_filter() {
        let transport = Arc::new(FakeTransport::default());
        let client =
            Client::with_transport("https://backend.example/", Auth::new("oat_secret").unwrap(), transport.clone())
                .unwrap()
                .org("tos")
                .unwrap();
        let profiles = client.profiles(Some("name:market research -> is:mine")).unwrap();
        assert!(profiles.is_empty());
        let requests = transport.0.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].bearer.as_deref(), Some("oat_secret"));
        assert_eq!(requests[0].org.as_deref(), Some("tos"));
        assert!(requests[0].url.ends_with("filter=name%3Amarket+research+-%3E+is%3Amine"));
    }

    /// Test group: org-scoped operations fail locally before touching transport.
    #[test]
    fn profile_calls_require_an_explicit_org() {
        let transport = Arc::new(FakeTransport::default());
        let client =
            Client::with_transport("https://backend.example", Auth::new("oat_secret").unwrap(), transport.clone())
                .unwrap();
        assert_eq!(client.profiles(None).unwrap_err().to_string(), "no organization selected; call Client::org first");
        assert!(transport.0.lock().unwrap().is_empty());
    }

    #[test]
    fn delivery_authorization_requires_org_and_rejects_an_invalid_token_locally() {
        let transport = Arc::new(FakeTransport::default());
        let client =
            Client::with_transport("https://backend.example", Auth::new("oat_secret").unwrap(), transport.clone())
                .unwrap();
        assert!(matches!(client.delivery_authorization(), Err(Error::Local(_))));
        assert!(matches!(client.end_delivery_authorization(), Err(Error::Local(_))));
        let client = client.org("org-1").unwrap();
        assert!(client.authorize_delivery(&DeliveryAuthorizationRequest { short_lived_token: String::new() }).is_err());
        assert!(transport.0.lock().unwrap().is_empty());
    }

    /// Test group: command execution cannot send a bearer before the caller selects an org.
    #[test]
    fn run_requires_an_explicit_org_before_transport() {
        let transport = Arc::new(FakeTransport::default());
        let client =
            Client::with_transport("https://backend.example", Auth::new("oat_secret").unwrap(), transport.clone())
                .unwrap();
        let error = client.session_connection("session-1").unwrap_err();
        assert!(error.to_string().contains("no organization selected"));
        assert!(transport.0.lock().unwrap().is_empty());
    }

    /// Test group: URL normalization cannot reinterpret a caller's ID as a parent endpoint.
    #[test]
    fn dot_segment_resource_ids_are_rejected_before_transport() {
        let transport = Arc::new(FakeTransport::default());
        let client =
            Client::with_transport("https://backend.example", Auth::new("oat_secret").unwrap(), transport.clone())
                .unwrap()
                .org("tos")
                .unwrap();
        for id in [".", ".."] {
            assert!(matches!(client.profile(id), Err(Error::Local(_))));
            assert!(matches!(client.session(id), Err(Error::Local(_))));
            assert!(matches!(client.trash_recording(id), Err(Error::Local(_))));
            assert!(matches!(client.retry_recording_delivery(id), Err(Error::Local(_))));
        }
        assert!(transport.0.lock().unwrap().is_empty());
    }

    /// Test group: capability discovery is an authenticated, org-scoped live
    /// backend query rather than client-side credential metadata.
    #[test]
    fn services_uses_the_live_scoped_endpoint() {
        let transport = Arc::new(FakeTransport::default());
        let client =
            Client::with_transport("https://backend.example", Auth::new("oat_secret").unwrap(), transport.clone())
                .unwrap()
                .org("tos")
                .unwrap();
        assert!(client.services().unwrap().is_empty());
        let requests = transport.0.lock().unwrap();
        assert!(requests[0].url.ends_with("/api/v1/services"));
        assert_eq!(requests[0].org.as_deref(), Some("tos"));
        assert_eq!(requests[0].bearer.as_deref(), Some("oat_secret"));
    }

    /// Test group: live-link redemption remains an authenticated, org-scoped
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
        let client =
            Client::with_transport("https://backend.example", Auth::new("oat_secret").unwrap(), transport.clone())
                .unwrap()
                .org("tos")
                .unwrap();
        let live = client.redeem_live("session-1", &LiveRedeemRequest { grant: "opaque-grant".into() }).unwrap();
        assert_eq!(live.session_id, "session-1");

        let requests = transport.0.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].url.ends_with("/api/v1/sessions/session-1/live/redeem"));
        assert_eq!(requests[0].bearer.as_deref(), Some("oat_secret"));
        assert_eq!(requests[0].org.as_deref(), Some("tos"));
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
            let error = Client::new(base, Auth::new("oat_secret").unwrap()).err().unwrap();
            assert!(matches!(error, Error::Local(_)));
            assert!(!error.to_string().contains("secret"));
        }
    }

    #[test]
    fn backend_url_requires_tls_except_for_explicit_loopback_development() {
        for base in ["https://backend.example", "http://localhost:8080", "http://127.0.0.1:8080", "http://[::1]:8080"] {
            assert!(Client::new(base, Auth::new("oat_secret").unwrap()).is_ok(), "{base}");
        }
        for base in ["http://backend.example", "http://10.0.0.1:8080", "http://[fd00::1]:8080"] {
            assert!(Client::new(base, Auth::new("oat_secret").unwrap()).is_err(), "{base}");
        }
    }

    /// Test group: auth helpers validate the server's opaque token families and
    /// preserve the caller's requested organization binding.
    #[test]
    fn refresh_rejects_a_cross_org_authentication_response() {
        let mut session: AuthSession = serde_json::from_value(serde_json::json!({
            "access_token": "oat_access",
            "refresh_token": "ort_refresh",
            "expires_at": "2030-03-17T17:46:40Z",
            "identity": {"id":"silicon-1","name":"Silicon","kind":"silicon"},
            "org": {"id":"other-org","name":"Other"},
            "services": ["session"]
        }))
        .unwrap();
        let error = validate_auth_session(&session, "expected-org").unwrap_err();
        assert!(matches!(error, Error::Protocol(_)));
        session.org.id = "expected-org".into();
        session.access_token = "oat_bad\nheader".into();
        assert!(validate_auth_session(&session, "expected-org").is_err());
    }

    /// Test group: the documented log default is today's UTC date, sent explicitly so backend
    /// locale and timezone cannot change the result.
    #[test]
    fn session_logs_default_to_today_in_utc() {
        let before = chrono::Utc::now().date_naive();
        let transport = Arc::new(FakeTransport::default());
        let client =
            Client::with_transport("https://backend.example", Auth::new("oat_secret").unwrap(), transport.clone())
                .unwrap()
                .org("tos")
                .unwrap();
        let logs = client.session_logs("s1", None).unwrap();
        let after = chrono::Utc::now().date_naive();
        assert!(logs.is_empty());
        let url = &transport.0.lock().unwrap()[0].url;
        assert!(
            url.ends_with(&format!("?date={before}")) || url.ends_with(&format!("?date={after}")),
            "unexpected log URL: {url}"
        );
    }

    #[test]
    fn fetch_uses_the_long_timeout_transport_contract() {
        let transport = Arc::new(TimedFetchTransport::default());
        let client =
            Client::with_transport("https://backend.example", Auth::new("oat_secret").unwrap(), transport.clone())
                .unwrap()
                .org("tos")
                .unwrap();
        let response = client
            .fetch(&FetchRequest {
                urls: vec!["https://example.com".into()],
                purpose: "read the page".into(),
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
