use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use silicon_browser_shared::Validate;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use tokio::time::Instant;

use super::error::{ProviderError, ProviderResult};
use super::search::{
    FetchRequest, FetchResponse, FetchResultBudget, SearchProvider, SearchRequest, SearchResponse, TinyFish,
};

const PROVIDER: &str = "tinyfish";
pub const TINYFISH_SEARCH_REQUESTS_PER_MINUTE: usize = 30;
pub const TINYFISH_FETCH_URLS_PER_MINUTE: usize = 150;
pub const FAIR_SEARCH_QUEUE_CAPACITY: usize = 1_024;
const QUOTA_WINDOW: Duration = Duration::from_secs(60);
const DEFAULT_RATE_LIMIT_BACKOFF: Duration = QUOTA_WINDOW;
const MIN_RATE_LIMIT_BACKOFF: Duration = Duration::from_millis(100);
const MAX_RATE_LIMIT_BACKOFF: Duration = Duration::from_secs(60);
const MAX_RATE_LIMIT_RETRIES: usize = 7;
const MAX_RATE_LIMIT_ELAPSED: Duration = Duration::from_secs(120);
const MAX_FETCH_URLS: usize = 1_000;

/// A pool of provider credentials with sliding-window quotas and per-actor round-robin queues.
///
/// Each provider should represent one independently quota-limited TinyFish API key. Calls for
/// the same actor retain their order; actors at the head of the queue receive one permit each
/// before any actor receives a second. Fetches are scheduled in upstream-sized chunks, so a
/// large multi-URL request cannot reserve a full minute of quota ahead of other actors.
#[derive(Clone)]
pub struct FairSearchPool {
    providers: Arc<[Arc<dyn SearchProvider>]>,
    search_quota: FairQuota,
    fetch_quota: FairQuota,
}

impl std::fmt::Debug for FairSearchPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FairSearchPool").field("provider_count", &self.providers.len()).finish_non_exhaustive()
    }
}

impl FairSearchPool {
    /// Construct a pool inside a Tokio runtime. Each entry must own a distinct provider key.
    pub fn new(providers: Vec<Arc<dyn SearchProvider>>) -> ProviderResult<Self> {
        if providers.is_empty() {
            return Err(ProviderError::InvalidInput("search provider pool requires at least one key".into()));
        }
        let handle = tokio::runtime::Handle::try_current().map_err(|_| {
            ProviderError::InvalidInput("search provider pool must be constructed inside a Tokio runtime".into())
        })?;
        let provider_count = providers.len();
        Ok(Self {
            providers: providers.into(),
            search_quota: FairQuota::spawn(provider_count, TINYFISH_SEARCH_REQUESTS_PER_MINUTE, QUOTA_WINDOW, &handle),
            fetch_quota: FairQuota::spawn(provider_count, TINYFISH_FETCH_URLS_PER_MINUTE, QUOTA_WINDOW, &handle),
        })
    }

