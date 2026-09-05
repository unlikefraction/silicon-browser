use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{CONTENT_TYPE, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use silicon_browser_shared::{ApiError, Validate};
pub use silicon_browser_shared::{
    FetchFormat, FetchItem, FetchRequest, FetchResponse, FetchStatus, SearchRequest, SearchResponse, SearchResult,
    SearchType,
};
use url::Url;

use crate::url_policy::is_https_or_loopback_http;

use super::error::{ProviderError, ProviderResult, json_response, json_response_with_limit, transport};

const PROVIDER: &str = "tinyfish";
const API_KEY_HEADER: &str = "X-API-Key";
const MAX_FETCH_BATCH: usize = 10;
const MAX_FETCH_URLS: usize = 1_000;
/// One upstream batch is bounded independently from the final aggregate. This keeps the decoded
/// body and its parsed representation small even when TinyFish omits `Content-Length`.
const MAX_TINYFISH_FETCH_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
/// Serialized `FetchItem`s are held below 12 MiB. Response/envelope framing is only a few KiB at
/// the 1,000-URL maximum, leaving ample room beneath the client's 64 MiB hard response limit and
/// headroom for the parsed representation of the current 2 MiB upstream batch.
pub(super) const MAX_FETCH_ITEMS_JSON_BYTES: usize = 12 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct FetchedPage {
    pub url: String,
    pub text: Value,
    #[serde(default)]
    pub links: Vec<String>,
    #[serde(default)]
    pub image_links: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
struct FetchFailure {
    pub url: String,
    pub error: String,
}

#[async_trait]
pub trait SearchProvider: Send + Sync {
    async fn search(&self, request: SearchRequest) -> ProviderResult<SearchResponse>;
    async fn fetch(&self, request: FetchRequest) -> ProviderResult<FetchResponse>;
}

/// Adapter for TinyFish's public Search and Fetch endpoints.
#[derive(Clone)]
pub struct TinyFish {
    http: reqwest::Client,
    search_endpoint: Url,
    fetch_endpoint: Url,
    api_key: HeaderValue,
}

impl std::fmt::Debug for TinyFish {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TinyFish")
            .field("search_endpoint", &self.search_endpoint)
            .field("fetch_endpoint", &self.fetch_endpoint)
            .field("api_key", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl TinyFish {
    pub fn new(api_key: impl AsRef<str>) -> ProviderResult<Self> {
        Self::with_endpoints(api_key, "https://api.search.tinyfish.ai", "https://api.fetch.tinyfish.ai")
    }

    /// Alternate endpoints for compatible deployments and local contract tests.
    pub fn with_endpoints(
        api_key: impl AsRef<str>,
        search_endpoint: impl AsRef<str>,
        fetch_endpoint: impl AsRef<str>,
    ) -> ProviderResult<Self> {
        let mut api_key = HeaderValue::from_str(api_key.as_ref())
            .map_err(|_| ProviderError::InvalidInput("TinyFish API key is not a valid header value".into()))?;
        if api_key.is_empty() {
            return Err(ProviderError::InvalidInput("TinyFish API key is empty".into()));
        }
        api_key.set_sensitive(true);
        let search_endpoint = http_endpoint(search_endpoint.as_ref(), "search")?;
        let fetch_endpoint = http_endpoint(fetch_endpoint.as_ref(), "fetch")?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            // Fetch has a 120 second CDN ceiling. Leave enough room to read its error body.
            .timeout(Duration::from_secs(150))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| transport(PROVIDER, error))?;
        Ok(Self { http, search_endpoint, fetch_endpoint, api_key })
    }
}

#[derive(Serialize)]
struct TinyFishSearchQuery<'a> {
    query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<&'a str>,
    page: u8,
}

#[derive(Deserialize)]
struct TinyFishSearchResponse {
    #[serde(default)]
    results: Vec<TinyFishSearchResult>,
    #[serde(default)]
    page: Option<u8>,
}

#[derive(Deserialize)]
struct TinyFishSearchResult {
    position: u32,
    title: String,
    url: String,
    #[serde(default)]
    snippet: Option<String>,
    #[serde(default)]
    date: Option<String>,
}

#[derive(Serialize)]
struct TinyFishFetchBody<'a> {
    urls: &'a [String],
    format: FetchFormat,
    links: bool,
    image_links: bool,
}

#[derive(Deserialize)]
struct TinyFishFetchResponse {
    #[serde(default)]
    results: Vec<FetchedPage>,
    #[serde(default)]
    errors: Vec<FetchFailure>,
}

#[async_trait]
impl SearchProvider for TinyFish {
    async fn search(&self, request: SearchRequest) -> ProviderResult<SearchResponse> {
        let query = tinyfish_search_query(&request)?;
        let url = tinyfish_search_url(self.search_endpoint.clone(), &query);
        let response = self
            .http
            .get(url)
            .header(API_KEY_HEADER, self.api_key.clone())
            .send()
            .await
            .map_err(|error| transport(PROVIDER, error))?;
        let response: TinyFishSearchResponse = json_response(PROVIDER, response).await?;
        Ok(SearchResponse {
            results: response
                .results
                .into_iter()
                .map(|result| SearchResult {
                    rank: result.position,
                    title: result.title,
                    url: result.url,
                    snippet: result.snippet.filter(|snippet| !snippet.is_empty()),
                    published_at: result.date.and_then(|date| {
                        chrono::DateTime::parse_from_rfc3339(&date).ok().map(|date| date.with_timezone(&chrono::Utc))
                    }),
                })
                .collect(),
            page: response.page.unwrap_or(request.page),
            queued_ms: 0,
        })
    }

    async fn fetch(&self, request: FetchRequest) -> ProviderResult<FetchResponse> {
        validate_fetch(&request)?;
        let mut items = Vec::with_capacity(request.urls.len());
        let mut budget = FetchResultBudget::default();
        for urls in request.urls.chunks(MAX_FETCH_BATCH) {
            let body = TinyFishFetchBody {
                urls,
                format: request.format,
                links: request.links,
                image_links: request.image_links,
            };
            let response = self
                .http
                .post(self.fetch_endpoint.clone())
                .header(API_KEY_HEADER, self.api_key.clone())
                .header(CONTENT_TYPE, "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|error| transport(PROVIDER, error))?;
            let response: TinyFishFetchResponse =
                json_response_with_limit(PROVIDER, response, MAX_TINYFISH_FETCH_RESPONSE_BYTES).await?;
            budget.extend(&mut items, order_fetch_chunk(urls, response)?)?;
        }
        Ok(FetchResponse { items, queued_ms: 0 })
    }
}

/// Running serialized-size accounting shared by the direct adapter and the fair pool. Counting is
/// performed through serde's writer interface, so checking a string full of escaped characters
/// does not allocate a second, potentially much larger JSON copy. Crossing the budget aborts the
/// complete request; content is never truncated into a deceptively successful partial document.
#[derive(Default)]
pub(super) struct FetchResultBudget {
    serialized_item_bytes: usize,
    item_count: usize,
}

impl FetchResultBudget {
    pub(super) fn extend(&mut self, destination: &mut Vec<FetchItem>, incoming: Vec<FetchItem>) -> ProviderResult<()> {
        for item in incoming {
            let separator_bytes = usize::from(self.item_count > 0);
            let remaining = MAX_FETCH_ITEMS_JSON_BYTES
                .checked_sub(self.serialized_item_bytes)
                .and_then(|remaining| remaining.checked_sub(separator_bytes))
                .ok_or_else(fetch_result_too_large)?;
            let mut counter = LimitedJsonCounter::new(remaining);
            let encoded = serde_json::to_writer(&mut counter, &item);
            if counter.exceeded {
                return Err(fetch_result_too_large());
            }
            encoded.map_err(|_| ProviderError::InvalidResponse {
                provider: PROVIDER,
                message: "fetch result could not be encoded safely".into(),
            })?;
            self.serialized_item_bytes = self
                .serialized_item_bytes
                .checked_add(separator_bytes)
                .and_then(|size| size.checked_add(counter.written))
                .ok_or_else(fetch_result_too_large)?;
            self.item_count += 1;
            destination.push(item);
        }
        Ok(())
    }

    #[cfg(test)]
    fn serialized_item_bytes(&self) -> usize {
        self.serialized_item_bytes
    }
}

struct LimitedJsonCounter {
    written: usize,
    limit: usize,
    exceeded: bool,
}

impl LimitedJsonCounter {
    const fn new(limit: usize) -> Self {
        Self { written: 0, limit, exceeded: false }
    }
}

impl io::Write for LimitedJsonCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.written) {
            self.exceeded = true;
            return Err(io::Error::new(io::ErrorKind::FileTooLarge, "JSON output budget exceeded"));
        }
        self.written += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn fetch_result_too_large() -> ProviderError {
    ProviderError::InvalidResponse {
        provider: PROVIDER,
        message: format!("fetch result exceeded the {MAX_FETCH_ITEMS_JSON_BYTES}-byte aggregate output limit"),
    }
}

fn tinyfish_search_url(mut url: Url, query: &TinyFishSearchQuery<'_>) -> Url {
    let mut pairs = url.query_pairs_mut();
    pairs.append_pair("query", &query.query);
    if let Some(location) = query.location {
        pairs.append_pair("location", location);
    }
    if let Some(language) = query.language {
        pairs.append_pair("language", language);
    }
    pairs.append_pair("page", &query.page.to_string());
    drop(pairs);
    url
}

fn tinyfish_search_query(request: &SearchRequest) -> ProviderResult<TinyFishSearchQuery<'_>> {
    validate_search(request)?;
    // `purpose` is caller intent for Silicon Browser's audit/accounting layer, not a documented
    // TinyFish search parameter. Appending it to `query` would change retrieval semantics and can
    // disclose internal task context, so it intentionally remains local-only.
    Ok(TinyFishSearchQuery {
        query: query_with_domain_operators(&request.query, &request.include_domains, &request.exclude_domains)?,
        location: request.location.as_deref(),
        language: request.language.as_deref(),
        page: request.page,
    })
}

