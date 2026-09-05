use std::collections::BTreeMap;

use async_trait::async_trait;
use reqwest::header::{CONTENT_TYPE, HeaderValue};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::url_policy::is_https_or_loopback_http;

use super::error::{ProviderError, ProviderResult, json_response, json_response_with_limit, transport};
use super::proxy::is_proxy_location;

const PROVIDER: &str = "browser-use";
const API_KEY_HEADER: &str = "X-Browser-Use-API-Key";
const PROFILE_RECONCILIATION_PAGE_SIZE: usize = 100;
const MAX_PROFILE_RECONCILIATION_PAGES: usize = 100;
const SESSION_METADATA_KEY: &str = "sb_session_id";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateBrowserProfile {
    pub name: String,
    /// Stable Silicon Browser profile id. Browser Use calls this `userId`; it is also the
    /// reconciliation key after an ambiguous create response.
    pub user_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpdateBrowserProfile {
    pub name: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Browser Use does not expose a stable browser fingerprint on a profile and randomizes
/// anti-detect browser attributes per session. The public profile `fingerprint` is therefore a
/// Silicon-managed opaque logical id, generated once by the service and never inferred here.
pub struct ProviderProfile {
    pub id: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub last_used_at: Option<String>,
    #[serde(default, deserialize_with = "deserialize_cookie_domains")]
    pub cookie_domains: Vec<String>,
}

// New provider profiles return null until cookies have been recorded. Keep the
// adapter's collection shape while accepting both absent and null metadata.
fn deserialize_cookie_domains<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    Ok(Option::<Vec<String>>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartBrowser {
    pub profile_id: Option<String>,
    pub proxy_country_code: Option<String>,
    pub timeout_minutes: u16,
    pub enable_recording: bool,
    /// Stable local id sent as provider metadata for recovery after an ambiguous create response.
    pub reconciliation_id: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderBrowserSession {
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub timeout_at: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub live_url: Option<String>,
    #[serde(default)]
    pub cdp_url: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default = "zero_decimal")]
    pub proxy_used_mb: String,
    #[serde(default = "zero_decimal")]
    pub proxy_cost: String,
    #[serde(default = "zero_decimal")]
    pub browser_cost: String,
    #[serde(default)]
    pub agent_session_id: Option<String>,
    #[serde(default)]
    pub recording_url: Option<String>,
    /// None preserves compatibility with providers predating the terminal readiness signal.
    #[serde(default)]
    pub recording_available: Option<bool>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

impl std::fmt::Debug for ProviderBrowserSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderBrowserSession")
            .field("id", &self.id)
            .field("status", &self.status)
            .field("timeout_at", &self.timeout_at)
            .field("started_at", &self.started_at)
            .field("live_url", &self.live_url.as_ref().map(|_| "[REDACTED]"))
            .field("cdp_url", &self.cdp_url.as_ref().map(|_| "[REDACTED]"))
            .field("finished_at", &self.finished_at)
            .field("proxy_used_mb", &self.proxy_used_mb)
            .field("proxy_cost", &self.proxy_cost)
            .field("browser_cost", &self.browser_cost)
            .field("agent_session_id", &self.agent_session_id)
            .field("recording_url", &self.recording_url.as_ref().map(|_| "[REDACTED]"))
            .field("recording_available", &self.recording_available)
            .finish()
    }
}

fn zero_decimal() -> String {
    "0".to_owned()
}

/// Deliberately decode only capacity fields from the account response. Payment
/// details, balances, identifiers, and global activity never cross this boundary.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ProviderAccountLimits {
    #[serde(rename = "concurrentSessionLimit")]
    pub concurrent_browser_limit: u64,
    #[serde(default, rename = "rateLimit")]
    pub rate_limit: Option<u64>,
}

#[async_trait]
pub trait BrowserProvider: Send + Sync {
    async fn account_limits(&self) -> ProviderResult<ProviderAccountLimits> {
        Err(ProviderError::Unsupported { provider: "browser", feature: "account limits" })
    }
    async fn create_profile(&self, request: CreateBrowserProfile) -> ProviderResult<ProviderProfile>;
    async fn update_profile(&self, id: &str, request: UpdateBrowserProfile) -> ProviderResult<ProviderProfile>;
    async fn start_browser(&self, request: StartBrowser) -> ProviderResult<ProviderBrowserSession>;
    async fn get_browser(&self, id: &str) -> ProviderResult<ProviderBrowserSession>;
    async fn stop_browser(&self, id: &str) -> ProviderResult<ProviderBrowserSession>;
    /// Browser Use v3 documents no idempotency header for profile creation. Call this with the
    /// stable `userId` before retrying an ambiguous POST; duplicates are reported as an error.
    async fn find_profile_by_user_id(&self, user_id: &str) -> ProviderResult<Option<ProviderProfile>>;
    /// Recover an ambiguous creation through its exact metadata label; reject duplicate matches.
    /// This is correlation, not provider-enforced POST idempotency.
    async fn find_browser_by_session_id(&self, session_id: &str) -> ProviderResult<Option<ProviderBrowserSession>>;
}