    /// Convenience constructor which keeps one TinyFish adapter (and quota bucket) per key.
    pub fn from_tinyfish_api_keys<I, S>(api_keys: I) -> ProviderResult<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut distinct_keys = BTreeSet::new();
        let mut providers = Vec::new();
        for key in api_keys {
            if !distinct_keys.insert(key.as_ref().to_owned()) {
                return Err(ProviderError::InvalidInput(
                    "TinyFish API keys must be distinct so quota buckets cannot double-count one credential".into(),
                ));
            }
            providers.push(Arc::new(TinyFish::new(key)?) as Arc<dyn SearchProvider>);
        }
        Self::new(providers)
    }

    pub async fn search_for(&self, actor: &str, request: SearchRequest) -> ProviderResult<SearchResponse> {
        validate_actor(actor)?;
        request.validate().map_err(|error| ProviderError::InvalidInput(error.to_string()))?;
        let mut queued_ms = 0_u64;
        let mut retry_budget = RateLimitBudget::default();
        loop {
            let permit = retry_budget.acquire(&self.search_quota, actor, 1).await?;
            queued_ms = queued_ms.saturating_add(permit.queued_ms());
            match retry_budget.run(self.providers[permit.provider].search(request.clone())).await {
                Ok(mut response) => {
                    response.queued_ms = response.queued_ms.saturating_add(queued_ms);
                    return Ok(response);
                }
                Err(error) if rate_limited(&error).is_some() => {
                    let retry_after = rate_limited(&error).expect("rate-limited error has a backoff");
                    self.search_quota.reject(permit, retry_after).await?;
                    retry_budget.retry(error, retry_after)?;
                }
                Err(error) => {
                    self.search_quota.refund(permit, 1).await?;
                    return Err(error);
                }
            }
        }
    }

    pub async fn fetch_for(&self, actor: &str, request: FetchRequest) -> ProviderResult<FetchResponse> {
        validate_actor(actor)?;
        request.validate().map_err(|error| ProviderError::InvalidInput(error.to_string()))?;
        if request.urls.len() > MAX_FETCH_URLS {
            return Err(ProviderError::InvalidInput(format!("fetch requires at most {MAX_FETCH_URLS} URLs")));
        }

        let mut items = Vec::with_capacity(request.urls.len());
        let mut budget = FetchResultBudget::default();
        let mut queued_ms = 0_u64;
        for urls in request.urls.chunks(FetchRequest::UPSTREAM_BATCH_SIZE) {
            let mut retry_budget = RateLimitBudget::default();
            let batch = FetchRequest {
                urls: urls.to_vec(),
                purpose: request.purpose.clone(),
                format: request.format,
                links: request.links,
                image_links: request.image_links,
                ttl_seconds: request.ttl_seconds,
                timeout_ms: request.timeout_ms,
                include_selectors: request.include_selectors.clone(),
                exclude_selectors: request.exclude_selectors.clone(),
            };
            loop {
                let permit = retry_budget.acquire(&self.fetch_quota, actor, urls.len()).await?;
                queued_ms = queued_ms.saturating_add(permit.queued_ms());
                match retry_budget.run(self.providers[permit.provider].fetch(batch.clone())).await {
                    Ok(response) if response.items.len() == urls.len() => {
                        let errors = response
                            .items
                            .iter()
                            .filter(|item| item.status == super::search::FetchStatus::Error)
                            .count();
                        if errors > 0 {
                            self.fetch_quota.refund(permit, errors).await?;
                        }
                        queued_ms = queued_ms.saturating_add(response.queued_ms);
                        budget.extend(&mut items, response.items)?;
                        break;
                    }
                    Ok(_) => {
                        self.fetch_quota.refund(permit, urls.len()).await?;
                        return Err(ProviderError::InvalidResponse {
                            provider: PROVIDER,
                            message: "pooled fetch provider violated the one-result-per-URL contract".into(),
                        });
                    }
                    Err(error) if rate_limited(&error).is_some() => {
                        let retry_after = rate_limited(&error).expect("rate-limited error has a backoff");
                        self.fetch_quota.reject(permit, retry_after).await?;
                        retry_budget.retry(error, retry_after)?;
                    }
                    Err(error) => {
                        self.fetch_quota.refund(permit, urls.len()).await?;
                        return Err(error);
                    }
                }
            }
        }
        Ok(FetchResponse { items, queued_ms })
    }

    /// Bind an actor once and use the result anywhere a `SearchProvider` is expected.
    pub fn for_actor(self: &Arc<Self>, actor: impl Into<String>) -> ProviderResult<ActorSearchProvider> {
        let actor = actor.into();
        validate_actor(&actor)?;
        Ok(ActorSearchProvider { pool: Arc::clone(self), actor })
    }
}

fn rate_limited(error: &ProviderError) -> Option<Duration> {
    match error {
        ProviderError::Http { status: 429, retry_after, .. } => Some(
            retry_after.unwrap_or(DEFAULT_RATE_LIMIT_BACKOFF).max(MIN_RATE_LIMIT_BACKOFF).min(MAX_RATE_LIMIT_BACKOFF),
        ),
        _ => None,
    }
}

#[derive(Debug, Default)]
struct RateLimitBudget {
    deadline: Option<Instant>,
    retries: usize,
    last_error: Option<ProviderError>,
}

impl RateLimitBudget {
    async fn acquire(&mut self, quota: &FairQuota, actor: &str, units: usize) -> ProviderResult<QuotaPermit> {
        self.run(quota.acquire(actor, units)).await
    }

    async fn run<T>(&mut self, operation: impl std::future::Future<Output = ProviderResult<T>>) -> ProviderResult<T> {
        let Some(deadline) = self.deadline else {
            return operation.await;
        };
        match tokio::time::timeout_at(deadline, operation).await {
            Ok(result) => result,
            Err(_) => Err(self.deadline_error()),
        }
    }

    fn retry(&mut self, error: ProviderError, retry_after: Duration) -> ProviderResult<()> {
        let now = Instant::now();
        let deadline = *self.deadline.get_or_insert(now + MAX_RATE_LIMIT_ELAPSED);
        if self.retries >= MAX_RATE_LIMIT_RETRIES || now + retry_after >= deadline {
            return Err(error);
        }
        self.retries += 1;
        self.last_error = Some(error);
        Ok(())
    }

