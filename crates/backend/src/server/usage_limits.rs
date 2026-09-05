use silicon_browser_shared::UsageLimits;
use tokio::time::Instant;

use super::*;

const SUCCESS_TTL: Duration = Duration::from_secs(60);
const FAILURE_TTL: Duration = Duration::from_secs(5);
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

struct CacheEntry {
    expires_at: Instant,
    result: Result<UsageLimits, ()>,
}

/// One bounded cache per configured browser account. Holding the lock through
/// refresh coalesces concurrent callers, including when the account API fails.
#[derive(Clone, Default)]
pub(super) struct UsageLimitsCache(Arc<Mutex<Option<CacheEntry>>>);

impl UsageLimitsCache {
    async fn get(&self, browser: &dyn BrowserProvider) -> Result<UsageLimits, ()> {
        let mut cached = self.0.lock().await;
        if let Some(entry) = cached.as_ref()
            && entry.expires_at > Instant::now()
        {
            return entry.result.clone();
        }
        let result = match tokio::time::timeout(CHECK_TIMEOUT, browser.account_limits()).await {
            Ok(Ok(limits)) => Ok(UsageLimits {
                concurrent_browser_limit: limits.concurrent_browser_limit,
                rate_limit: limits.rate_limit,
                checked_at: Utc::now(),
            }),
            Ok(Err(error)) => {
                tracing::warn!(error_kind = provider_error_kind(&error), "account limits check failed");
                Err(())
            }
            Err(_) => {
                tracing::warn!("account limits check timed out");
                Err(())
            }
        };
        let ttl = if result.is_ok() { SUCCESS_TTL } else { FAILURE_TTL };
        *cached = Some(CacheEntry { expires_at: Instant::now() + ttl, result: result.clone() });
        result
    }
}

pub(super) async fn get_usage_limits(State(state): State<AppState>, _scope: Scope) -> Response {
    // Scope authenticates current organization membership before accessing the
    // shared capacity. It does not authorize any account activity or finances.
    let mut response = match state.usage_limits.get(state.browser.as_ref()).await {
        Ok(limits) => success(limits).into_response(),
        Err(()) => {
            let mut error = ApiFailure::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "usage_limits_unavailable",
                "browser capacity is temporarily unavailable",
            );
            error.retry_after_ms = Some(FAILURE_TTL.as_millis() as u64);
            error.into_response()
        }
    };
    response.headers_mut().insert("cache-control", HeaderValue::from_static("no-store"));
    response
}