/// Browser Use's direct-browser v3 API. Its API key never leaves this adapter.
#[derive(Clone)]
pub struct BrowserUseV3 {
    http: reqwest::Client,
    base: Url,
    api_key: HeaderValue,
}

impl std::fmt::Debug for BrowserUseV3 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserUseV3").field("base", &self.base).field("api_key", &"[REDACTED]").finish_non_exhaustive()
    }
}

impl BrowserUseV3 {
    pub fn new(api_key: impl AsRef<str>) -> ProviderResult<Self> {
        Self::with_base_url(api_key, "https://api.browser-use.com")
    }

    /// Alternate endpoint for a compatible deployment or a local contract test.
    pub fn with_base_url(api_key: impl AsRef<str>, base: impl AsRef<str>) -> ProviderResult<Self> {
        let base = endpoint(base.as_ref())?;
        let mut api_key = HeaderValue::from_str(api_key.as_ref())
            .map_err(|_| ProviderError::InvalidInput("Browser Use API key is not a valid header value".into()))?;
        if api_key.is_empty() {
            return Err(ProviderError::InvalidInput("Browser Use API key is empty".into()));
        }
        api_key.set_sensitive(true);
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(70))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| transport(PROVIDER, error))?;
        Ok(Self { http, base, api_key })
    }

    fn url(&self, path: &str) -> ProviderResult<Url> {
        self.base
            .join(path.trim_start_matches('/'))
            .map_err(|error| ProviderError::InvalidInput(format!("invalid Browser Use endpoint path: {error}")))
    }

    fn request(&self, method: reqwest::Method, path: &str) -> ProviderResult<reqwest::RequestBuilder> {
        Ok(self
            .http
            .request(method, self.url(path)?)
            .header(API_KEY_HEADER, self.api_key.clone())
            .header(CONTENT_TYPE, "application/json"))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateProfileBody<'a> {
    name: &'a str,
    user_id: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProfileBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_id: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StartBrowserBody<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    profile_id: Option<&'a str>,
    proxy_country_code: Option<&'a str>,
    timeout: u16,
    enable_recording: bool,
    metadata: BTreeMap<&'static str, &'a str>,
}

#[derive(Serialize)]
struct StopBrowserBody {
    action: &'static str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderPage<T> {
    items: Vec<T>,
    total_items: usize,
    page_number: usize,
    page_size: usize,
}