fn validate_search(request: &SearchRequest) -> ProviderResult<()> {
    request.validate().map_err(|error| ProviderError::InvalidInput(error.to_string()))?;
    if request.search_type != SearchType::Web {
        return Err(ProviderError::Unsupported { provider: PROVIDER, feature: "news and research search types" });
    }
    if request.recency_minutes.is_some()
        || request.after.is_some()
        || request.before.is_some()
        || request.pub_year_min.is_some()
        || request.pub_year_max.is_some()
    {
        return Err(ProviderError::Unsupported {
            provider: PROVIDER,
            feature: "search recency, date, and publication-year filters",
        });
    }
    query_with_domain_operators(&request.query, &request.include_domains, &request.exclude_domains)?;
    Ok(())
}

fn validate_fetch(request: &FetchRequest) -> ProviderResult<()> {
    request.validate().map_err(|error| ProviderError::InvalidInput(error.to_string()))?;
    if request.urls.is_empty() || request.urls.len() > MAX_FETCH_URLS {
        return Err(ProviderError::InvalidInput(format!("fetch requires 1..={MAX_FETCH_URLS} URLs")));
    }
    if request.ttl_seconds.is_some()
        || request.timeout_ms.is_some()
        || !request.include_selectors.is_empty()
        || !request.exclude_selectors.is_empty()
    {
        return Err(ProviderError::Unsupported {
            provider: PROVIDER,
            feature: "fetch TTL, per-URL timeout, and selector filters",
        });
    }
    for raw in &request.urls {
        let url =
            Url::parse(raw).map_err(|error| ProviderError::InvalidInput(format!("invalid fetch URL: {error}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(ProviderError::InvalidInput("fetch URLs must use http or https".into()));
        }
        if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
            return Err(ProviderError::InvalidInput(
                "fetch URLs must have a host and may not contain credentials".into(),
            ));
        }
    }
    Ok(())
}

fn order_fetch_chunk(urls: &[String], response: TinyFishFetchResponse) -> ProviderResult<Vec<FetchItem>> {
    let mut positions: BTreeMap<&str, VecDeque<usize>> = BTreeMap::new();
    for (index, url) in urls.iter().enumerate() {
        positions.entry(url.as_str()).or_default().push_back(index);
    }
    let mut ordered = vec![None; urls.len()];

    let mut place = |url: &str, outcome: FetchItem| -> ProviderResult<()> {
        let index =
            positions.get_mut(url).and_then(VecDeque::pop_front).ok_or_else(|| ProviderError::InvalidResponse {
                provider: PROVIDER,
                message: "fetch response named an unrequested or duplicate URL".into(),
            })?;
        ordered[index] = Some(outcome);
        Ok(())
    };
    for page in response.results {
        let url = page.url.clone();
        let content = match page.text {
            Value::String(content) => content,
            value => serde_json::to_string(&value).map_err(|error| ProviderError::InvalidResponse {
                provider: PROVIDER,
                message: format!("could not encode JSON fetch content: {error}"),
            })?,
        };
        place(
            &url,
            FetchItem {
                url: url.clone(),
                status: FetchStatus::Ok,
                content: Some(content),
                links: page.links,
                image_links: page.image_links,
                error: None,
                cached: false,
            },
        )?;
    }
    for failure in response.errors {
        let url = failure.url.clone();
        place(
            &url,
            FetchItem {
                url: url.clone(),
                status: FetchStatus::Error,
                content: None,
                links: vec![],
                image_links: vec![],
                error: Some(fetch_error(&failure.error)),
                cached: false,
            },
        )?;
    }

    Ok(ordered
        .into_iter()
        .zip(urls)
        .map(|(outcome, url)| {
            outcome.unwrap_or_else(|| FetchItem {
                url: url.clone(),
                status: FetchStatus::Error,
                content: None,
                links: vec![],
                image_links: vec![],
                error: Some(public_fetch_error("provider_missing_result", "TinyFish returned no result for this URL")),
                cached: false,
            })
        })
        .collect())
}

fn fetch_error(_untrusted_provider_error: &str) -> ApiError {
    // Per-URL provider error strings are untrusted and may echo submitted URLs or
    // credentials. Preserve the fact of failure, but never carry the raw string out.
    public_fetch_error("provider_fetch_failed", "TinyFish could not fetch this URL")
}

fn public_fetch_error(code: &str, message: &str) -> ApiError {
    ApiError {
        code: code.to_owned(),
        message: message.to_owned(),
        fields: vec![],
        details: BTreeMap::new(),
        request_id: None,
        retry_after_ms: None,
    }
}

fn normal_domain(raw: &str) -> ProviderResult<String> {
    let raw = raw.trim().trim_start_matches("*.");
    if raw.is_empty()
        || raw.len() > 253
        || raw.contains(['/', ':', '?', '#', '@'])
        || !raw.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err(ProviderError::InvalidInput(format!("invalid search domain: {raw}")));
    }
    let labels = raw.split('.');
    if labels.clone().any(|label| label.is_empty() || label.starts_with('-') || label.ends_with('-')) {
        return Err(ProviderError::InvalidInput(format!("invalid search domain: {raw}")));
    }
    Ok(raw.to_ascii_lowercase())
}