    fn deadline_error(&mut self) -> ProviderError {
        self.last_error.take().unwrap_or(ProviderError::Http {
            provider: PROVIDER,
            status: 429,
            message: "rate limit retry deadline elapsed".into(),
            retry_after: None,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ActorSearchProvider {
    pool: Arc<FairSearchPool>,
    actor: String,
}

#[async_trait]
impl SearchProvider for ActorSearchProvider {
    async fn search(&self, request: SearchRequest) -> ProviderResult<SearchResponse> {
        self.pool.search_for(&self.actor, request).await
    }

    async fn fetch(&self, request: FetchRequest) -> ProviderResult<FetchResponse> {
        self.pool.fetch_for(&self.actor, request).await
    }
}

fn validate_actor(actor: &str) -> ProviderResult<()> {
    if actor.trim().is_empty() || actor.len() > 255 || actor.chars().any(char::is_control) {
        return Err(ProviderError::InvalidInput("invalid actor id for search scheduling".into()));
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct FairQuota {
    sender: mpsc::Sender<QuotaMessage>,
    queue_slots: Arc<Semaphore>,
    max_units: usize,
}

impl FairQuota {
    fn spawn(provider_count: usize, max_units: usize, window: Duration, handle: &tokio::runtime::Handle) -> Self {
        let (sender, receiver) = mpsc::channel(FAIR_SEARCH_QUEUE_CAPACITY);
        handle.spawn(run_quota_scheduler(receiver, provider_count, max_units, window));
        Self { sender, queue_slots: Arc::new(Semaphore::new(FAIR_SEARCH_QUEUE_CAPACITY)), max_units }
    }

    async fn acquire(&self, actor: &str, units: usize) -> ProviderResult<QuotaPermit> {
        if units == 0 || units > self.max_units {
            return Err(ProviderError::InvalidInput(format!(
                "provider quota request must consume 1..={} units",
                self.max_units
            )));
        }
        // This permit follows the request through the channel and per-actor queues. Merely
        // draining the bounded channel can therefore never turn it into an unbounded heap queue.
        let queue_slot = Arc::clone(&self.queue_slots)
            .try_acquire_owned()
            .map_err(|_| ProviderError::Overloaded { provider: PROVIDER, capacity: FAIR_SEARCH_QUEUE_CAPACITY })?;
        let (reply, received) = oneshot::channel();
        self.sender
            .try_send(QuotaMessage::Acquire(QuotaRequest {
                actor: actor.to_owned(),
                units,
                queued_at: Instant::now(),
                reply,
                _queue_slot: queue_slot,
            }))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => {
                    ProviderError::Overloaded { provider: PROVIDER, capacity: FAIR_SEARCH_QUEUE_CAPACITY }
                }
                mpsc::error::TrySendError::Closed(_) => {
                    ProviderError::Transport { provider: PROVIDER, message: "quota scheduler is unavailable".into() }
                }
            })?;
        received.await.map_err(|_| ProviderError::Transport {
            provider: PROVIDER,
            message: "quota scheduler stopped before granting a permit".into(),
        })
    }

    async fn refund(&self, permit: QuotaPermit, units: usize) -> ProviderResult<()> {
        if units == 0 {
            return Ok(());
        }
        if units > permit.units {
            return Err(ProviderError::InvalidInput("quota refund exceeded its reservation".into()));
        }
        self.send_control(QuotaControl::Refund { provider: permit.provider, reservation: permit.reservation, units })
            .await
    }

    async fn reject(&self, permit: QuotaPermit, retry_after: Duration) -> ProviderResult<()> {
        self.send_control(QuotaControl::Reject {
            provider: permit.provider,
            reservation: permit.reservation,
            units: permit.units,
            retry_after: retry_after.max(MIN_RATE_LIMIT_BACKOFF).min(MAX_RATE_LIMIT_BACKOFF),
        })
        .await
    }

    async fn send_control(&self, control: QuotaControl) -> ProviderResult<()> {
        self.sender.send(QuotaMessage::Control(control)).await.map_err(|_| ProviderError::Transport {
            provider: PROVIDER,
            message: "quota scheduler is unavailable".into(),
        })
    }
}

#[derive(Debug)]
enum QuotaMessage {
    Acquire(QuotaRequest),
    Control(QuotaControl),
}

#[derive(Debug)]
enum QuotaControl {
    Refund { provider: usize, reservation: u64, units: usize },
    Reject { provider: usize, reservation: u64, units: usize, retry_after: Duration },
}

#[derive(Debug)]
struct QuotaRequest {
    actor: String,
    units: usize,
    queued_at: Instant,
    reply: oneshot::Sender<QuotaPermit>,
    _queue_slot: OwnedSemaphorePermit,
}

#[derive(Clone, Copy, Debug)]
struct QuotaPermit {
    provider: usize,
    waited: Duration,
    reservation: u64,
    units: usize,
}

impl QuotaPermit {
    fn queued_ms(self) -> u64 {
        self.waited.as_millis().min(u128::from(u64::MAX)) as u64
    }
}

#[derive(Debug)]
struct SlidingQuota {
    used_at: Vec<VecDeque<Charge>>,
    blocked_until: Vec<Option<Instant>>,
    max_units: usize,
    window: Duration,
    next_reservation: u64,
}

#[derive(Clone, Copy, Debug)]
struct Charge {
    reservation: u64,
    at: Instant,
}

impl SlidingQuota {
    fn new(provider_count: usize, max_units: usize, window: Duration) -> Self {
        Self {
            used_at: vec![VecDeque::new(); provider_count],
            blocked_until: vec![None; provider_count],
            max_units,
            window,
            next_reservation: 1,
        }
    }

    fn prune(&mut self, now: Instant) {
        for used in &mut self.used_at {
            while used.front().is_some_and(|charge| now.saturating_duration_since(charge.at) >= self.window) {
                used.pop_front();
            }
        }
        for blocked_until in &mut self.blocked_until {
            if blocked_until.is_some_and(|deadline| deadline <= now) {
                *blocked_until = None;
            }
        }
    }

    fn take(&mut self, units: usize, now: Instant) -> Option<(usize, u64)> {
        self.prune(now);
        let provider = self
            .used_at
            .iter()
            .enumerate()
            .filter(|(index, used)| {
                self.blocked_until[*index].is_none() && used.len().saturating_add(units) <= self.max_units
            })
            .min_by_key(|(index, used)| (used.len(), *index))
            .map(|(index, _)| index)?;
        let reservation = self.next_reservation;
        self.next_reservation = self.next_reservation.wrapping_add(1).max(1);
        self.used_at[provider].extend(std::iter::repeat_n(Charge { reservation, at: now }, units));
        Some((provider, reservation))
    }

    fn wait_for(&mut self, units: usize, now: Instant) -> Duration {
        self.prune(now);
        self.used_at
            .iter()
            .enumerate()
            .map(|(provider, used)| {
                let must_expire = used.len().saturating_add(units).saturating_sub(self.max_units);
                let quota_wait = if must_expire > 0 {
                    (used[must_expire - 1].at + self.window).saturating_duration_since(now)
                } else {
                    Duration::ZERO
                };
                let block_wait = self.blocked_until[provider]
                    .map(|deadline| deadline.saturating_duration_since(now))
                    .unwrap_or(Duration::ZERO);
                quota_wait.max(block_wait)
            })
            .min()
            .unwrap_or(Duration::ZERO)
    }

    fn refund(&mut self, provider: usize, reservation: u64, units: usize) {
        let Some(used) = self.used_at.get_mut(provider) else {
            return;
        };
        let mut remaining = units;
        used.retain(|charge| {
            if remaining > 0 && charge.reservation == reservation {
                remaining -= 1;
                false
            } else {
                true
            }
        });
    }

    fn reject(&mut self, provider: usize, reservation: u64, units: usize, retry_after: Duration, now: Instant) {
        self.refund(provider, reservation, units);
        if let Some(blocked_until) = self.blocked_until.get_mut(provider) {
            let deadline = now + retry_after;
            if blocked_until.is_none_or(|current| current < deadline) {
                *blocked_until = Some(deadline);
            }
        }
    }
}

async fn run_quota_scheduler(
    mut receiver: mpsc::Receiver<QuotaMessage>,
    provider_count: usize,
    max_units: usize,
    window: Duration,
) {
    let mut quota = SlidingQuota::new(provider_count, max_units, window);
    let mut by_actor: BTreeMap<String, VecDeque<QuotaRequest>> = BTreeMap::new();
    let mut actor_order = VecDeque::new();
    let mut input_open = true;

    loop {
        while let Ok(message) = receiver.try_recv() {
            apply_message(message, &mut quota, &mut by_actor, &mut actor_order);
        }
        discard_cancelled(&mut by_actor, &mut actor_order);

        let mut delay = None;
        if let Some(actor) = actor_order.front() {
            let request = &by_actor[actor].front().expect("scheduled actor has a request");
            let now = Instant::now();
            if let Some((provider, reservation)) = quota.take(request.units, now) {
                let actor = actor_order.pop_front().expect("front actor exists");
                let queue = by_actor.get_mut(&actor).expect("scheduled actor has a queue");
                let request = queue.pop_front().expect("scheduled actor has a request");
                if queue.is_empty() {
                    by_actor.remove(&actor);
                } else {
                    actor_order.push_back(actor);
                }
                let permit = QuotaPermit {
                    provider,
                    waited: now.saturating_duration_since(request.queued_at),
                    reservation,
                    units: request.units,
                };
                if request.reply.send(permit).is_err() {
                    // The HTTP request was cancelled between the queue sweep and grant.
                    quota.refund(provider, reservation, request.units);
                }
                continue;
            }
            delay = Some(quota.wait_for(request.units, now));
        }

        match (actor_order.is_empty(), input_open, delay) {
            (true, false, _) => return,
            (true, true, _) => match receiver.recv().await {
                Some(message) => apply_message(message, &mut quota, &mut by_actor, &mut actor_order),
                None => input_open = false,
            },
            (false, true, Some(delay)) => {
                tokio::select! {
                    request = receiver.recv() => match request {
                        Some(message) => apply_message(message, &mut quota, &mut by_actor, &mut actor_order),
                        None => input_open = false,
                    },
                    () = tokio::time::sleep(delay) => {}
                }
            }
            (false, false, Some(delay)) => tokio::time::sleep(delay).await,
            // `wait_for` always returns a duration for a non-empty queue. These cases are
            // retained as a defensive yield if that invariant changes.
            (false, _, None) => tokio::task::yield_now().await,
        }
    }
}

fn apply_message(
    message: QuotaMessage,
    quota: &mut SlidingQuota,
    by_actor: &mut BTreeMap<String, VecDeque<QuotaRequest>>,
    actor_order: &mut VecDeque<String>,
) {
    match message {
        QuotaMessage::Acquire(request) => enqueue_request(request, by_actor, actor_order),
        QuotaMessage::Control(QuotaControl::Refund { provider, reservation, units }) => {
            quota.refund(provider, reservation, units);
        }
        QuotaMessage::Control(QuotaControl::Reject { provider, reservation, units, retry_after }) => {
            quota.reject(provider, reservation, units, retry_after, Instant::now());
        }
    }
}

fn enqueue_request(
    request: QuotaRequest,
    by_actor: &mut BTreeMap<String, VecDeque<QuotaRequest>>,
    actor_order: &mut VecDeque<String>,
) {
    let actor = request.actor.clone();
    let queue = by_actor.entry(actor.clone()).or_default();
    if queue.is_empty() {
        actor_order.push_back(actor);
    }
    queue.push_back(request);
}

fn discard_cancelled(by_actor: &mut BTreeMap<String, VecDeque<QuotaRequest>>, actor_order: &mut VecDeque<String>) {
    by_actor.retain(|_, queue| {
        queue.retain(|request| !request.reply.is_closed());
        !queue.is_empty()
    });
    actor_order.retain(|actor| by_actor.contains_key(actor));
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use silicon_browser_shared::{FetchFormat, FetchItem, FetchStatus, SearchType};

    use super::*;

    #[derive(Debug)]
    struct ScriptedProvider {
        search: Mutex<VecDeque<ProviderResult<SearchResponse>>>,
        fetch: Mutex<VecDeque<ProviderResult<FetchResponse>>>,
    }

    #[async_trait]
    impl SearchProvider for ScriptedProvider {
        async fn search(&self, _request: SearchRequest) -> ProviderResult<SearchResponse> {
            self.search.lock().unwrap().pop_front().expect("scripted search response")
        }

        async fn fetch(&self, _request: FetchRequest) -> ProviderResult<FetchResponse> {
            self.fetch.lock().unwrap().pop_front().expect("scripted fetch response")
        }
    }

    #[derive(Debug, Default)]
    struct RateLimitThenStall {
        attempts: AtomicUsize,
    }

    #[async_trait]
    impl SearchProvider for RateLimitThenStall {
        async fn search(&self, _request: SearchRequest) -> ProviderResult<SearchResponse> {
            if self.attempts.fetch_add(1, Ordering::Relaxed) == 0 {
                Err(rate_limit_error(Duration::ZERO))
            } else {
                std::future::pending().await
            }
        }

        async fn fetch(&self, _request: FetchRequest) -> ProviderResult<FetchResponse> {
            std::future::pending().await
        }
    }

    fn search_request() -> SearchRequest {
        SearchRequest {
            query: "browser automation".into(),
            purpose: "test fair scheduling".into(),
            search_type: SearchType::Web,
            include_domains: vec![],
            exclude_domains: vec![],
            location: None,
            language: None,
            recency_minutes: None,
            after: None,
            before: None,
            pub_year_min: None,
            pub_year_max: None,
            page: 0,
        }
    }

    fn fetch_request(url: &str) -> FetchRequest {
        FetchRequest {
            urls: vec![url.into()],
            purpose: "test quota accounting".into(),
            format: FetchFormat::Markdown,
            links: false,
            image_links: false,
            ttl_seconds: None,
            timeout_ms: None,
            include_selectors: vec![],
            exclude_selectors: vec![],
        }
    }

    fn rate_limit_error(retry_after: Duration) -> ProviderError {
        ProviderError::Http {
            provider: PROVIDER,
            status: 429,
            message: "redacted".into(),
            retry_after: Some(retry_after),
        }
    }

    fn scripted_search_pool(
        script: VecDeque<ProviderResult<SearchResponse>>,
    ) -> (Arc<FairSearchPool>, Arc<ScriptedProvider>) {
        let provider = Arc::new(ScriptedProvider { search: Mutex::new(script), fetch: Mutex::new(VecDeque::new()) });
        let pool = Arc::new(FairSearchPool::new(vec![provider.clone() as Arc<dyn SearchProvider>]).unwrap());
        (pool, provider)
    }

    #[tokio::test]
    async fn duplicate_tinyfish_keys_are_rejected_without_disclosing_the_key() {
        let secret = "duplicate-secret-key";
        let error = FairSearchPool::from_tinyfish_api_keys([secret, secret]).unwrap_err();
        assert!(matches!(error, ProviderError::InvalidInput(_)));
        assert!(error.to_string().contains("distinct"));
        assert!(!error.to_string().contains(secret));
    }

    #[tokio::test(start_paused = true)]
    async fn zero_retry_after_uses_a_nonzero_backoff() {
        let (pool, _provider) = scripted_search_pool(VecDeque::from([
            Err(rate_limit_error(Duration::ZERO)),
            Ok(SearchResponse { results: vec![], page: 0, queued_ms: 0 }),
        ]));
        let started = Instant::now();
        let response = pool.search_for("alpha", search_request()).await.unwrap();
        assert_eq!(started.elapsed(), MIN_RATE_LIMIT_BACKOFF);
        assert_eq!(response.queued_ms, MIN_RATE_LIMIT_BACKOFF.as_millis() as u64);
    }

    #[tokio::test(start_paused = true)]
    async fn huge_retry_after_is_clamped_and_cannot_exceed_the_elapsed_budget() {
        let (pool, provider) = scripted_search_pool(VecDeque::from([
            Err(rate_limit_error(Duration::MAX)),
            Err(rate_limit_error(Duration::MAX)),
        ]));
        let started = Instant::now();
        let error = pool.search_for("alpha", search_request()).await.unwrap_err();
        assert!(matches!(error, ProviderError::Http { provider: PROVIDER, status: 429, .. }));
        assert_eq!(started.elapsed(), MAX_RATE_LIMIT_BACKOFF);
        assert!(provider.search.lock().unwrap().is_empty());
        assert!(started.elapsed() < MAX_RATE_LIMIT_ELAPSED);
    }

    #[tokio::test(start_paused = true)]
    async fn perpetual_429_stops_at_the_retry_count_with_a_typed_error() {
        let attempts = MAX_RATE_LIMIT_RETRIES + 1;
        let script = (0..attempts)
            .map(|_| Err(rate_limit_error(Duration::ZERO)))
            .collect::<VecDeque<ProviderResult<SearchResponse>>>();
        let (pool, provider) = scripted_search_pool(script);
        let started = Instant::now();
        let error = pool.search_for("alpha", search_request()).await.unwrap_err();
        assert!(matches!(error, ProviderError::Http { provider: PROVIDER, status: 429, .. }));
        assert!(provider.search.lock().unwrap().is_empty());
        assert_eq!(started.elapsed(), MIN_RATE_LIMIT_BACKOFF * MAX_RATE_LIMIT_RETRIES as u32);
    }

    #[tokio::test(start_paused = true)]
    async fn elapsed_budget_cancels_a_stalled_attempt_after_a_429() {
        let provider = Arc::new(RateLimitThenStall::default());
        let pool = FairSearchPool::new(vec![provider.clone() as Arc<dyn SearchProvider>]).unwrap();
        let started = Instant::now();
        let error = pool.search_for("alpha", search_request()).await.unwrap_err();
        assert!(matches!(error, ProviderError::Http { provider: PROVIDER, status: 429, .. }));
        assert_eq!(provider.attempts.load(Ordering::Relaxed), 2);
        assert_eq!(started.elapsed(), MAX_RATE_LIMIT_ELAPSED);
    }

    #[tokio::test(start_paused = true)]
    async fn test_group_quota_waits_and_rotates_actors_fairly() {
        let handle = tokio::runtime::Handle::current();
        let quota = FairQuota::spawn(1, 1, QUOTA_WINDOW, &handle);
        assert_eq!(quota.acquire("warmup", 1).await.unwrap().queued_ms(), 0);

        let (completed, mut completions) = mpsc::unbounded_channel();
        for (actor, sequence) in [("alpha", 1), ("alpha", 2), ("beta", 1), ("beta", 2)] {
            let quota = quota.clone();
            let completed = completed.clone();
            tokio::spawn(async move {
                let permit = quota.acquire(actor, 1).await.unwrap();
                completed.send((actor, sequence, permit.queued_ms())).unwrap();
            });
            tokio::task::yield_now().await;
        }

        for _ in 0..4 {
            tokio::time::advance(QUOTA_WINDOW).await;
            tokio::task::yield_now().await;
        }
        let mut actual = Vec::new();
        for _ in 0..4 {
            actual.push(completions.recv().await.unwrap());
        }
        assert_eq!(
            actual,
            vec![("alpha", 1, 60_000), ("beta", 1, 120_000), ("alpha", 2, 180_000), ("beta", 2, 240_000),]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_group_quota_distributes_initial_capacity_across_keys() {
        let handle = tokio::runtime::Handle::current();
        let quota = FairQuota::spawn(2, 1, QUOTA_WINDOW, &handle);
        let first = quota.acquire("alpha", 1).await.unwrap();
        let second = quota.acquire("beta", 1).await.unwrap();
        assert_eq!((first.provider, second.provider), (0, 1));

        let pending = tokio::spawn({
            let quota = quota.clone();
            async move { quota.acquire("gamma", 1).await.unwrap() }
        });
        tokio::task::yield_now().await;
        assert!(!pending.is_finished());
        tokio::time::advance(QUOTA_WINDOW).await;
        assert_eq!(pending.await.unwrap().queued_ms(), 60_000);
    }

    #[tokio::test(start_paused = true)]
    async fn test_group_quota_refunds_exact_reservation_and_rate_limit_fails_over() {
        let handle = tokio::runtime::Handle::current();
        let quota = FairQuota::spawn(2, 1, QUOTA_WINDOW, &handle);
        let first = quota.acquire("alpha", 1).await.unwrap();
        assert_eq!(first.provider, 0);
        quota.reject(first, Duration::from_secs(30)).await.unwrap();

        let second = quota.acquire("beta", 1).await.unwrap();
        assert_eq!(second.provider, 1);
        quota.refund(second, 1).await.unwrap();
        let third = quota.acquire("gamma", 1).await.unwrap();
        assert_eq!(third.provider, 1, "refund made the unblocked key immediately reusable");
    }

    #[tokio::test(start_paused = true)]
    async fn test_group_pool_requeues_429_without_charging_and_reports_wait() {
        let provider = Arc::new(ScriptedProvider {
            search: Mutex::new(VecDeque::from([
                Err(ProviderError::Http {
                    provider: PROVIDER,
                    status: 429,
                    message: "redacted".into(),
                    retry_after: Some(Duration::from_secs(7)),
                }),
                Ok(SearchResponse { results: vec![], page: 0, queued_ms: 0 }),
            ])),
            fetch: Mutex::new(VecDeque::new()),
        });
        let pool = Arc::new(FairSearchPool {
            providers: vec![provider as Arc<dyn SearchProvider>].into(),
            search_quota: FairQuota::spawn(1, 1, QUOTA_WINDOW, &tokio::runtime::Handle::current()),
            fetch_quota: FairQuota::spawn(1, 1, QUOTA_WINDOW, &tokio::runtime::Handle::current()),
        });
        let pending = tokio::spawn({
            let pool = Arc::clone(&pool);
            async move { pool.search_for("alpha", search_request()).await.unwrap() }
        });
        tokio::task::yield_now().await;
        assert!(!pending.is_finished());
        tokio::time::advance(Duration::from_secs(7)).await;
        assert_eq!(pending.await.unwrap().queued_ms, 7_000);
    }

    #[tokio::test(start_paused = true)]
    async fn test_group_pool_refunds_fetch_item_errors() {
        let url = "https://example.com";
        let error_item = FetchItem {
            url: url.into(),
            status: FetchStatus::Error,
            content: None,
            links: vec![],
            image_links: vec![],
            error: None,
            cached: false,
        };
        let ok_item = FetchItem {
            url: url.into(),
            status: FetchStatus::Ok,
            content: Some("ok".into()),
            links: vec![],
            image_links: vec![],
            error: None,
            cached: false,
        };
        let provider = Arc::new(ScriptedProvider {
            search: Mutex::new(VecDeque::new()),
            fetch: Mutex::new(VecDeque::from([
                Ok(FetchResponse { items: vec![error_item], queued_ms: 0 }),
                Ok(FetchResponse { items: vec![ok_item], queued_ms: 0 }),
            ])),
        });
        let pool = FairSearchPool {
            providers: vec![provider as Arc<dyn SearchProvider>].into(),
            search_quota: FairQuota::spawn(1, 1, QUOTA_WINDOW, &tokio::runtime::Handle::current()),
            fetch_quota: FairQuota::spawn(1, 1, QUOTA_WINDOW, &tokio::runtime::Handle::current()),
        };
        assert_eq!(pool.fetch_for("alpha", fetch_request(url)).await.unwrap().items[0].status, FetchStatus::Error);
        let second = pool.fetch_for("beta", fetch_request(url)).await.unwrap();
        assert_eq!(second.queued_ms, 0);
        assert_eq!(second.items[0].status, FetchStatus::Ok);
    }

    #[tokio::test(start_paused = true)]
    async fn pooled_batches_share_one_aggregate_output_budget() {
        let urls = (0..11).map(|index| format!("https://example.com/{index}")).collect::<Vec<_>>();
        let item = |url: &str, content: String| FetchItem {
            url: url.into(),
            status: FetchStatus::Ok,
            content: Some(content),
            links: vec![],
            image_links: vec![],
            error: None,
            cached: false,
        };
        let mut first_batch =
            vec![item(&urls[0], "x".repeat(super::super::search::MAX_FETCH_ITEMS_JSON_BYTES - 4_096))];
        first_batch.extend(urls[1..10].iter().map(|url| item(url, "ok".into())));
        let second_batch = vec![item(&urls[10], "must-not-appear-in-error".repeat(512))];
        let provider = Arc::new(ScriptedProvider {
            search: Mutex::new(VecDeque::new()),
            fetch: Mutex::new(VecDeque::from([
                Ok(FetchResponse { items: first_batch, queued_ms: 0 }),
                Ok(FetchResponse { items: second_batch, queued_ms: 0 }),
            ])),
        });
        let pool = FairSearchPool {
            providers: vec![provider as Arc<dyn SearchProvider>].into(),
            search_quota: FairQuota::spawn(1, 30, QUOTA_WINDOW, &tokio::runtime::Handle::current()),
            fetch_quota: FairQuota::spawn(1, 150, QUOTA_WINDOW, &tokio::runtime::Handle::current()),
        };
        let error = pool
            .fetch_for(
                "alpha",
                FetchRequest {
                    urls,
                    purpose: "exercise the aggregate boundary".into(),
                    format: FetchFormat::Markdown,
                    links: false,
                    image_links: false,
                    ttl_seconds: None,
                    timeout_ms: None,
                    include_selectors: vec![],
                    exclude_selectors: vec![],
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(error, ProviderError::InvalidResponse { provider: PROVIDER, .. }));
        assert!(!error.to_string().contains("must-not-appear-in-error"));
    }

    #[tokio::test]
    async fn test_group_quota_queue_is_bounded_across_scheduler_storage() {
        let (sender, _receiver) = mpsc::channel(1);
        let quota = FairQuota { sender, queue_slots: Arc::new(Semaphore::new(1)), max_units: 1 };
        let first = tokio::spawn({
            let quota = quota.clone();
            async move { quota.acquire("alpha", 1).await }
        });
        tokio::task::yield_now().await;
        let error = quota.acquire("beta", 1).await.unwrap_err();
        assert!(matches!(error, ProviderError::Overloaded { capacity: FAIR_SEARCH_QUEUE_CAPACITY, .. }));
        first.abort();
    }
}