#[async_trait]
impl BrowserProvider for BrowserUseV3 {
    async fn account_limits(&self) -> ProviderResult<ProviderAccountLimits> {
        let response = self
            .request(reqwest::Method::GET, "api/v3/billing/account")?
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|error| transport(PROVIDER, error))?;
        json_response_with_limit(PROVIDER, response, 64 * 1024).await
    }

    async fn create_profile(&self, request: CreateBrowserProfile) -> ProviderResult<ProviderProfile> {
        if request.name.trim().is_empty() || request.name.chars().count() > 100 {
            return Err(ProviderError::InvalidInput("profile name must be 1..=100 characters".into()));
        }
        if request.user_id.trim().is_empty() || request.user_id.len() > 255 {
            return Err(ProviderError::InvalidInput("profile user id must be 1..=255 bytes".into()));
        }
        let response = self
            .request(reqwest::Method::POST, "api/v3/profiles")?
            .json(&CreateProfileBody { name: &request.name, user_id: &request.user_id })
            .send()
            .await
            .map_err(|error| transport(PROVIDER, error))?;
        json_response(PROVIDER, response).await
    }

    async fn update_profile(&self, id: &str, request: UpdateBrowserProfile) -> ProviderResult<ProviderProfile> {
        validate_path_id(id, "profile")?;
        if request.name.is_none() && request.user_id.is_none() {
            return Err(ProviderError::InvalidInput("profile update is empty".into()));
        }
        if request.name.as_ref().is_some_and(|name| name.trim().is_empty() || name.chars().count() > 100) {
            return Err(ProviderError::InvalidInput("profile name must be 1..=100 characters".into()));
        }
        if request.user_id.as_ref().is_some_and(|id| id.trim().is_empty() || id.len() > 255) {
            return Err(ProviderError::InvalidInput("profile user id must be 1..=255 bytes".into()));
        }
        let path = format!("api/v3/profiles/{id}");
        let response = self
            .request(reqwest::Method::PATCH, &path)?
            .json(&UpdateProfileBody { name: request.name.as_deref(), user_id: request.user_id.as_deref() })
            .send()
            .await
            .map_err(|error| transport(PROVIDER, error))?;
        json_response(PROVIDER, response).await
    }

    async fn start_browser(&self, request: StartBrowser) -> ProviderResult<ProviderBrowserSession> {
        validate_browser_start(&request)?;
        let proxy_country_code = request.proxy_country_code.as_deref().map(str::to_ascii_lowercase);
        let body = StartBrowserBody {
            profile_id: request.profile_id.as_deref(),
            proxy_country_code: proxy_country_code.as_deref(),
            timeout: request.timeout_minutes,
            enable_recording: request.enable_recording,
            metadata: BTreeMap::from([(SESSION_METADATA_KEY, request.reconciliation_id.as_str())]),
        };
        let response = self
            .request(reqwest::Method::POST, "api/v3/browsers")?
            .json(&body)
            .send()
            .await
            .map_err(|error| transport(PROVIDER, error))?;
        json_response(PROVIDER, response).await
    }

    async fn get_browser(&self, id: &str) -> ProviderResult<ProviderBrowserSession> {
        validate_path_id(id, "browser")?;
        let path = format!("api/v3/browsers/{id}");
        let response =
            self.request(reqwest::Method::GET, &path)?.send().await.map_err(|error| transport(PROVIDER, error))?;
        json_response(PROVIDER, response).await
    }

    async fn stop_browser(&self, id: &str) -> ProviderResult<ProviderBrowserSession> {
        validate_path_id(id, "browser")?;
        let path = format!("api/v3/browsers/{id}");
        let response = self
            .request(reqwest::Method::PATCH, &path)?
            .json(&StopBrowserBody { action: "stop" })
            .send()
            .await
            .map_err(|error| transport(PROVIDER, error))?;
        json_response(PROVIDER, response).await
    }

    async fn find_profile_by_user_id(&self, user_id: &str) -> ProviderResult<Option<ProviderProfile>> {
        validate_reconciliation_value(user_id, "profile user id")?;
        let mut exact = Vec::new();
        let mut page_number = 1_usize;
        let mut expected_total = None;
        let mut received_items = 0_usize;
        loop {
            if page_number > MAX_PROFILE_RECONCILIATION_PAGES {
                return Err(invalid_profile_pagination("profile search exceeded its bounded page count"));
            }
            let mut url = self.url("api/v3/profiles")?;
            {
                let mut query = url.query_pairs_mut();
                query.append_pair("query", user_id);
                query.append_pair("pageSize", &PROFILE_RECONCILIATION_PAGE_SIZE.to_string());
                query.append_pair("pageNumber", &page_number.to_string());
            }
            let response = self
                .http
                .get(url)
                .header(API_KEY_HEADER, self.api_key.clone())
                .send()
                .await
                .map_err(|error| transport(PROVIDER, error))?;
            let page: ProviderPage<ProviderProfile> = json_response(PROVIDER, response).await?;
            if page.page_number != page_number {
                return Err(invalid_profile_pagination("profile search returned a repeated or unexpected page number"));
            }
            if page.page_size != PROFILE_RECONCILIATION_PAGE_SIZE {
                return Err(invalid_profile_pagination("profile search did not honor the requested page size"));
            }
            if expected_total.is_some_and(|total| total != page.total_items) {
                return Err(invalid_profile_pagination("profile search changed totalItems between pages"));
            }
            expected_total.get_or_insert(page.total_items);
            let total_pages = page.total_items.div_ceil(PROFILE_RECONCILIATION_PAGE_SIZE);
            if total_pages > MAX_PROFILE_RECONCILIATION_PAGES {
                return Err(invalid_profile_pagination("profile search totalItems exceeded the reconciliation limit"));
            }
            let page_offset = (page_number - 1)
                .checked_mul(PROFILE_RECONCILIATION_PAGE_SIZE)
                .ok_or_else(|| invalid_profile_pagination("profile search page offset overflowed"))?;
            let expected_items_on_page = page.total_items.saturating_sub(page_offset).min(page.page_size);
            if page.items.len() != expected_items_on_page {
                return Err(invalid_profile_pagination("profile search item count contradicted its page metadata"));
            }
            received_items = received_items
                .checked_add(page.items.len())
                .ok_or_else(|| invalid_profile_pagination("profile search item count overflowed"))?;
            if received_items > page.total_items || page.items.len() > page.page_size {
                return Err(invalid_profile_pagination(
                    "profile search item counts contradicted its pagination metadata",
                ));
            }
            exact.extend(page.items.into_iter().filter(|profile| profile.user_id.as_deref() == Some(user_id)));
            if exact.len() > 1 {
                return Err(duplicate_reconciliation("profiles"));
            }
            if page_number >= total_pages.max(1) {
                if received_items != page.total_items {
                    return Err(invalid_profile_pagination(
                        "profile search returned fewer items than its totalItems metadata",
                    ));
                }
                return Ok(exact.pop());
            }
            page_number = page_number
                .checked_add(1)
                .ok_or_else(|| invalid_profile_pagination("profile search page number overflowed"))?;
        }
    }

    async fn find_browser_by_session_id(&self, session_id: &str) -> ProviderResult<Option<ProviderBrowserSession>> {
        validate_reconciliation_value(session_id, "session id")?;
        let mut exact = None;
        let mut expected_total = None;
        for page_number in 1..=MAX_PROFILE_RECONCILIATION_PAGES {
            let mut url = self.url("api/v3/browsers")?;
            url.query_pairs_mut()
                .append_pair("metadata", &format!("{SESSION_METADATA_KEY}={session_id}"))
                .append_pair("pageSize", &PROFILE_RECONCILIATION_PAGE_SIZE.to_string())
                .append_pair("pageNumber", &page_number.to_string());
            let response = self
                .http
                .get(url)
                .header(API_KEY_HEADER, self.api_key.clone())
                .send()
                .await
                .map_err(|error| transport(PROVIDER, error))?;
            let page: ProviderPage<ProviderBrowserSession> = json_response(PROVIDER, response).await?;
            if page.page_number != page_number || page.page_size != PROFILE_RECONCILIATION_PAGE_SIZE {
                return Err(invalid_profile_pagination("browser search returned unexpected pagination"));
            }
            if expected_total.is_some_and(|total| total != page.total_items) {
                return Err(invalid_profile_pagination("browser search changed totalItems between pages"));
            }
            expected_total.get_or_insert(page.total_items);
            let total_pages = page.total_items.div_ceil(PROFILE_RECONCILIATION_PAGE_SIZE).max(1);
            if total_pages > MAX_PROFILE_RECONCILIATION_PAGES {
                return Err(invalid_profile_pagination("browser search totalItems exceeded the reconciliation limit"));
            }
            let offset = (page_number - 1) * PROFILE_RECONCILIATION_PAGE_SIZE;
            if page.items.len() != page.total_items.saturating_sub(offset).min(page.page_size) {
                return Err(invalid_profile_pagination("browser search item count contradicted pagination"));
            }
            for browser in page.items {
                if browser.metadata.get(SESSION_METADATA_KEY).map(String::as_str) != Some(session_id) {
                    return Err(invalid_profile_pagination(
                        "browser search returned metadata outside the exact filter",
                    ));
                }
                if exact.replace(browser.id).is_some() {
                    return Err(duplicate_reconciliation("browsers"));
                }
            }
            if page_number == total_pages {
                return match exact {
                    None => Ok(None),
                    Some(id) => {
                        // List responses omit recording URLs; detail GET also validates that the
                        // selected provider identity still carries our exact correlation label.
                        let browser = self.get_browser(&id).await?;
                        if browser.id != id
                            || browser.metadata.get(SESSION_METADATA_KEY).map(String::as_str) != Some(session_id)
                        {
                            return Err(invalid_profile_pagination(
                                "browser detail did not match reconciliation identity",
                            ));
                        }
                        Ok(Some(browser))
                    }
                };
            }
        }
        Err(invalid_profile_pagination("browser search exceeded its bounded page count"))
    }
}