fn query_with_domain_operators(query: &str, includes: &[String], excludes: &[String]) -> ProviderResult<String> {
    let includes = includes.iter().map(|domain| normal_domain(domain)).collect::<ProviderResult<Vec<_>>>()?;
    let excludes = excludes.iter().map(|domain| normal_domain(domain)).collect::<ProviderResult<Vec<_>>>()?;
    let mut result = query.to_owned();
    if !includes.is_empty() {
        result.push_str(" (");
        result.push_str(&includes.iter().map(|domain| format!("site:{domain}")).collect::<Vec<_>>().join(" OR "));
        result.push(')');
    }
    for domain in excludes {
        result.push_str(" -site:");
        result.push_str(&domain);
    }
    Ok(result)
}

fn http_endpoint(raw: &str, kind: &str) -> ProviderResult<Url> {
    let url = Url::parse(raw)
        .map_err(|error| ProviderError::InvalidInput(format!("invalid TinyFish {kind} endpoint: {error}")))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ProviderError::InvalidInput(format!(
            "TinyFish {kind} endpoint must be a credential-free http(s) URL without query or fragment"
        )));
    }
    if !is_https_or_loopback_http(&url) {
        return Err(ProviderError::InvalidInput(format!(
            "TinyFish {kind} endpoint must use HTTPS unless its host is loopback"
        )));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternate_endpoints_reject_credentials_query_and_fragment() {
        let safe = "https://example.com/search";
        for endpoint in [
            "https://user:password@example.com/search",
            "https://example.com/search?api_key=secret",
            "https://example.com/search#secret",
        ] {
            for result in [
                TinyFish::with_endpoints("test-key", endpoint, safe),
                TinyFish::with_endpoints("test-key", safe, endpoint),
            ] {
                let error = result.unwrap_err();
                assert!(matches!(error, ProviderError::InvalidInput(_)));
                assert!(!error.to_string().contains("password"));
                assert!(!error.to_string().contains("api_key"));
                assert!(!error.to_string().contains("secret"));
            }
        }
    }

    #[test]
    fn alternate_endpoints_require_tls_except_for_loopback_contract_tests() {
        let safe = "https://example.com/search";
        for endpoint in ["http://api.example/search", "http://10.0.0.1/search"] {
            assert!(TinyFish::with_endpoints("test-key", endpoint, safe).is_err(), "{endpoint}");
            assert!(TinyFish::with_endpoints("test-key", safe, endpoint).is_err(), "{endpoint}");
        }
        for endpoint in ["http://localhost:8080/search", "http://127.0.0.1:8080/search", "http://[::1]:8080/search"] {
            assert!(TinyFish::with_endpoints("test-key", endpoint, endpoint).is_ok(), "{endpoint}");
        }
    }

    #[test]
    fn test_group_search_sends_only_documented_fields_and_uses_domain_operators() {
        let request = SearchRequest {
            query: "rust browser automation".into(),
            purpose: "Find primary docs".into(),
            search_type: SearchType::Web,
            include_domains: vec!["*.example.com".into(), "docs.rs".into()],
            exclude_domains: vec!["spam.example".into()],
            location: Some("US".into()),
            language: Some("en".into()),
            recency_minutes: None,
            after: None,
            before: None,
            pub_year_min: None,
            pub_year_max: None,
            page: 2,
        };
        let query = serde_json::to_value(tinyfish_search_query(&request).unwrap()).unwrap();
        assert_eq!(query["query"], "rust browser automation (site:example.com OR site:docs.rs) -site:spam.example");
        assert_eq!(query["location"], "US");
        assert_eq!(query["language"], "en");
        assert_eq!(query["page"], 2);
        assert_eq!(query.as_object().unwrap().len(), 4);
    }

    #[test]
    fn test_group_search_rejects_unsupported_provider_semantics() {
        let request = SearchRequest {
            query: "attention".into(),
            purpose: "find the paper".into(),
            search_type: SearchType::Research,
            include_domains: vec![],
            exclude_domains: vec![],
            location: None,
            language: None,
            recency_minutes: None,
            after: None,
            before: None,
            pub_year_min: Some(2017),
            pub_year_max: Some(2024),
            page: 0,
        };
        assert!(matches!(validate_search(&request), Err(ProviderError::Unsupported { .. })));

        let recency = SearchRequest {
            search_type: SearchType::Web,
            pub_year_min: None,
            pub_year_max: None,
            recency_minutes: Some(5),
            ..request
        };
        assert!(matches!(validate_search(&recency), Err(ProviderError::Unsupported { .. })));
    }

    #[test]
    fn test_group_fetch_order_restores_provider_results_and_represents_missing_items() {
        let urls = vec!["https://a.test".into(), "https://b.test".into(), "https://c.test".into()];
        let page = |url: &str| FetchedPage {
            url: url.into(),
            text: Value::String(url.into()),
            links: vec![],
            image_links: vec![],
        };
        let response =
            TinyFishFetchResponse { results: vec![page("https://c.test"), page("https://a.test")], errors: vec![] };
        let ordered = order_fetch_chunk(&urls, response).unwrap();
        assert_eq!(ordered[0].status, FetchStatus::Ok);
        assert_eq!(ordered[0].url, urls[0]);
        assert_eq!(ordered[1].status, FetchStatus::Error);
        assert_eq!(ordered[1].error.as_ref().unwrap().code, "provider_missing_result");
        assert_eq!(ordered[2].status, FetchStatus::Ok);
        assert_eq!(ordered[2].url, urls[2]);
    }

    #[test]
    fn test_group_fetch_validation_rejects_credentials_and_unsupported_options() {
        let credentials = FetchRequest {
            urls: vec!["https://user:pass@example.com".into()],
            purpose: "read".into(),
            format: FetchFormat::Markdown,
            links: false,
            image_links: false,
            ttl_seconds: None,
            timeout_ms: None,
            include_selectors: vec![],
            exclude_selectors: vec![],
        };
        assert!(validate_fetch(&credentials).is_err());

        let supported = FetchRequest {
            urls: vec!["https://example.com".into()],
            purpose: "read".into(),
            format: FetchFormat::Markdown,
            links: false,
            image_links: false,
            ttl_seconds: None,
            timeout_ms: None,
            include_selectors: vec![],
            exclude_selectors: vec![],
        };
        assert!(validate_fetch(&supported).is_ok());

        let body = TinyFishFetchBody {
            urls: &supported.urls,
            format: supported.format,
            links: supported.links,
            image_links: supported.image_links,
        };
        let body = serde_json::to_value(body).unwrap();
        assert_eq!(body.as_object().unwrap().len(), 4);
        assert!(body.get("purpose").is_none());
        assert!(body.get("ttl").is_none());
        assert!(body.get("per_url_timeout_ms").is_none());

        let selectors = FetchRequest { include_selectors: vec!["main".into()], ..supported };
        assert!(matches!(validate_fetch(&selectors), Err(ProviderError::Unsupported { .. })));
    }

    #[test]
    fn fetch_result_budget_accepts_exact_boundary_then_fails_without_echoing_content() {
        let template = FetchItem {
            url: "https://example.com".into(),
            status: FetchStatus::Ok,
            content: Some(String::new()),
            links: vec![],
            image_links: vec![],
            error: None,
            cached: false,
        };
        let framing = serde_json::to_vec(&template).unwrap().len();
        let at_boundary =
            FetchItem { content: Some("x".repeat(MAX_FETCH_ITEMS_JSON_BYTES - framing)), ..template.clone() };
        let mut budget = FetchResultBudget::default();
        let mut items = Vec::new();
        budget.extend(&mut items, vec![at_boundary]).unwrap();
        assert_eq!(budget.serialized_item_bytes(), MAX_FETCH_ITEMS_JSON_BYTES);

        let secret = "must-not-appear-in-error";
        let error =
            budget.extend(&mut items, vec![FetchItem { content: Some(secret.into()), ..template }]).unwrap_err();
        assert!(matches!(error, ProviderError::InvalidResponse { provider: PROVIDER, .. }));
        assert!(!error.to_string().contains(secret));
    }

    #[tokio::test]
    async fn tinyfish_fetch_rejects_an_oversized_batch_before_reading_or_decoding_it() {
        let (base, server) = super::super::test_http::spawn_header_only_server(
            200,
            vec![("Content-Length".into(), (MAX_TINYFISH_FETCH_RESPONSE_BYTES + 1).to_string())],
        )
        .await;
        let provider = TinyFish::with_endpoints("test-key", &base, &base).unwrap();
        let error = provider
            .fetch(FetchRequest {
                urls: vec!["https://example.com".into()],
                purpose: "read safely".into(),
                format: FetchFormat::Markdown,
                links: false,
                image_links: false,
                ttl_seconds: None,
                timeout_ms: None,
                include_selectors: vec![],
                exclude_selectors: vec![],
            })
            .await
            .unwrap_err();
        server.await.unwrap();
        assert!(matches!(error, ProviderError::InvalidResponse { provider: PROVIDER, .. }));
        assert!(error.to_string().contains(&MAX_TINYFISH_FETCH_RESPONSE_BYTES.to_string()));
    }

    #[tokio::test]
    async fn test_group_tinyfish_search_sends_direct_documented_query_parameters() {
        let (base, mut captured, server) = super::super::test_http::spawn_json_server(vec![(
            200,
            serde_json::json!({"results": [], "page": 2}).to_string(),
        )])
        .await;
        let provider = TinyFish::with_endpoints("test-key", &base, &base).unwrap();
        let response = provider
            .search(SearchRequest {
                query: "latest browser research".into(),
                purpose: "evaluate the primary literature".into(),
                search_type: SearchType::Web,
                include_domains: vec!["arxiv.org".into(), "acm.org".into()],
                exclude_domains: vec!["example.org".into()],
                location: Some("US".into()),
                language: Some("en".into()),
                recency_minutes: None,
                after: None,
                before: None,
                pub_year_min: None,
                pub_year_max: None,
                page: 2,
            })
            .await
            .unwrap();
        assert_eq!(response.page, 2);

        let request = captured.recv().await.unwrap();
        server.await.unwrap();
        assert_eq!(request.method, "GET");
        assert!(request.headers.to_ascii_lowercase().contains("x-api-key: test-key"));
        let parsed = Url::parse(&format!("http://test.invalid{}", request.target)).unwrap();
        let parameters = parsed.query_pairs().into_owned().collect::<BTreeMap<_, _>>();
        assert_eq!(parameters["query"], "latest browser research (site:arxiv.org OR site:acm.org) -site:example.org");
        assert_eq!(parameters["location"], "US");
        assert_eq!(parameters["language"], "en");
        assert_eq!(parameters["page"], "2");
        assert_eq!(parameters.len(), 4);
    }

    #[tokio::test]
    async fn test_group_tinyfish_fetch_batches_ten_and_restores_stable_order() {
        let urls = (0..11).map(|index| format!("https://example.com/{index}")).collect::<Vec<_>>();
        let response_for = |batch: &[String]| {
            let mut results =
                batch.iter().rev().map(|url| serde_json::json!({"url": url, "text": url})).collect::<Vec<_>>();
            // Keep this mutable to make the deliberately reversed provider order explicit.
            results.shrink_to_fit();
            serde_json::json!({"results": results, "errors": []}).to_string()
        };
        let (base, mut captured, server) = super::super::test_http::spawn_json_server(vec![
            (200, response_for(&urls[..10])),
            (200, response_for(&urls[10..])),
        ])
        .await;
        let provider = TinyFish::with_endpoints("test-key", &base, &base).unwrap();
        let response = provider
            .fetch(FetchRequest {
                urls: urls.clone(),
                purpose: "extract article bodies".into(),
                format: FetchFormat::Markdown,
                links: true,
                image_links: false,
                ttl_seconds: None,
                timeout_ms: None,
                include_selectors: vec![],
                exclude_selectors: vec![],
            })
            .await
            .unwrap();
        assert_eq!(response.items.iter().map(|item| &item.url).collect::<Vec<_>>(), urls.iter().collect::<Vec<_>>());

        let first: Value = serde_json::from_slice(&captured.recv().await.unwrap().body).unwrap();
        let second: Value = serde_json::from_slice(&captured.recv().await.unwrap().body).unwrap();
        server.await.unwrap();
        assert_eq!(first["urls"].as_array().unwrap().len(), 10);
        assert_eq!(second["urls"].as_array().unwrap().len(), 1);
        assert_eq!(first.as_object().unwrap().len(), 4);
        assert!(first.get("purpose").is_none());
        assert!(first.get("ttl").is_none());
        assert!(first.get("per_url_timeout_ms").is_none());
        assert!(first.get("include_selectors").is_none());
        assert!(first.get("exclude_selectors").is_none());
    }
}