fn duplicate_reconciliation(kind: &'static str) -> ProviderError {
    ProviderError::InvalidResponse {
        provider: PROVIDER,
        message: format!("multiple {kind} matched one Silicon Browser reconciliation key"),
    }
}

fn invalid_profile_pagination(message: &'static str) -> ProviderError {
    ProviderError::InvalidResponse { provider: PROVIDER, message: message.into() }
}

fn validate_reconciliation_value(value: &str, kind: &str) -> ProviderResult<()> {
    if value.trim().is_empty() || value.len() > 255 || value.chars().any(char::is_control) {
        return Err(ProviderError::InvalidInput(format!("invalid {kind} for provider reconciliation")));
    }
    Ok(())
}

fn endpoint(raw: &str) -> ProviderResult<Url> {
    let mut url = Url::parse(raw)
        .map_err(|error| ProviderError::InvalidInput(format!("invalid Browser Use base URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.cannot_be_a_base()
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ProviderError::InvalidInput(
            "Browser Use base URL must be a credential-free http(s) base URL without query or fragment".into(),
        ));
    }
    if !is_https_or_loopback_http(&url) {
        return Err(ProviderError::InvalidInput(
            "Browser Use base URL must use HTTPS unless its host is loopback".into(),
        ));
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

fn validate_path_id(id: &str, kind: &str) -> ProviderResult<()> {
    if id.is_empty()
        || id.len() > 255
        || !id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ProviderError::InvalidInput(format!("invalid {kind} provider id")));
    }
    Ok(())
}

fn validate_browser_start(request: &StartBrowser) -> ProviderResult<()> {
    if !(1..=240).contains(&request.timeout_minutes) {
        return Err(ProviderError::InvalidInput("browser timeout must be between 1 and 240 minutes".into()));
    }
    if !request.enable_recording {
        return Err(ProviderError::InvalidInput("managed browser sessions must enable recording".into()));
    }
    match (&request.profile_id, &request.proxy_country_code) {
        (Some(_), Some(country)) if is_proxy_location(country) => {}
        (Some(_), Some(_)) => {
            return Err(ProviderError::InvalidInput("proxy country must be a two-letter ISO code".into()));
        }
        (Some(_), None) => {
            return Err(ProviderError::InvalidInput("profile sessions must use their pinned proxy location".into()));
        }
        (None, Some(_)) => {
            return Err(ProviderError::InvalidInput("incognito sessions cannot use a proxy".into()));
        }
        (None, None) => {}
    }
    validate_reconciliation_value(&request.reconciliation_id, "session id")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn account_limits_reject_missing_negative_fractional_or_string_capacity() {
        let invalid = [
            serde_json::json!({"rateLimit":3}),
            serde_json::json!({"concurrentSessionLimit":null}),
            serde_json::json!({"concurrentSessionLimit":-1}),
            serde_json::json!({"concurrentSessionLimit":3.5}),
            serde_json::json!({"concurrentSessionLimit":"500"}),
            serde_json::json!({"concurrentSessionLimit":3,"rateLimit":-1}),
        ];
        let count = invalid.len();
        let (base, mut captured, server) = super::super::test_http::spawn_json_server(
            invalid.into_iter().map(|body| (200, body.to_string())).collect(),
        )
        .await;
        let provider = BrowserUseV3::with_base_url("test-key", &base).unwrap();
        for _ in 0..count {
            assert!(matches!(provider.account_limits().await, Err(ProviderError::InvalidResponse { .. })));
            assert_eq!(captured.recv().await.unwrap().target, "/api/v3/billing/account");
        }
        server.await.unwrap();
    }

    fn profile_page(page_number: usize, total_items: usize, item_count: usize) -> String {
        let items = (0..item_count)
            .map(|index| {
                serde_json::json!({
                    "id": format!("profile-{page_number}-{index}"),
                    "userId": format!("other-{page_number}-{index}")
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "items": items,
            "totalItems": total_items,
            "pageNumber": page_number,
            "pageSize": PROFILE_RECONCILIATION_PAGE_SIZE
        })
        .to_string()
    }

    #[test]
    fn profile_cookie_domains_accept_null_missing_or_array_but_reject_wrong_types() {
        for value in [
            serde_json::json!({"id":"profile"}),
            serde_json::json!({"id":"profile","cookieDomains":null}),
            serde_json::json!({"id":"profile","cookieDomains":[]}),
        ] {
            assert!(serde_json::from_value::<ProviderProfile>(value).unwrap().cookie_domains.is_empty());
        }
        let populated: ProviderProfile = serde_json::from_value(serde_json::json!({
            "id":"profile", "cookieDomains":["example.com"]
        }))
        .unwrap();
        assert_eq!(populated.cookie_domains, ["example.com"]);
        for invalid in [serde_json::json!("example.com"), serde_json::json!([null]), serde_json::json!(42)] {
            assert!(
                serde_json::from_value::<ProviderProfile>(serde_json::json!({"id":"profile","cookieDomains":invalid}))
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn newly_created_profiles_with_null_cookie_domains_can_update_and_reconcile() {
        let profile = serde_json::json!({"id":"provider-profile", "userId":"profile-local", "name":"New profile",
            "createdAt":"2026-09-05T12:00:00Z", "updatedAt":"2026-09-05T12:00:00Z",
            "lastUsedAt":null, "cookieDomains":null});
        let page = serde_json::json!({"items":[profile.clone()], "totalItems":1,"pageNumber":1,"pageSize":100});
        let (base, mut captured, server) = super::super::test_http::spawn_json_server(vec![
            (201, profile.to_string()),
            (200, profile.to_string()),
            (200, page.to_string()),
        ])
        .await;
        let provider = BrowserUseV3::with_base_url("test-key", &base).unwrap();
        let created = provider
            .create_profile(CreateBrowserProfile { name: "New profile".into(), user_id: "profile-local".into() })
            .await
            .unwrap();
        assert!(created.cookie_domains.is_empty());
        assert_eq!(created.user_id.as_deref(), Some("profile-local"));
        let updated = provider
            .update_profile(&created.id, UpdateBrowserProfile { name: Some("New profile".into()), user_id: None })
            .await
            .unwrap();
        assert_eq!(updated, created);
        assert_eq!(provider.find_profile_by_user_id("profile-local").await.unwrap(), Some(created));
        assert_eq!(captured.recv().await.unwrap().method, "POST");
        assert_eq!(captured.recv().await.unwrap().method, "PATCH");
        assert_eq!(captured.recv().await.unwrap().method, "GET");
        server.await.unwrap();
    }

    #[test]
    fn alternate_endpoint_rejects_credentials_query_and_fragment() {
        for endpoint in
            ["https://user:password@example.com", "https://example.com?api_key=secret", "https://example.com#secret"]
        {
            let error = BrowserUseV3::with_base_url("test-key", endpoint).unwrap_err();
            assert!(matches!(error, ProviderError::InvalidInput(_)));
            assert!(!error.to_string().contains("password"));
            assert!(!error.to_string().contains("api_key"));
            assert!(!error.to_string().contains("secret"));
        }
    }

    #[test]
    fn alternate_endpoint_requires_tls_except_for_loopback_contract_tests() {
        for endpoint in ["http://api.example", "http://10.0.0.1"] {
            assert!(matches!(BrowserUseV3::with_base_url("test-key", endpoint), Err(ProviderError::InvalidInput(_))));
        }
        for endpoint in ["https://api.example", "http://localhost:8080", "http://127.0.0.1:8080", "http://[::1]:8080"] {
            assert!(BrowserUseV3::with_base_url("test-key", endpoint).is_ok(), "{endpoint}");
        }
    }

    #[test]
    fn test_group_browser_invariants_reject_invalid_proxy_and_reconciliation_id() {
        let incognito_with_proxy = StartBrowser {
            profile_id: None,
            proxy_country_code: Some("us".into()),
            timeout_minutes: 15,
            enable_recording: true,
            reconciliation_id: "session-local".into(),
        };
        assert!(validate_browser_start(&incognito_with_proxy).is_err());

        let missing_reconciliation_id = StartBrowser {
            profile_id: Some("upstream-profile".into()),
            proxy_country_code: Some("in".into()),
            timeout_minutes: 30,
            enable_recording: true,
            reconciliation_id: String::new(),
        };
        assert!(validate_browser_start(&missing_reconciliation_id).is_err());
    }

    #[test]
    fn test_group_browser_invariants_accepts_profile_and_incognito_shapes() {
        let profile = StartBrowser {
            profile_id: Some("upstream-profile".into()),
            proxy_country_code: Some("in".into()),
            timeout_minutes: 240,
            enable_recording: true,
            reconciliation_id: "session-profile".into(),
        };
        assert!(validate_browser_start(&profile).is_ok());

        let incognito = StartBrowser {
            profile_id: None,
            proxy_country_code: None,
            timeout_minutes: 15,
            enable_recording: true,
            reconciliation_id: "session-incognito".into(),
        };
        assert!(validate_browser_start(&incognito).is_ok());
    }

    #[tokio::test]
    async fn test_group_browser_v3_contract_redacts_urls_and_sends_only_documented_fields() {
        let secret_cdp = "wss://cdp.example/devtools?token=cdp-secret";
        let secret_live = "https://live.example/view?token=live-secret";
        let secret_recording = "https://recording.example/file?signature=recording-secret";
        let (base, mut captured, server) = super::super::test_http::spawn_json_server(vec![
            (
                201,
                serde_json::json!({
                    "id": "browser-id",
                    "status": "active",
                    "liveUrl": secret_live,
                    "cdpUrl": secret_cdp,
                    "recordingUrl": secret_recording
                })
                .to_string(),
            ),
            (401, format!(r#"{{"error":"invalid connection {secret_cdp}"}}"#)),
        ])
        .await;
        let provider = BrowserUseV3::with_base_url("test-key", &base).unwrap();
        let session = provider
            .start_browser(StartBrowser {
                profile_id: Some("profile-id".into()),
                proxy_country_code: Some("us".into()),
                timeout_minutes: 30,
                enable_recording: true,
                reconciliation_id: "session-local".into(),
            })
            .await
            .unwrap();
        let debug = format!("{session:?}");
        assert!(!debug.contains("cdp-secret"));
        assert!(!debug.contains("live-secret"));
        assert!(!debug.contains("recording-secret"));

        let request = captured.recv().await.unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, "/api/v3/browsers");
        assert!(request.headers.to_ascii_lowercase().contains("x-browser-use-api-key: test-key"));
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "profileId": "profile-id",
                "proxyCountryCode": "us",
                "timeout": 30,
                "enableRecording": true,
                "metadata": {"sb_session_id": "session-local"}
            })
        );

        let error = provider.get_browser("browser-id").await.unwrap_err();
        server.await.unwrap();
        let _ = captured.recv().await.unwrap();
        let shown = error.to_string();
        assert!(shown.contains("HTTP 401"));
        assert!(shown.contains("redacted"));
        assert!(!shown.contains("cdp-secret"));
    }

    #[tokio::test]
    async fn incognito_browser_wire_contract_sends_an_explicit_null_proxy() {
        let (base, mut captured, server) = super::super::test_http::spawn_json_server(vec![(
            201,
            serde_json::json!({
                "id": "incognito-browser",
                "status": "active"
            })
            .to_string(),
        )])
        .await;
        let provider = BrowserUseV3::with_base_url("test-key", &base).unwrap();

        provider
            .start_browser(StartBrowser {
                profile_id: None,
                proxy_country_code: None,
                timeout_minutes: 15,
                enable_recording: true,
                reconciliation_id: "session-incognito".into(),
            })
            .await
            .unwrap();

        let request = captured.recv().await.unwrap();
        server.await.unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, "/api/v3/browsers");
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "proxyCountryCode": null,
                "timeout": 15,
                "enableRecording": true,
                "metadata": {"sb_session_id": "session-incognito"}
            })
        );
    }

    #[tokio::test]
    async fn test_group_profile_reconciliation_is_exact() {
        let (base, mut captured, server) = super::super::test_http::spawn_json_server(vec![(
            200,
            serde_json::json!({
                "items": [
                    {"id": "fuzzy", "userId": "profile-local-extra"},
                    {"id": "exact", "userId": "profile-local"}
                ],
                "totalItems": 2,
                "pageNumber": 1,
                "pageSize": 100
            })
            .to_string(),
        )])
        .await;
        let provider = BrowserUseV3::with_base_url("test-key", &base).unwrap();
        assert_eq!(provider.find_profile_by_user_id("profile-local").await.unwrap().unwrap().id, "exact");

        let profile_request = captured.recv().await.unwrap();
        server.await.unwrap();
        let profile_url = Url::parse(&format!("http://test.invalid{}", profile_request.target)).unwrap();
        let profile_query = profile_url.query_pairs().into_owned().collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(profile_query["query"], "profile-local");
    }

    fn browser_page(items: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"totalItems": items.as_array().unwrap().len(), "items": items,
            "pageNumber": 1, "pageSize": 100})
    }

    #[test]
    fn browser_optional_readiness_and_metadata_preserve_legacy_responses() {
        let browser: ProviderBrowserSession = serde_json::from_value(serde_json::json!({
            "id": "legacy", "status": "stopped"
        }))
        .unwrap();
        assert_eq!(browser.recording_available, None);
        assert!(browser.metadata.is_empty());
        let browser: ProviderBrowserSession = serde_json::from_value(serde_json::json!({
            "id": "new", "status": "stopped", "recordingAvailable": false,
            "metadata": {"sb_session_id": "local"}
        }))
        .unwrap();
        assert_eq!(browser.recording_available, Some(false));
        assert_eq!(browser.metadata[SESSION_METADATA_KEY], "local");
    }

    #[tokio::test]
    async fn browser_reconciliation_filters_exact_metadata_and_fetches_recording_detail() {
        let item = serde_json::json!({"id":"remote", "status":"stopped",
            "metadata":{"sb_session_id":"local &=value"}});
        let mut detail = item.clone();
        detail["recordingUrl"] = serde_json::json!("https://recording.example/video.mp4");
        detail["recordingAvailable"] = serde_json::json!(true);
        let (base, mut captured, server) = super::super::test_http::spawn_json_server(vec![
            (200, browser_page(serde_json::json!([item])).to_string()),
            (200, detail.to_string()),
        ])
        .await;
        let provider = BrowserUseV3::with_base_url("test-key", base).unwrap();
        let browser = provider.find_browser_by_session_id("local &=value").await.unwrap().unwrap();
        assert_eq!(browser.id, "remote");
        assert!(browser.recording_url.is_some());
        assert_eq!(browser.recording_available, Some(true));
        let list = captured.recv().await.unwrap();
        let url = Url::parse(&format!("http://example.test{}", list.target)).unwrap();
        let query = url.query_pairs().into_owned().collect::<BTreeMap<_, _>>();
        assert_eq!(query.len(), 3);
        assert_eq!(query["metadata"], "sb_session_id=local &=value");
        assert_eq!(query["pageSize"], "100");
        assert_eq!(query["pageNumber"], "1");
        assert_eq!(captured.recv().await.unwrap().target, "/api/v3/browsers/remote");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn browser_reconciliation_empty_search_is_absent() {
        let (base, _captured, server) =
            super::super::test_http::spawn_json_server(vec![(200, browser_page(serde_json::json!([])).to_string())])
                .await;
        assert!(
            BrowserUseV3::with_base_url("test-key", base)
                .unwrap()
                .find_browser_by_session_id("local")
                .await
                .unwrap()
                .is_none()
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn browser_reconciliation_rejects_nonmatches_duplicates_and_malformed_pages() {
        let item = serde_json::json!({"id":"remote", "status":"active",
            "metadata":{"sb_session_id":"local"}});
        let mut repeated_page = browser_page(serde_json::json!([item]));
        repeated_page["pageNumber"] = serde_json::json!(2);
        let mut wrong_count = browser_page(serde_json::json!([]));
        wrong_count["totalItems"] = serde_json::json!(1);
        let mut wrong_size = browser_page(serde_json::json!([]));
        wrong_size["pageSize"] = serde_json::json!(10);
        let mut unbounded = browser_page(serde_json::json!([]));
        unbounded["totalItems"] = serde_json::json!(10001);
        for page in [
            browser_page(serde_json::json!([{"id":"other", "status":"active",
                "metadata":{"sb_session_id":"local-extra"}}])),
            browser_page(serde_json::json!([{"id":"unlabeled", "status":"active"}])),
            browser_page(serde_json::json!([item.clone(), item])),
            repeated_page,
            wrong_count,
            wrong_size,
            unbounded,
        ] {
            let (base, _captured, server) =
                super::super::test_http::spawn_json_server(vec![(200, page.to_string())]).await;
            let error = BrowserUseV3::with_base_url("test-key", base)
                .unwrap()
                .find_browser_by_session_id("local")
                .await
                .unwrap_err();
            assert!(matches!(error, ProviderError::InvalidResponse { .. }));
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn browser_reconciliation_rejects_changed_detail_identity() {
        let item = serde_json::json!({"id":"remote", "status":"active",
            "metadata":{"sb_session_id":"local"}});
        for detail in [
            serde_json::json!({"id":"other", "status":"active", "metadata":{"sb_session_id":"local"}}),
            serde_json::json!({"id":"remote", "status":"active", "metadata":{"sb_session_id":"different"}}),
        ] {
            let (base, _captured, server) = super::super::test_http::spawn_json_server(vec![
                (200, browser_page(serde_json::json!([item])).to_string()),
                (200, detail.to_string()),
            ])
            .await;
            let error = BrowserUseV3::with_base_url("test-key", base)
                .unwrap()
                .find_browser_by_session_id("local")
                .await
                .unwrap_err();
            assert!(matches!(error, ProviderError::InvalidResponse { .. }));
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn profile_reconciliation_rejects_a_repeated_provider_page() {
        let (base, mut captured, server) = super::super::test_http::spawn_json_server(vec![
            (200, profile_page(1, 101, 100)),
            (200, profile_page(1, 101, 1)),
        ])
        .await;
        let provider = BrowserUseV3::with_base_url("test-key", &base).unwrap();
        let error = provider.find_profile_by_user_id("profile-local").await.unwrap_err();
        assert!(matches!(error, ProviderError::InvalidResponse { provider: PROVIDER, .. }));
        assert!(error.to_string().contains("page number"));

        let first = captured.recv().await.unwrap();
        let second = captured.recv().await.unwrap();
        server.await.unwrap();
        assert!(first.target.contains("pageNumber=1"));
        assert!(second.target.contains("pageNumber=2"));
    }

    #[tokio::test]
    async fn profile_reconciliation_rejects_inconsistent_totals() {
        let (base, _captured, server) = super::super::test_http::spawn_json_server(vec![
            (200, profile_page(1, 101, 100)),
            (200, profile_page(2, 100, 0)),
        ])
        .await;
        let provider = BrowserUseV3::with_base_url("test-key", &base).unwrap();
        let error = provider.find_profile_by_user_id("profile-local").await.unwrap_err();
        server.await.unwrap();
        assert!(matches!(error, ProviderError::InvalidResponse { provider: PROVIDER, .. }));
        assert!(error.to_string().contains("totalItems"));
    }

    #[tokio::test]
    async fn profile_reconciliation_rejects_unbounded_totals_before_following_pages() {
        let unbounded_total = PROFILE_RECONCILIATION_PAGE_SIZE * MAX_PROFILE_RECONCILIATION_PAGES + 1;
        let (base, _captured, server) =
            super::super::test_http::spawn_json_server(vec![(200, profile_page(1, unbounded_total, 0))]).await;
        let provider = BrowserUseV3::with_base_url("test-key", &base).unwrap();
        let error = provider.find_profile_by_user_id("profile-local").await.unwrap_err();
        server.await.unwrap();
        assert!(matches!(error, ProviderError::InvalidResponse { provider: PROVIDER, .. }));
        assert!(error.to_string().contains("reconciliation limit"));
    }
}
