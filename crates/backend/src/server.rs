//! Axum HTTP boundary for the public, stateless `silicon-browser` client.
//!
//! Every scoped operation re-introspects the opaque IAM bearer in the explicit
//! `X-Org-ID` context. Provider credentials stay private; an authorized client
//! receives its sensitive CDP capability and controls the remote browser directly.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Weak};
use std::time::Duration;

#[cfg(test)]
use axum::body::Body;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{DefaultBodyLimit, FromRequestParts, Path, Query, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use silicon_browser_shared::{
    ApiError, ApiErrorEnvelope, AuthExchangeRequest, AuthRefreshRequest, AuthSession, CommandReport, Envelope,
    FetchRequest, FetchResponse, FieldError, IamInfo, Identity, LiveRedeemRequest, Org, ProfileCreate, ProfileEnd,
    ProfileUpdate, ProxyLocation, RecordingFilter, SearchRequest, SearchResponse, Session, SessionConnection,
    SessionCreate, SessionEnd, SessionFilter, SessionStatus, UsageFilter, UsagePredicate, Validate,
};
use tokio::sync::Mutex;
use url::Url;
use uuid::Uuid;

use crate::auth::{
    ExchangeRequest, ExchangedAuth, IdentityError, IdentityProvider, OrganizationAccess, PrincipalIdentity,
    RefreshRequest, UpstreamFailure,
};
use crate::crypto::SecretBox;
use crate::decimal::decimal_to_millionths;
use crate::providers::{
    BrowserProvider, CreateBrowserProfile, FairSearchPool, ProviderBrowserSession, ProviderError, ProviderProfile,
    StartBrowser, UpdateBrowserProfile, proxy_locations as provider_proxy_locations,
};
use crate::store::{
    FailedSessionFinalization, ProviderSession, RECORDING_SOURCE_INITIAL_DELAY, RecordingSourceClaim, Store,
    StoreError, UsageSample,
};
use crate::url_policy::{has_forbidden_host, is_https_or_loopback_http};

mod delivery;
mod usage_limits;

const MAX_REQUEST_BODY: usize = 2 * 1024 * 1024;
const PROFILE_RECONCILIATION_ITEM_TIMEOUT: Duration = Duration::from_secs(10);
const PROFILE_RECONCILIATION_CONCURRENCY: usize = 8;
// stop_browser_confirmed can spend one 70-second Browser Use timeout on PATCH
// and another on its GET reconciliation. The attempt budget leaves 40 seconds
// for durable finalization, and the lease adds another 30
// seconds so a second process cannot reclaim work which is still in flight.
const TTL_STOP_ATTEMPT_BUDGET: Duration = Duration::from_secs(180);
const TTL_STOP_LEASE: Duration = Duration::from_secs(210);
const TTL_MAINTENANCE_BUDGET: Duration = Duration::from_secs(195);
const TTL_STOP_CONCURRENCY: u32 = 8;
// Recording resolution needs only one Browser Use GET. Its per-item timeout is
// longer than the adapter's 70-second request timeout, with separate cycle and
// lease headroom for rescheduling the durable claim.
const RECORDING_RECONCILIATION_ITEM_TIMEOUT: Duration = Duration::from_secs(75);
const RECORDING_RECONCILIATION_CYCLE_BUDGET: Duration = Duration::from_secs(85);
const RECORDING_RECONCILIATION_LEASE: Duration = Duration::from_secs(120);
const RECORDING_RECONCILIATION_BATCH: u32 = 8;
const RECORDING_RECONCILIATION_MAX_ATTEMPTS: u32 = 10;
const RECORDING_RECONCILIATION_MAX_BACKOFF: Duration = Duration::from_secs(300);

#[derive(Clone)]
pub struct AppState {
    store: Store,
    secrets: SecretBox,
    identity: Arc<dyn IdentityProvider>,
    browser: Arc<dyn BrowserProvider>,
    search: Option<Arc<FairSearchPool>>,
    public_origin: Arc<str>,
    proxy_locations: Arc<Vec<ProxyLocation>>,
    session_locks: SessionLocks,
    usage_limits: usage_limits::UsageLimitsCache,
    recording_delivery: Option<Arc<delivery::RecordingDeliveryServices>>,
    recording_delivery_required: bool,
}

impl AppState {
    pub fn new(
        public_origin: impl AsRef<str>,
        store: Store,
        secrets: SecretBox,
        identity: Arc<dyn IdentityProvider>,
        browser: Arc<dyn BrowserProvider>,
        search: Option<Arc<FairSearchPool>>,
    ) -> Result<Self, String> {
        let parsed = Url::parse(public_origin.as_ref()).map_err(|_| "SB_ORIGIN must be an HTTP(S) URL")?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || parsed.path() != "/"
            || !is_https_or_loopback_http(&parsed)
        {
            return Err("SB_ORIGIN must be an HTTP(S) URL".into());
        }
        Ok(Self {
            store,
            secrets,
            identity,
            browser,
            search,
            public_origin: Arc::from(parsed.as_str().trim_end_matches('/')),
            proxy_locations: Arc::new(default_proxy_locations()),
            session_locks: SessionLocks::default(),
            usage_limits: usage_limits::UsageLimitsCache::default(),
            recording_delivery: None,
            recording_delivery_required: false,
        })
    }

    pub fn with_proxy_locations(mut self, locations: Vec<ProxyLocation>) -> Self {
        self.proxy_locations = Arc::new(locations);
        self
    }

    /// Reconcile provider profiles whose create succeeded (or was ambiguous)
    /// before local activation committed. Browser Use exposes the reserved
    /// local id as an exact `userId` lookup, making this retry read-only and
    /// safe across multiple service processes.
    pub async fn reconcile_profiles_once(&self) -> Result<usize, StoreError> {
        self.reconcile_profiles_once_with_timeout(PROFILE_RECONCILIATION_ITEM_TIMEOUT).await
    }

    async fn reconcile_profiles_once_with_timeout(&self, item_timeout: Duration) -> Result<usize, StoreError> {
        let pending = self.store.provisioning_profiles().await?;
        let mut activated = 0;
        for batch in pending.chunks(PROFILE_RECONCILIATION_CONCURRENCY) {
            let mut workers = tokio::task::JoinSet::new();
            for profile in batch.iter().cloned() {
                let state = self.clone();
                workers.spawn(async move {
                    let profile_id = profile.profile_id.clone();
                    match tokio::time::timeout(item_timeout, state.reconcile_profile(profile)).await {
                        Ok(value) => value,
                        Err(_) => {
                            tracing::warn!(
                                %profile_id,
                                timeout_ms = item_timeout.as_millis(),
                                "provider profile reconciliation item timed out"
                            );
                            false
                        }
                    }
                });
            }
            while let Some(result) = workers.join_next().await {
                match result {
                    Ok(true) => activated += 1,
                    Ok(false) => {}
                    Err(error) => tracing::error!(error = %error, "profile reconciliation worker failed"),
                }
            }
        }
        Ok(activated)
    }

    async fn reconcile_profile(&self, profile: crate::store::ProvisioningProfile) -> bool {
        match self.browser.find_profile_by_user_id(&profile.profile_id).await {
            Ok(Some(provider)) => {
                if let Err(error) = validate_provider_profile_identity(&provider, &profile.profile_id) {
                    tracing::warn!(
                        profile_id = %profile.profile_id,
                        error_kind = provider_error_kind(&error),
                        "profile reconciliation returned an invalid provider identity"
                    );
                    return false;
                }
                match self
                    .store
                    .activate_profile(
                        &profile.org_id,
                        &profile.profile_id,
                        &provider.id,
                        &profile_fingerprint(&profile.profile_id),
                    )
                    .await
                {
                    Ok(_) => true,
                    Err(StoreError::SessionState { .. }) => false,
                    Err(error) => {
                        tracing::warn!(
                            profile_id = %profile.profile_id,
                            error = %error,
                            "could not activate a reconciled provider profile"
                        );
                        false
                    }
                }
            }
            Ok(None) => false,
            Err(error) => {
                tracing::warn!(
                    profile_id = %profile.profile_id,
                    error_kind = provider_error_kind(&error),
                    "provider profile reconciliation is temporarily unavailable"
                );
                false
            }
        }
    }

    /// Reconcile provider recording URLs which can materialize shortly after a
    /// browser has stopped. Claims are durably leased in SQLite, provider
    /// failures are isolated per recording, and a fixed attempt cap prevents an
    /// unavailable recording from retrying forever.
    pub async fn reconcile_recording_sources_once(&self, now: DateTime<Utc>) -> Result<usize, StoreError> {
        self.reconcile_recording_sources_once_with_budgets(
            now,
            RECORDING_RECONCILIATION_ITEM_TIMEOUT,
            RECORDING_RECONCILIATION_CYCLE_BUDGET,
        )
        .await
    }

    async fn reconcile_recording_sources_once_with_budgets(
        &self,
        now: DateTime<Utc>,
        item_timeout: Duration,
        cycle_budget: Duration,
    ) -> Result<usize, StoreError> {
        let started = tokio::time::Instant::now();
        let deadline = started + cycle_budget;
        let mut count = 0;
        loop {
            if deadline.saturating_duration_since(tokio::time::Instant::now()) < item_timeout {
                break;
            }
            let batch_now = now
                + TimeDelta::from_std(started.elapsed())
                    .map_err(|_| StoreError::Invalid("recording reconciliation elapsed time is too large".into()))?;
            let lease_until = batch_now
                + TimeDelta::from_std(RECORDING_RECONCILIATION_LEASE)
                    .map_err(|_| StoreError::Invalid("recording reconciliation lease is too large".into()))?;
            let claims = self
                .store
                .claim_recording_source_resolutions(batch_now, lease_until, RECORDING_RECONCILIATION_BATCH)
                .await?;
            let batch_size = claims.len();
            if batch_size == 0 {
                break;
            }
            count += batch_size;
            let mut workers = tokio::task::JoinSet::new();
            for claim in claims {
                let state = self.clone();
                workers.spawn(async move {
                    let retained = claim.clone();
                    if tokio::time::timeout(item_timeout, state.reconcile_recording_source(claim, batch_now))
                        .await
                        .is_err()
                    {
                        tracing::warn!(
                            session_id = %retained.session_id,
                            timeout_ms = item_timeout.as_millis(),
                            "provider recording lookup timed out"
                        );
                        state.defer_or_exhaust_recording_source(&retained, batch_now, None).await;
                    }
                });
            }
            while let Some(result) = workers.join_next().await {
                if let Err(error) = result {
                    tracing::error!(error = %error, "recording reconciliation worker failed");
                }
            }
            if batch_size < usize::try_from(RECORDING_RECONCILIATION_BATCH).expect("small constant fits usize") {
                break;
            }
        }
        Ok(count)
    }

    async fn reconcile_recording_source(&self, claim: RecordingSourceClaim, now: DateTime<Utc>) {
        // A previous lookup may have been cancelled by the outer maintenance
        // budget after the durable claim incremented. Finish an over-limit
        // intent before issuing another provider request so even a provider
        // which hangs on every GET cannot retry forever.
        if claim.attempt > RECORDING_RECONCILIATION_MAX_ATTEMPTS {
            self.defer_or_exhaust_recording_source(&claim, now, None).await;
            return;
        }
        let provider = self
            .browser
            .get_browser(&claim.provider_session_id)
            .await
            .and_then(|provider| validated_terminal_provider_session(provider, &claim.provider_session_id));
        match provider {
            Ok(provider) => {
                if let Some(recording_url) = provider.recording_url.as_deref() {
                    match self
                        .store
                        .complete_recording_source_resolution(&claim, recording_url, &self.secrets, now)
                        .await
                    {
                        Ok(_) => return,
                        Err(error) => {
                            tracing::error!(
                                session_id = %claim.session_id,
                                error = %error,
                                "could not persist a materialized recording source"
                            );
                            return;
                        }
                    }
                }
                if provider.recording_available == Some(false) {
                    if let Err(error) = self.store.fail_recording_source_resolution(&claim, now).await {
                        tracing::error!(
                            session_id = %claim.session_id,
                            error = %error,
                            "could not terminalize an unavailable provider recording"
                        );
                    }
                    return;
                }
                self.defer_or_exhaust_recording_source(&claim, now, None).await;
            }
            Err(error) => {
                self.defer_or_exhaust_recording_source(&claim, now, Some(&error)).await;
            }
        }
    }

    async fn defer_or_exhaust_recording_source(
        &self,
        claim: &RecordingSourceClaim,
        now: DateTime<Utc>,
        provider_error: Option<&ProviderError>,
    ) {
        if claim.attempt >= RECORDING_RECONCILIATION_MAX_ATTEMPTS {
            match self.store.fail_recording_source_resolution(claim, now).await {
                Ok(true) => tracing::warn!(
                    session_id = %claim.session_id,
                    attempts = claim.attempt,
                    "provider recording URL did not materialize before the retry limit"
                ),
                Ok(false) => {}
                Err(error) => tracing::error!(
                    session_id = %claim.session_id,
                    error = %error,
                    "could not terminalize an exhausted recording resolution"
                ),
            }
            return;
        }

        let delay = recording_reconciliation_backoff(claim.attempt);
        let next_attempt_at = now
            + TimeDelta::from_std(delay).expect("bounded recording reconciliation delays always fit chrono::TimeDelta");
        match self.store.reschedule_recording_source_resolution(claim, next_attempt_at).await {
            Ok(true) => {
                if let Some(error) = provider_error {
                    tracing::warn!(
                        session_id = %claim.session_id,
                        attempt = claim.attempt,
                        error_kind = provider_error_kind(error),
                        retry_after_ms = delay.as_millis(),
                        "provider recording lookup will be retried"
                    );
                }
            }
            Ok(false) => {}
            Err(error) => tracing::error!(
                session_id = %claim.session_id,
                error = %error,
                "could not reschedule recording source resolution"
            ),
        }
    }

    pub async fn reap_expired_once(&self) -> Result<usize, StoreError> {
        self.reap_expired_once_with_profile_budget(PROFILE_RECONCILIATION_ITEM_TIMEOUT).await
    }

    async fn reap_expired_once_with_profile_budget(&self, profile_item_timeout: Duration) -> Result<usize, StoreError> {
        // Keep this combined entrypoint for callers which want one maintenance
        // pass, but never let profile-provider latency delay expiry work.
        let (expired, profiles) = tokio::join!(
            self.reap_expired_sessions_once(),
            self.reconcile_profiles_once_with_timeout(profile_item_timeout)
        );
        if let Err(error) = profiles {
            tracing::warn!(error = %error, "profile reconciliation iteration failed");
        }
        expired
    }

    async fn reap_expired_sessions_once(&self) -> Result<usize, StoreError> {
        self.reap_expired_sessions_once_with_budgets(TTL_STOP_ATTEMPT_BUDGET, TTL_MAINTENANCE_BUDGET).await
    }

    async fn reap_expired_sessions_once_with_budgets(
        &self,
        attempt_budget: Duration,
        cycle_budget: Duration,
    ) -> Result<usize, StoreError> {
        let started = tokio::time::Instant::now();
        let deadline = started + cycle_budget;
        let mut count = 0;
        loop {
            if deadline.saturating_duration_since(tokio::time::Instant::now()) < attempt_budget {
                break;
            }
            let now = Utc::now();
            let lease_until = now
                + TimeDelta::from_std(TTL_STOP_LEASE)
                    .map_err(|_| StoreError::Invalid("TTL stop lease is too large".into()))?;
            let expired =
                self.store.claim_expired_sessions(now, lease_until, TTL_STOP_CONCURRENCY, &self.secrets).await?;
            let batch_size = expired.len();
            if batch_size == 0 {
                break;
            }
            count += batch_size;

            // Claim no more work than can start immediately. Each provider
            // stop is bounded independently, so one hung browser cannot hold
            // the rest of the batch behind it or consume their leases unused.
            let mut workers = tokio::task::JoinSet::new();
            for session in expired {
                let state = self.clone();
                workers.spawn(async move {
                    let session_id = session.session_id.clone();
                    if tokio::time::timeout(attempt_budget, state.reap_claimed_session(&session)).await.is_err() {
                        // Do not release an ambiguous timed-out stop: the
                        // provider may have committed it. The bounded lease
                        // makes it retryable after the confirmation window.
                        tracing::warn!(
                            %session_id,
                            timeout_ms = attempt_budget.as_millis(),
                            "TTL stop attempt timed out; retaining its durable lease"
                        );
                    }
                });
            }
            while let Some(result) = workers.join_next().await {
                if let Err(error) = result {
                    tracing::error!(error = %error, "TTL stop worker failed");
                }
            }
            if batch_size < usize::try_from(TTL_STOP_CONCURRENCY).expect("small constant fits usize") {
                break;
            }
        }
        Ok(count)
    }

    async fn reap_claimed_session(&self, session: &crate::store::ExpiringSession) {
        let gate = self.session_locks.for_session(&session.org_id, &session.session_id).await;
        let _guard = gate.lock.lock().await;

        // A manual end may have completed while this claim waited for the
        // in-process session gate. Revalidate both state and lease under the
        // gate so the reaper cannot issue a second provider stop.
        match self
            .store
            .expired_claim_is_current(&session.org_id, &session.session_id, &session.lease_id, Utc::now())
            .await
        {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                tracing::error!(session_id = %session.session_id, error = %error, "could not validate TTL stop claim");
                self.release_expired_claim(session).await;
                return;
            }
        }

        let provider = match stop_browser_confirmed(self.browser.as_ref(), &session.runtime.provider_session_id).await {
            Ok(provider) => provider,
            Err(error) => {
                tracing::warn!(
                    session_id = %session.session_id,
                    provider = "browser-use",
                    error_kind = provider_error_kind(&error),
                    "TTL reaper could not confirm browser stop; session remains ending"
                );
                self.release_expired_claim(session).await;
                return;
            }
        };

        let has_recording_source = if let Some(url) = provider.recording_url.as_deref() {
            if let Err(error) = self
                .store
                .remember_provider_recording_url(&session.org_id, &session.session_id, url, &self.secrets)
                .await
            {
                tracing::error!(session_id = %session.session_id, error = %error, "could not persist recording source");
                self.release_expired_claim(session).await;
                return;
            }
            true
        } else {
            session.runtime.recording_url.is_some()
        };
        let now = Utc::now();
        if let Err(error) = record_provider_usage(
            &self.store,
            UsageSubject {
                org_id: &session.org_id,
                session_id: &session.session_id,
                incognito: session.incognito,
                started_at: session.started_at,
            },
            &provider,
            now,
            UsageObservation::Terminal,
        )
        .await
        {
            tracing::error!(session_id = %session.session_id, error = %error, "could not persist final provider usage");
            self.release_expired_claim(session).await;
            return;
        }
        let duration = browser_duration_ms(&provider, session.started_at, now);
        let recording = if has_recording_source {
            self.store.mark_recording_pending(&session.org_id, &session.session_id, duration, 0).await
        } else {
            self.store.queue_recording_source_resolution(&session.org_id, &session.session_id, duration, now).await
        };
        if let Err(error) = recording {
            tracing::error!(session_id = %session.session_id, error = %error, "could not enqueue recording storage");
            self.release_expired_claim(session).await;
            return;
        }
        if let Err(error) =
            self.store.finalize_expired_session(&session.org_id, &session.session_id, &session.lease_id, now).await
        {
            tracing::error!(session_id = %session.session_id, error = %error, "could not finalize expired session");
            self.release_expired_claim(session).await;
        }
    }

    async fn release_expired_claim(&self, session: &crate::store::ExpiringSession) {
        if let Err(error) =
            self.store.release_expired_claim(&session.org_id, &session.session_id, &session.lease_id).await
        {
            tracing::error!(session_id = %session.session_id, error = %error, "could not release TTL stop claim");
        }
    }
}

/// Native API middleware. `SB_ORIGIN` is the separately hosted frontend origin.
pub fn production_router(state: AppState) -> Router {
    let origin = HeaderValue::from_str(&state.public_origin).expect("AppState validates an HTTP origin");
    router(state)
        .layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(origin)
                .allow_methods([Method::GET, Method::POST, Method::PATCH])
                .allow_headers([AUTHORIZATION, CONTENT_TYPE, HeaderName::from_static("x-org-id")])
                .expose_headers([
                    HeaderName::from_static("x-sb-auth-rejected"),
                    HeaderName::from_static("x-request-id"),
                ]),
        )
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .layer(tower_http::sensitive_headers::SetSensitiveRequestHeadersLayer::new([AUTHORIZATION]))
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/api/v1/iam", get(iam))
        .route("/api/v1/auth/exchange", post(exchange_auth))
        .route("/api/v1/auth/refresh", post(refresh_auth))
        .route("/api/v1/auth/delivery", get(delivery::authorization_status).post(delivery::authorize))
        .route("/api/v1/auth/delivery/end", post(delivery::disable_authorization))
        .route("/api/v1/recordings/{session_id}/retry", post(delivery::retry_recording))
        .route("/api/v1/me", get(me))
        .route("/api/v1/orgs", get(orgs))
        .route("/api/v1/services", get(services))
        .route("/api/v1/proxy-locations", get(proxy_locations))
        .route("/api/v1/profiles", get(list_profiles).post(create_profile))
        .route("/api/v1/profiles/{profile_id}", get(get_profile).patch(update_profile))
        .route("/api/v1/profiles/{profile_id}/end", post(end_profile))
        .route("/api/v1/sessions", get(list_sessions).post(create_session))
        .route("/api/v1/sessions/{session_id}", get(get_session))
        .route("/api/v1/sessions/{session_id}/end", post(end_session))
        .route("/api/v1/sessions/{session_id}/live", post(live_session))
        .route("/api/v1/sessions/{session_id}/live/redeem", post(redeem_live_session))
        .route("/api/v1/sessions/{session_id}/logs", get(session_logs))
        .route("/api/v1/sessions/{session_id}/commands", post(report_command))
        .route("/api/v1/sessions/{session_id}/connection", get(session_connection))
        .route("/api/v1/recordings", get(list_recordings))
        .route("/api/v1/recordings/{session_id}", get(get_recording))
        .route("/api/v1/recordings/{session_id}/trash", post(trash_recording))
        .route("/api/v1/usage/org", get(org_usage))
        .route("/api/v1/usage/limits", get(usage_limits::get_usage_limits))
        .route("/api/v1/usage", get(list_usage))
        .route("/api/v1/usage/{session_id}", get(get_usage))
        .route("/api/v1/search", post(search))
        .route("/api/v1/fetch", post(fetch))
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY))
        .with_state(state)
}

#[derive(Default)]
struct SessionGate {
    lock: Mutex<()>,
}

#[derive(Clone, Default)]
struct SessionLocks(Arc<Mutex<HashMap<String, Weak<SessionGate>>>>);

impl SessionLocks {
    async fn for_session(&self, org_id: &str, session_id: &str) -> Arc<SessionGate> {
        let key = format!("{org_id}\0{session_id}");
        let mut locks = self.0.lock().await;
        locks.retain(|_, lock| lock.strong_count() != 0);
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(SessionGate::default());
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }
}

#[derive(Clone)]
struct Scope {
    org_id: String,
    identity: Identity,
    principal: PrincipalIdentity,
}

impl FromRequestParts<AppState> for Scope {
    type Rejection = ApiFailure;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let bearer = bearer(&parts.headers).map_err(ApiFailure::before_handler)?;
        let org_id = required_header(&parts.headers, "x-org-id", "organization scope is required")?;
        authenticate_scope(state, &bearer, &org_id).await.map_err(ApiFailure::before_handler)
    }
}

async fn authenticate_scope(state: &AppState, bearer: &str, org_id: &str) -> Result<Scope, ApiFailure> {
    let principal = state.identity.identify(bearer, org_id).await.map_err(ApiFailure::from)?;
    if principal.org_id != org_id {
        return Err(ApiFailure::forbidden("IAM returned a different organization scope"));
    }
    let identity = resolve_identity(state, &principal).await?;
    Ok(Scope { org_id: org_id.to_owned(), identity, principal })
}

#[derive(Clone)]
struct Bearer(String);

impl FromRequestParts<AppState> for Bearer {
    type Rejection = ApiFailure;

    async fn from_request_parts(parts: &mut Parts, _state: &AppState) -> Result<Self, Self::Rejection> {
        Ok(Self(bearer(&parts.headers).map_err(ApiFailure::before_handler)?))
    }
}

async fn resolve_identity(state: &AppState, principal: &PrincipalIdentity) -> Result<Identity, ApiFailure> {
    let principal_id = principal.principal_id.to_string();
    let projected = state
        .store
        .projected_identity(&principal.org_id, &principal_id, principal.kind)
        .await
        .map_err(ApiFailure::from)?;
    let mut identity = match (principal.public_id.as_deref(), projected) {
        (Some(public_id), Some(projected)) if projected.id == public_id => projected,
        (Some(public_id), _) => state
            .store
            .remember_identity_projection(
                &principal.org_id,
                &principal_id,
                public_id,
                principal.kind,
                &state.secrets,
                Utc::now(),
            )
            .await
            .map_err(ApiFailure::from)?,
        (None, Some(projected)) => projected,
        (None, None) => Identity {
            id: principal_id.clone(),
            name: principal_id,
            kind: principal.kind,
            tags: Vec::new(),
            verified_aliases: Vec::new(),
        },
    };
    // Membership disclosure belongs to this exact live token. Never recover
    // tags from a persisted identity or a broader token's previous snapshot.
    identity.tags = principal.tags.clone().unwrap_or_default();
    Ok(identity)
}

async fn health() -> impl IntoResponse {
    success(Health { status: "ok" })
}

async fn iam(State(state): State<AppState>) -> impl IntoResponse {
    success(IamInfo { app_id: state.identity.app_id().to_owned() })
}

async fn exchange_auth(
    State(state): State<AppState>,
    payload: Result<Json<AuthExchangeRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let request = json_payload(payload)?;
    request.validate().map_err(ApiFailure::validation)?;
    let requested_org = request.org_id;
    let exchanged = state
        .identity
        .exchange_short_lived_token(ExchangeRequest {
            idempotency_key: stable_idempotency(
                "exchange",
                &request.short_lived_token,
                requested_org.as_deref().unwrap_or("unscoped"),
            ),
            short_lived_token: request.short_lived_token,
            required_org_id: requested_org,
        })
        .await
        .map_err(ApiFailure::from)?;
    let session = auth_session(&state, exchanged).await?;
    let mut response = success(session).into_response();
    response.headers_mut().insert("cache-control", HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn refresh_auth(
    State(state): State<AppState>,
    payload: Result<Json<AuthRefreshRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let request = json_payload(payload)?;
    request.validate().map_err(ApiFailure::validation)?;
    let exchanged = state
        .identity
        .refresh(RefreshRequest {
            idempotency_key: stable_idempotency("refresh", &request.refresh_token, &request.org_id),
            refresh_token: request.refresh_token,
            required_org_id: request.org_id,
        })
        .await
        .map_err(ApiFailure::from)?;
    let session = auth_session(&state, exchanged).await?;
    let mut response = success(session).into_response();
    response.headers_mut().insert("cache-control", HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn auth_session(state: &AppState, exchanged: ExchangedAuth) -> Result<AuthSession, ApiFailure> {
    let identity = resolve_identity(state, &exchanged.identity).await?;
    let org = Org { id: exchanged.identity.org_id.clone(), name: exchanged.identity.org_id.clone() };
    // IAM's exchange `scope` is the webhook event catalogue, not a list of
    // operations the resulting OAT may perform. Organization-bound OAT
    // introspection and membership are therefore the authorization authority.
    let _iam_webhook_scope = exchanged.scope;
    Ok(AuthSession {
        access_token: exchanged.access_token,
        refresh_token: exchanged.refresh_token,
        expires_at: exchanged.identity.expires_at,
        identity,
        org,
        services: available_services(state),
    })
}

async fn me(scope: Scope) -> impl IntoResponse {
    success(scope.identity)
}

async fn services(State(state): State<AppState>, _scope: Scope) -> impl IntoResponse {
    success(available_services(&state))
}

async fn orgs(State(state): State<AppState>, bearer: Bearer) -> Result<impl IntoResponse, ApiFailure> {
    let orgs = state.identity.orgs(&bearer.0).await.map_err(ApiFailure::from)?;
    Ok(success(orgs.into_iter().map(public_org).collect::<Vec<_>>()))
}

async fn proxy_locations(State(state): State<AppState>, _scope: Scope) -> impl IntoResponse {
    success((*state.proxy_locations).clone())
}

async fn list_profiles(
    State(state): State<AppState>,
    scope: Scope,
    query: Result<Query<FilterQuery>, QueryRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let query = query_payload(query)?;
    reject_profile_filter(query.filter.as_deref())?;
    Ok(success(state.store.profiles(&scope.org_id, &scope.identity).await.map_err(ApiFailure::from)?))
}

async fn get_profile(
    State(state): State<AppState>,
    scope: Scope,
    Path(profile_id): Path<String>,
) -> Result<impl IntoResponse, ApiFailure> {
    Ok(success(state.store.profile(&scope.org_id, &scope.identity, &profile_id).await.map_err(ApiFailure::from)?))
}

async fn create_profile(
    State(state): State<AppState>,
    scope: Scope,
    payload: Result<Json<ProfileCreate>, JsonRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let request = json_payload(payload)?;
    request.validate().map_err(ApiFailure::validation)?;
    if !state.proxy_locations.iter().any(|location| location.code.eq_ignore_ascii_case(request.location.trim())) {
        return Err(ApiFailure::field("location", "location is not supported by the configured browser provider"));
    }
    let reserved = state
        .store
        .reserve_profile(&scope.org_id, &scope.identity, &request, Utc::now())
        .await
        .map_err(ApiFailure::from)?;
    let provider = match state
        .browser
        .create_profile(CreateBrowserProfile { name: request.name.clone(), user_id: reserved.id.clone() })
        .await
    {
        Ok(provider) => provider,
        Err(error) => {
            let ambiguous = provider_mutation_may_have_committed(&error);
            if ambiguous {
                match state.browser.find_profile_by_user_id(&reserved.id).await {
                    Ok(Some(provider)) => {
                        validate_provider_profile_identity(&provider, &reserved.id).map_err(ApiFailure::from)?;
                        let profile = state
                            .store
                            .activate_profile(
                                &scope.org_id,
                                &reserved.id,
                                &provider.id,
                                &profile_fingerprint(&reserved.id),
                            )
                            .await
                            .map_err(ApiFailure::from)?;
                        return Ok(success(profile));
                    }
                    Ok(None) => {
                        // Exact userId lookup authoritatively proved the
                        // provider did not commit the ambiguous create.
                    }
                    Err(reconciliation_error) => {
                        tracing::warn!(
                            profile_id = %reserved.id,
                            error_kind = provider_error_kind(&reconciliation_error),
                            "ambiguous profile create retained for background reconciliation"
                        );
                        return Err(ApiFailure::from(error));
                    }
                }
            }
            let _ = state
                .store
                .fail_profile(&scope.org_id, &reserved.id, "upstream profile provisioning failed", Utc::now())
                .await;
            return Err(ApiFailure::from(error));
        }
    };
    if let Err(error) = validate_provider_profile_identity(&provider, &reserved.id) {
        if let Err(store_error) = state
            .store
            .fail_profile(
                &scope.org_id,
                &reserved.id,
                "upstream profile response did not match its reservation",
                Utc::now(),
            )
            .await
        {
            tracing::error!(
                profile_id = %reserved.id,
                error = %store_error,
                "could not fail a profile after an invalid provider identity response"
            );
        }
        return Err(ApiFailure::from(error));
    }
    // This is Silicon Browser's immutable profile identity, not a claim that
    // Browser Use exposes its internal anti-detect attribute bundle.
    match state
        .store
        .activate_profile(&scope.org_id, &reserved.id, &provider.id, &profile_fingerprint(&reserved.id))
        .await
    {
        Ok(profile) => Ok(success(profile)),
        Err(error) => {
            // The provider has already committed a profile correlated by this
            // reserved local id. Preserve provisioning state so the periodic
            // exact userId lookup can complete activation after database
            // recovery; failing it here would orphan the upstream resource.
            tracing::warn!(profile_id = %reserved.id, "local profile activation deferred for reconciliation");
            Err(ApiFailure::from(error))
        }
    }
}

async fn update_profile(
    State(state): State<AppState>,
    scope: Scope,
    Path(profile_id): Path<String>,
    payload: Result<Json<ProfileUpdate>, JsonRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let request = json_payload(payload)?;
    request.validate().map_err(ApiFailure::validation)?;
    let profile = state.store.profile(&scope.org_id, &scope.identity, &profile_id).await.map_err(ApiFailure::from)?;
    if !scope.identity.matches_principal(&profile.owner_id) {
        return Err(ApiFailure::forbidden("only the profile owner may update it"));
    }
    if request.name.is_some() {
        let provider_id = state
            .store
            .provider_profile_id(&scope.org_id, &scope.identity, &profile_id)
            .await
            .map_err(ApiFailure::from)?;
        let provider = state
            .browser
            .update_profile(&provider_id, UpdateBrowserProfile { name: request.name.clone(), user_id: None })
            .await
            .map_err(ApiFailure::from)?;
        validate_provider_profile_identity(&provider, &profile_id).map_err(ApiFailure::from)?;
        if provider.id != provider_id {
            return Err(ApiFailure::from(provider_contract(
                "provider profile update returned a different resource id",
            )));
        }
    }
    Ok(success(
        state
            .store
            .update_profile(&scope.org_id, &scope.identity, &profile_id, &request)
            .await
            .map_err(ApiFailure::from)?,
    ))
}

async fn end_profile(
    State(state): State<AppState>,
    scope: Scope,
    Path(profile_id): Path<String>,
    payload: Result<Json<ProfileEnd>, JsonRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let request = json_payload(payload)?;
    request.validate().map_err(ApiFailure::validation)?;
    Ok(success(
        state
            .store
            .end_profile(&scope.org_id, &scope.identity, &profile_id, &request, Utc::now())
            .await
            .map_err(ApiFailure::from)?,
    ))
}

async fn list_sessions(
    State(state): State<AppState>,
    scope: Scope,
    query: Result<Query<FilterQuery>, QueryRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let query = query_payload(query)?;
    let filter = parse_session_filter(query.filter.as_deref())?;
    let mut sessions = state.store.sessions(&scope.org_id, &scope.identity).await.map_err(ApiFailure::from)?;
    if let Some(filter) = filter {
        sessions.retain(|session| filter.matches(session, &scope.identity.id));
    }
    Ok(success(sessions))
}

async fn get_session(
    State(state): State<AppState>,
    scope: Scope,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiFailure> {
    let mut session =
        state.store.session(&scope.org_id, &scope.identity, &session_id).await.map_err(ApiFailure::from)?;
    if refresh_active_usage(&state, &scope, &session).await {
        session = state.store.session(&scope.org_id, &scope.identity, &session_id).await.map_err(ApiFailure::from)?;
    }
    Ok(success(session))
}

async fn create_session(
    State(state): State<AppState>,
    scope: Scope,
    payload: Result<Json<SessionCreate>, JsonRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let request = json_payload(payload)?;
    request.validate().map_err(ApiFailure::validation)?;
    let delivery_binding = state.require_recording_authorization(&scope).await?;
    // Resolve the visible, active profile before reserving its exclusive
    // session slot. A failed lookup therefore cannot strand a `starting` row.
    // reserve_session repeats the state/access check atomically to close the
    // race with profile retirement or another browser start.
    let (provider_profile_id, proxy_country_code) = if let Some(local_profile_id) = request.profile_id.as_deref() {
        let profile =
            state.store.profile(&scope.org_id, &scope.identity, local_profile_id).await.map_err(ApiFailure::from)?;
        let provider_id = state
            .store
            .provider_profile_id(&scope.org_id, &scope.identity, local_profile_id)
            .await
            .map_err(ApiFailure::from)?;
        (Some(provider_id), Some(profile.location.code))
    } else {
        (None, None)
    };
    let reserved = state
        .store
        .reserve_session(&scope.org_id, &scope.identity, &request, Utc::now())
        .await
        .map_err(ApiFailure::from)?;
    if let Some((principal, membership)) = delivery_binding
        && let Err(error) =
            state.store.bind_session_delivery_owner(&scope.org_id, &reserved.id, &principal, &membership).await
    {
        let _ = state
            .store
            .fail_session(&scope.org_id, &reserved.id, "recording authorization could not be saved", Utc::now())
            .await;
        return Err(ApiFailure::from(error));
    }
    let provider = match state
        .browser
        .start_browser(StartBrowser {
            profile_id: provider_profile_id,
            proxy_country_code,
            timeout_minutes: request.ttl.minutes(),
            enable_recording: true,
            reconciliation_id: reserved.id.clone(),
        })
        .await
    {
        Ok(provider) => provider,
        Err(error) => {
            let ambiguous = provider_mutation_may_have_committed(&error);
            if ambiguous {
                match state.browser.find_browser_by_session_id(&reserved.id).await {
                    Ok(Some(provider)) => {
                        let runtime = provider_session(&provider)?;
                        let session = state
                            .store
                            .activate_session(&scope.org_id, &reserved.id, &runtime, &state.secrets, Utc::now())
                            .await
                            .map_err(ApiFailure::from)?;
                        return Ok(success(session));
                    }
                    Ok(None) => {
                        // Metadata search can race an in-flight create or its
                        // visibility. Absence must not release the profile slot.
                        return Err(ApiFailure::from(error));
                    }
                    Err(reconciliation_error) => {
                        // A remote browser may exist, so retain the starting
                        // reservation until TTL to prevent overlapping sessions.
                        tracing::warn!(
                            session_id = %reserved.id,
                            error_kind = provider_error_kind(&reconciliation_error),
                            "ambiguous browser start could not be reconciled; reservation retained until TTL"
                        );
                        return Err(ApiFailure::from(error));
                    }
                }
            }
            let _ = state
                .store
                .fail_session(&scope.org_id, &reserved.id, "upstream browser start failed", Utc::now())
                .await;
            return Err(ApiFailure::from(error));
        }
    };
    let runtime = match provider_session(&provider) {
        Ok(runtime) => runtime,
        Err(error) => {
            recover_failed_session_start(
                &state,
                &scope.org_id,
                &reserved,
                &provider.id,
                None,
                "upstream browser returned an invalid runtime",
            )
            .await;
            return Err(error);
        }
    };
    match state.store.activate_session(&scope.org_id, &reserved.id, &runtime, &state.secrets, Utc::now()).await {
        Ok(session) => Ok(success(session)),
        Err(error) => {
            recover_failed_session_start(
                &state,
                &scope.org_id,
                &reserved,
                &provider.id,
                Some(&runtime),
                "local browser activation failed",
            )
            .await;
            Err(ApiFailure::from(error))
        }
    }
}

async fn end_session(
    State(state): State<AppState>,
    scope: Scope,
    Path(session_id): Path<String>,
    payload: Result<Json<SessionEnd>, JsonRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let request = json_payload(payload)?;
    request.validate().map_err(ApiFailure::validation)?;
    let gate = state.session_locks.for_session(&scope.org_id, &session_id).await;
    let _guard = gate.lock.lock().await;
    let session = state.store.session(&scope.org_id, &scope.identity, &session_id).await.map_err(ApiFailure::from)?;
    let runtime = state
        .store
        .begin_end_session(&scope.org_id, &scope.identity, &session_id, &request, &state.secrets)
        .await
        .map_err(ApiFailure::from)?;
    let provider =
        stop_browser_confirmed(state.browser.as_ref(), &runtime.provider_session_id).await.map_err(ApiFailure::from)?;

    let now = Utc::now();
    let has_recording_source = if let Some(url) = provider.recording_url.as_deref() {
        state
            .store
            .remember_provider_recording_url(&scope.org_id, &session_id, url, &state.secrets)
            .await
            .map_err(ApiFailure::from)?;
        true
    } else {
        runtime.recording_url.is_some()
    };
    record_provider_usage(
        &state.store,
        UsageSubject {
            org_id: &scope.org_id,
            session_id: &session_id,
            incognito: session.incognito,
            started_at: session.started_at,
        },
        &provider,
        now,
        UsageObservation::Terminal,
    )
    .await
    .map_err(ApiFailure::from)?;
    let duration = browser_duration_ms(&provider, session.started_at, now);
    if has_recording_source {
        state.store.mark_recording_pending(&scope.org_id, &session_id, duration, 0).await
    } else {
        state.store.queue_recording_source_resolution(&scope.org_id, &session_id, duration, now).await
    }
    .map_err(ApiFailure::from)?;
    let ended = state.store.finalize_end_session(&scope.org_id, &session_id, now).await.map_err(ApiFailure::from)?;
    Ok(success(ended))
}

async fn live_session(
    State(state): State<AppState>,
    scope: Scope,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiFailure> {
    let session = state.store.session(&scope.org_id, &scope.identity, &session_id).await.map_err(ApiFailure::from)?;
    if session.status != SessionStatus::Active {
        return Err(ApiFailure::conflict("session_not_active", "the session is not active"));
    }
    let grant = state
        .secrets
        .seal_for(
            &live_grant_context(&scope.org_id, &session_id),
            &serde_json::to_string(&LiveGrant {
                version: 1,
                org_id: &scope.org_id,
                session_id: &session.id,
                expires_at: session.expires_at,
            })
            .map_err(|_| {
                ApiFailure::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "could not create live grant")
            })?,
        )
        .map_err(|_| ApiFailure::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "could not create live grant"))?;
    Ok(success(silicon_browser_shared::LiveLink {
        session_id: session.id,
        // Fragments are never sent in HTTP requests. The authenticated web UI
        // can explicitly redeem this TTL-bounded grant later without putting a
        // provider URL or IAM bearer in browser history and access logs.
        url: format!("{}/sessions/{session_id}/live#grant={grant}", state.public_origin),
        expires_at: session.expires_at,
    }))
}

async fn redeem_live_session(
    State(state): State<AppState>,
    scope: Scope,
    Path(session_id): Path<String>,
    payload: Result<Json<LiveRedeemRequest>, JsonRejection>,
) -> Result<Response, ApiFailure> {
    let request = json_payload(payload)?;
    request.validate().map_err(ApiFailure::validation)?;
    let plaintext = state
        .secrets
        .open_for(&live_grant_context(&scope.org_id, &session_id), &request.grant)
        .map_err(|_| ApiFailure::bad_request("invalid_live_grant", "live grant is invalid or expired"))?;
    let grant: LiveGrantOwned = serde_json::from_str(&plaintext)
        .map_err(|_| ApiFailure::bad_request("invalid_live_grant", "live grant is invalid or expired"))?;
    let now = Utc::now();
    if grant.version != 1 || grant.org_id != scope.org_id || grant.session_id != session_id || grant.expires_at <= now {
        return Err(ApiFailure::bad_request("invalid_live_grant", "live grant is invalid or expired"));
    }
    state
        .store
        .associate_participant(
            &scope.org_id,
            &session_id,
            &scope.identity.id,
            crate::store::ParticipantRole::Viewer,
            now,
        )
        .await
        .map_err(ApiFailure::from)?;
    let runtime = state
        .store
        .provider_runtime(&scope.org_id, &scope.identity, &session_id, &state.secrets)
        .await
        .map_err(ApiFailure::from)?;
    let mut response =
        success(silicon_browser_shared::LiveLink { session_id, url: runtime.live_url, expires_at: grant.expires_at })
            .into_response();
    response.headers_mut().insert("cache-control", HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn session_logs(
    State(state): State<AppState>,
    scope: Scope,
    Path(session_id): Path<String>,
    query: Result<Query<LogsQuery>, QueryRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let query = query_payload(query)?;
    let date = query
        .date
        .as_deref()
        .map(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d"))
        .transpose()
        .map_err(|_| ApiFailure::field("date", "expected YYYY-MM-DD"))?
        .or_else(|| Some(Utc::now().date_naive()));
    Ok(success(
        state
            .store
            .session_logs(&scope.org_id, &scope.identity, &session_id, date, &state.secrets)
            .await
            .map_err(ApiFailure::from)?,
    ))
}

async fn session_connection(
    State(state): State<AppState>,
    scope: Scope,
    Path(session_id): Path<String>,
) -> Result<Response, ApiFailure> {
    let (runtime, expires_at) =
        state.store.connection_runtime(&scope.org_id, &scope.identity, &session_id, &state.secrets, Utc::now()).await?;
    validate_provider_cdp_url(&runtime.cdp_url)?;
    let mut response = success(SessionConnection {
        session_id,
        principal_id: scope.principal.principal_id.to_string(),
        cdp_url: runtime.cdp_url,
        expires_at,
    })
    .into_response();
    response.headers_mut().insert("cache-control", HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn report_command(
    State(state): State<AppState>,
    scope: Scope,
    Path(session_id): Path<String>,
    payload: Result<Json<CommandReport>, JsonRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let report = json_payload(payload)?;
    report.validate().map_err(ApiFailure::validation)?;
    let receipt = state
        .store
        .report_command(
            &scope.org_id,
            &scope.identity,
            &scope.principal.principal_id.to_string(),
            &session_id,
            &report,
            &state.secrets,
            Utc::now(),
        )
        .await?;
    Ok(success(receipt))
}

async fn list_recordings(
    State(state): State<AppState>,
    scope: Scope,
    query: Result<Query<FilterQuery>, QueryRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let query = query_payload(query)?;
    let filter = parse_recording_filter(query.filter.as_deref())?;
    Ok(success(
        state
            .store
            .recordings(&scope.org_id, &scope.identity, filter.as_ref(), &state.secrets)
            .await
            .map_err(ApiFailure::from)?,
    ))
}

async fn get_recording(
    State(state): State<AppState>,
    scope: Scope,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiFailure> {
    Ok(success(
        state
            .store
            .recording(&scope.org_id, &scope.identity, &session_id, &state.secrets)
            .await
            .map_err(ApiFailure::from)?,
    ))
}

async fn trash_recording(
    State(state): State<AppState>,
    scope: Scope,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiFailure> {
    // Hide this recording in Browser. Briefcase owns retention and has no OBO deletion.
    Ok(success(
        state
            .store
            .trash_recording(&scope.org_id, &scope.identity, &session_id, Utc::now(), &state.secrets)
            .await
            .map_err(ApiFailure::from)?,
    ))
}

async fn get_usage(
    State(state): State<AppState>,
    scope: Scope,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiFailure> {
    let session = state.store.session(&scope.org_id, &scope.identity, &session_id).await.map_err(ApiFailure::from)?;
    refresh_active_usage(&state, &scope, &session).await;
    Ok(success(state.store.usage(&scope.org_id, &scope.identity, &session_id).await.map_err(ApiFailure::from)?))
}

async fn list_usage(
    State(state): State<AppState>,
    scope: Scope,
    query: Result<Query<FilterQuery>, QueryRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let query = query_payload(query)?;
    let filter = parse_usage_filter(query.filter.as_deref())?;
    Ok(success(
        state.store.usage_list(&scope.org_id, &scope.identity, filter.as_ref()).await.map_err(ApiFailure::from)?,
    ))
}

async fn org_usage(
    State(state): State<AppState>,
    scope: Scope,
    query: Result<Query<FilterQuery>, QueryRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let query = query_payload(query)?;
    let filter = parse_usage_filter(query.filter.as_deref())?;
    // IAM's application-token introspection proves current membership in
    // this exact organization, but exposes no billing/admin capability. Until
    // IAM supplies one, membership authorizes only this non-dimensionalized
    // aggregate: it contains no session or principal identifiers. In
    // particular, allow only date windows so this endpoint cannot become an
    // oracle for usage hidden behind profile/session ACLs. Keep this allowlist
    // fail-closed if UsageFilter gains more dimensions later.
    if filter.as_ref().is_some_and(|filter| {
        filter.predicates.iter().any(|predicate| !matches!(predicate, UsagePredicate::Between { .. }))
    }) {
        return Err(ApiFailure::bad_request(
            "invalid_filter",
            "organization usage supports only the between date window",
        ));
    }
    let mut response =
        success(state.store.org_usage_total(&scope.org_id, filter.as_ref()).await.map_err(ApiFailure::from)?)
            .into_response();
    response.headers_mut().insert("x-sb-usage-scope", HeaderValue::from_static("organization"));
    Ok(response)
}

async fn search(
    State(state): State<AppState>,
    scope: Scope,
    payload: Result<Json<SearchRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let request = json_payload(payload)?;
    request.validate().map_err(ApiFailure::validation)?;
    let pool = state.search.as_ref().ok_or_else(ApiFailure::discovery_unavailable)?;
    let result: SearchResponse =
        pool.search_for(&actor_key(&scope), request.clone()).await.map_err(ApiFailure::from)?;
    state
        .store
        .record_discovery(
            &scope.org_id,
            &scope.identity.id,
            "search",
            &request.purpose,
            result.results.len(),
            Utc::now(),
        )
        .await
        .map_err(ApiFailure::from)?;
    Ok(success(result))
}

async fn fetch(
    State(state): State<AppState>,
    scope: Scope,
    payload: Result<Json<FetchRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let request = json_payload(payload)?;
    request.validate().map_err(ApiFailure::validation)?;
    let pool = state.search.as_ref().ok_or_else(ApiFailure::discovery_unavailable)?;
    let result: FetchResponse = pool.fetch_for(&actor_key(&scope), request.clone()).await.map_err(ApiFailure::from)?;
    state
        .store
        .record_discovery(&scope.org_id, &scope.identity.id, "fetch", &request.purpose, result.items.len(), Utc::now())
        .await
        .map_err(ApiFailure::from)?;
    Ok(success(result))
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FilterQuery {
    filter: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogsQuery {
    date: Option<String>,
}

#[derive(Serialize)]
struct LiveGrant<'a> {
    version: u8,
    org_id: &'a str,
    session_id: &'a str,
    expires_at: DateTime<Utc>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveGrantOwned {
    version: u8,
    org_id: String,
    session_id: String,
    expires_at: DateTime<Utc>,
}

fn live_grant_context(org_id: &str, session_id: &str) -> String {
    format!("silicon-browser:v1:live-grant:org[{}]:{org_id}:session[{}]:{session_id}", org_id.len(), session_id.len())
}

fn success<T: Serialize>(data: T) -> Json<Envelope<T>> {
    Json(Envelope::new(data))
}

fn json_payload<T>(payload: Result<Json<T>, JsonRejection>) -> Result<T, ApiFailure> {
    payload.map(|Json(value)| value).map_err(|error| ApiFailure::bad_request("invalid_json", error.body_text()))
}

fn query_payload<T>(query: Result<Query<T>, QueryRejection>) -> Result<T, ApiFailure> {
    query.map(|Query(value)| value).map_err(|_| ApiFailure::bad_request("invalid_query", "query string is invalid"))
}

fn bearer(headers: &HeaderMap) -> Result<String, ApiFailure> {
    let value = headers
        .get(AUTHORIZATION)
        .ok_or_else(ApiFailure::unauthenticated)?
        .to_str()
        .map_err(|_| ApiFailure::unauthenticated())?;
    let token = value.strip_prefix("Bearer ").ok_or_else(ApiFailure::unauthenticated)?;
    if token.is_empty() || token.len() > 16 * 1024 || token.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(ApiFailure::unauthenticated());
    }
    Ok(token.to_owned())
}

fn required_header(headers: &HeaderMap, name: &'static str, message: &'static str) -> Result<String, ApiFailure> {
    let value = headers
        .get(name)
        .ok_or_else(|| ApiFailure::bad_request("missing_org", message))?
        .to_str()
        .map_err(|_| ApiFailure::bad_request("invalid_org", message))?;
    if value.is_empty()
        || value.len() > 255
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace() || matches!(character, '/' | '\\'))
    {
        return Err(ApiFailure::bad_request("invalid_org", message));
    }
    Ok(value.to_owned())
}

fn stable_idempotency(operation: &str, credential: &str, org_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"silicon-browser-auth-v1\0");
    digest.update(operation.as_bytes());
    digest.update(b"\0");
    digest.update(org_id.as_bytes());
    digest.update(b"\0");
    digest.update(credential.as_bytes());
    hex::encode(digest.finalize())
}

fn profile_fingerprint(profile_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"silicon-browser-profile-fingerprint-v1\0");
    digest.update(profile_id.as_bytes());
    format!("sbf_{}", hex::encode(&digest.finalize()[..16]))
}

fn public_org(org: OrganizationAccess) -> Org {
    let name = org.name.unwrap_or_else(|| org.id.clone());
    Org { id: org.id, name }
}

fn available_services(state: &AppState) -> Vec<String> {
    let mut services =
        ["profile", "proxy", "session", "recording", "usage"].into_iter().map(str::to_owned).collect::<Vec<_>>();
    if state.search.is_some() {
        services.extend(["search".into(), "fetch".into()]);
    }
    if state.recording_delivery.is_some() {
        services.push("recording_delivery".into());
    }
    services
}

fn provider_session(provider: &ProviderBrowserSession) -> Result<ProviderSession, ApiFailure> {
    validate_provider_browser_session(provider, true).map_err(ApiFailure::from)?;
    let cdp_url = provider
        .cdp_url
        .clone()
        .ok_or_else(|| ApiFailure::bad_gateway("browser_contract", "browser provider returned no CDP endpoint"))?;
    let live_url = provider
        .live_url
        .clone()
        .ok_or_else(|| ApiFailure::bad_gateway("browser_contract", "browser provider returned no live endpoint"))?;
    Ok(ProviderSession { id: provider.id.clone(), cdp_url, live_url, recording_url: provider.recording_url.clone() })
}

fn validate_provider_id(id: &str) -> Result<(), ProviderError> {
    if id.is_empty()
        || id.len() > 255
        || !id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(provider_contract("provider returned an invalid resource id"));
    }
    Ok(())
}

fn validate_provider_profile_identity(profile: &ProviderProfile, expected_user_id: &str) -> Result<(), ProviderError> {
    validate_provider_id(&profile.id)?;
    if profile.user_id.as_deref() != Some(expected_user_id) {
        return Err(provider_contract("provider profile userId did not match its reservation"));
    }
    Ok(())
}

fn validate_provider_browser_session(
    provider: &ProviderBrowserSession,
    require_runtime_urls: bool,
) -> Result<(), ProviderError> {
    validate_provider_id(&provider.id)?;
    match provider.status.as_str() {
        "active" if provider.finished_at.is_some() => {
            return Err(provider_contract("browser provider returned an inconsistent active browser"));
        }
        "active" | "stopped" => {}
        _ => return Err(provider_contract("browser provider returned an invalid browser status")),
    }
    if require_runtime_urls && provider.status != "active" {
        return Err(provider_contract("browser provider did not return an active browser"));
    }
    match provider.cdp_url.as_deref() {
        Some(url) => validate_provider_cdp_url(url)?,
        None if require_runtime_urls => return Err(provider_contract("provider returned no CDP endpoint")),
        None => {}
    }
    match provider.live_url.as_deref() {
        Some(url) => validate_provider_url(url, &["https"], "live")?,
        None if require_runtime_urls => return Err(provider_contract("provider returned no live endpoint")),
        None => {}
    }
    if let Some(url) = provider.recording_url.as_deref() {
        validate_provider_url(url, &["https"], "recording")?;
    }
    Ok(())
}

// Browser providers can return either a direct WebSocket or an HTTPS CDP
// discovery endpoint. Creation and later capability issuance must agree.
fn validate_provider_cdp_url(value: &str) -> Result<(), ProviderError> {
    validate_provider_url(value, &["wss", "https"], "CDP")
}

fn validate_provider_url(value: &str, schemes: &[&str], kind: &'static str) -> Result<(), ProviderError> {
    const MAX_PROVIDER_URL_BYTES: usize = 16 * 1024;
    let parsed = if value.len() <= MAX_PROVIDER_URL_BYTES { Url::parse(value).ok() } else { None };
    if parsed.as_ref().is_none_or(|url| {
        !schemes.contains(&url.scheme())
            || url.cannot_be_a_base()
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || has_forbidden_host(url)
    }) {
        return Err(provider_contract(match kind {
            "CDP" => "provider returned an invalid CDP endpoint",
            "live" => "provider returned an invalid live endpoint",
            "recording" => "provider returned an invalid recording endpoint",
            _ => "provider returned an invalid endpoint",
        }));
    }
    Ok(())
}

fn provider_contract(message: &'static str) -> ProviderError {
    ProviderError::InvalidResponse { provider: "browser-use", message: message.into() }
}

async fn record_provider_usage(
    store: &Store,
    subject: UsageSubject<'_>,
    provider: &ProviderBrowserSession,
    now: DateTime<Utc>,
    observation: UsageObservation,
) -> Result<(), StoreError> {
    let browser_millis = browser_duration_ms(provider, subject.started_at, now);
    let proxy_bytes = decimal_megabytes_to_bytes(&provider.proxy_used_mb)
        .ok_or_else(|| StoreError::Invalid("provider proxy usage is not a valid non-negative decimal".into()))?;
    if subject.incognito && proxy_bytes != 0 {
        tracing::warn!(session_id = %subject.session_id, proxy_bytes,
            "provider reported proxy traffic despite an incognito session requesting proxy disabled");
    }
    let sample = UsageSample {
        browser_millis,
        proxy_bytes_in: None,
        proxy_bytes_out: None,
        // Browser Use exposes a combined proxy counter, so retain it in the
        // explicit unclassified bucket rather than inventing a direction.
        proxy_bytes_unclassified: Some(proxy_bytes),
        browser_cost: provider.browser_cost.clone(),
        proxy_cost: provider.proxy_cost.clone(),
        currency: "USD".into(),
        sampled_at: now,
    };
    match observation {
        UsageObservation::Active => store.record_usage(subject.org_id, subject.session_id, &sample).await?,
        UsageObservation::Terminal => store.record_terminal_usage(subject.org_id, subject.session_id, &sample).await?,
    };
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct UsageSubject<'a> {
    org_id: &'a str,
    session_id: &'a str,
    incognito: bool,
    started_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UsageObservation {
    Active,
    Terminal,
}

/// Resolve the compensating side of a provider start that succeeded after the
/// local saga could no longer activate it. A local terminal transition is
/// attempted only after the provider stop is authoritatively confirmed.
async fn recover_failed_session_start(
    state: &AppState,
    org_id: &str,
    reserved: &Session,
    provider_session_id: &str,
    runtime: Option<&ProviderSession>,
    reason: &str,
) {
    let retained_runtime_source = if let Some(runtime) = runtime {
        match state.store.retain_session_runtime(org_id, &reserved.id, runtime, reason, &state.secrets).await {
            Ok(()) => runtime.recording_url.is_some(),
            Err(error) => {
                // Even if this persistence attempt fails, keep the original
                // starting row. It continues to hold the profile slot unless a
                // stop below is confirmed and all final local writes succeed.
                tracing::error!(
                    session_id = %reserved.id,
                    error = %error,
                    "could not retain runtime after activation failure"
                );
                false
            }
        }
    } else {
        false
    };

    let stopped = match stop_browser_confirmed(state.browser.as_ref(), provider_session_id).await {
        Ok(stopped) => stopped,
        Err(error) => {
            tracing::warn!(
                session_id = %reserved.id,
                error_kind = provider_error_kind(&error),
                "browser start compensation was not confirmed; profile slot remains held"
            );
            return;
        }
    };
    let now = Utc::now();
    let persisted = async {
        let mut has_recording_source = retained_runtime_source;
        if let Some(url) = stopped.recording_url.as_deref() {
            state.store.remember_provider_recording_url(org_id, &reserved.id, url, &state.secrets).await?;
            has_recording_source = true;
        }
        record_provider_usage(
            &state.store,
            UsageSubject {
                org_id,
                session_id: &reserved.id,
                incognito: reserved.incognito,
                started_at: reserved.started_at,
            },
            &stopped,
            now,
            UsageObservation::Terminal,
        )
        .await?;
        let duration = browser_duration_ms(&stopped, reserved.started_at, now);
        state
            .store
            .finalize_failed_session_after_stop(
                org_id,
                &reserved.id,
                FailedSessionFinalization {
                    provider_session_id,
                    reason,
                    duration_ms: duration,
                    has_recording_source,
                    at: now,
                },
            )
            .await?;
        Ok::<_, StoreError>(())
    }
    .await;
    if let Err(error) = persisted {
        tracing::error!(
            session_id = %reserved.id,
            error = %error,
            "provider stopped but failed-start finalization was not persisted; slot remains held"
        );
    }
}

/// A stop response can be lost after the provider has committed it. Reconcile
/// only the ambiguous cases and only accept a provider snapshot that proves a
/// terminal browser; an active/unknown browser always remains `ending`.
async fn stop_browser_confirmed(
    browser: &dyn BrowserProvider,
    provider_session_id: &str,
) -> Result<ProviderBrowserSession, ProviderError> {
    validate_provider_id(provider_session_id)?;
    match browser.stop_browser(provider_session_id).await {
        Ok(session) => validated_terminal_provider_session(session, provider_session_id),
        Err(stop_error) => {
            let ambiguous = provider_mutation_may_have_committed(&stop_error)
                || matches!(&stop_error, ProviderError::Http { status: 404 | 409, .. });
            if ambiguous
                && let Ok(session) = browser.get_browser(provider_session_id).await
                && let Ok(session) = validated_terminal_provider_session(session, provider_session_id)
            {
                return Ok(session);
            }
            Err(stop_error)
        }
    }
}

fn provider_mutation_may_have_committed(error: &ProviderError) -> bool {
    // A successful mutation can return malformed JSON or an incompatible schema.
    // Decoding failure does not establish that the provider rolled it back.
    matches!(
        error,
        ProviderError::Transport { .. }
            | ProviderError::InvalidResponse { .. }
            | ProviderError::Http { status: 500..=599, .. }
    )
}

fn validated_terminal_provider_session(
    session: ProviderBrowserSession,
    expected_id: &str,
) -> Result<ProviderBrowserSession, ProviderError> {
    validate_provider_browser_session(&session, false)?;
    if session.id != expected_id {
        return Err(provider_contract("provider stop returned a different browser id"));
    }
    if !provider_browser_is_terminal(&session) {
        return Err(provider_contract("provider stop did not confirm a terminal browser"));
    }
    Ok(session)
}

fn provider_browser_is_terminal(session: &ProviderBrowserSession) -> bool {
    session.status == "stopped"
}

/// Refresh cost-so-far without turning an otherwise useful read into a
/// provider-availability dependency. Final stop/reaper sampling remains the
/// authoritative lifecycle boundary.
async fn refresh_active_usage(state: &AppState, scope: &Scope, session: &Session) -> bool {
    if session.status != SessionStatus::Active {
        return false;
    }
    let sampled = async {
        let runtime = state.store.provider_runtime(&scope.org_id, &scope.identity, &session.id, &state.secrets).await?;
        let provider = state.browser.get_browser(&runtime.provider_session_id).await.map_err(ApiFailure::from)?;
        validate_provider_browser_session(&provider, false).map_err(ApiFailure::from)?;
        if provider.id != runtime.provider_session_id {
            return Err(ApiFailure::from(provider_contract("provider lookup returned a different browser id")));
        }
        record_provider_usage(
            &state.store,
            UsageSubject {
                org_id: &scope.org_id,
                session_id: &session.id,
                incognito: session.incognito,
                started_at: session.started_at,
            },
            &provider,
            Utc::now(),
            UsageObservation::Active,
        )
        .await
        .map_err(ApiFailure::from)?;
        Ok::<_, ApiFailure>(())
    }
    .await;
    match sampled {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(session_id = %session.id, status = %error.status, "could not refresh active session usage");
            false
        }
    }
}

fn browser_duration_ms(
    provider: &ProviderBrowserSession,
    started_at: DateTime<Utc>,
    observed_at: DateTime<Utc>,
) -> u64 {
    // Reconciliation can finish long after the browser stopped. Recording and
    // usage durations must share the provider's actual terminal timestamps.
    provider_duration_ms(provider)
        .unwrap_or_else(|| observed_at.signed_duration_since(started_at).num_milliseconds().max(0) as u64)
}

fn provider_duration_ms(provider: &ProviderBrowserSession) -> Option<u64> {
    let start = DateTime::parse_from_rfc3339(provider.started_at.as_deref()?).ok()?;
    let finish = DateTime::parse_from_rfc3339(provider.finished_at.as_deref()?).ok()?;
    Some(finish.signed_duration_since(start).num_milliseconds().max(0) as u64)
}

fn decimal_megabytes_to_bytes(value: &str) -> Option<u64> {
    decimal_to_millionths(value).ok()
}

fn parse_session_filter(value: Option<&str>) -> Result<Option<SessionFilter>, ApiFailure> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(SessionFilter::parse)
        .transpose()
        .map_err(|error| ApiFailure::bad_request("invalid_filter", error.to_string()))
}

fn parse_recording_filter(value: Option<&str>) -> Result<Option<RecordingFilter>, ApiFailure> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(RecordingFilter::parse)
        .transpose()
        .map_err(|error| ApiFailure::bad_request("invalid_filter", error.to_string()))
}

fn parse_usage_filter(value: Option<&str>) -> Result<Option<UsageFilter>, ApiFailure> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(UsageFilter::parse)
        .transpose()
        .map_err(|error| ApiFailure::bad_request("invalid_filter", error.to_string()))
}

fn reject_profile_filter(value: Option<&str>) -> Result<(), ApiFailure> {
    if value.is_some_and(|value| !value.trim().is_empty()) {
        return Err(ApiFailure::bad_request("invalid_filter", "profile filters are not supported"));
    }
    Ok(())
}

fn actor_key(scope: &Scope) -> String {
    format!("{}:{}", scope.org_id, scope.identity.id)
}

fn default_proxy_locations() -> Vec<ProxyLocation> {
    provider_proxy_locations()
        .iter()
        .map(|location| ProxyLocation {
            code: location.code.into(),
            name: location.name.into(),
            country: Some(location.code.to_ascii_uppercase()),
        })
        .collect()
}

async fn not_found() -> ApiFailure {
    ApiFailure::new(StatusCode::NOT_FOUND, "route_not_found", "API route was not found")
}

async fn method_not_allowed() -> ApiFailure {
    ApiFailure::new(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed", "HTTP method is not allowed")
}

#[derive(Debug)]
pub struct ApiFailure {
    status: StatusCode,
    code: String,
    message: String,
    fields: Vec<FieldError>,
    details: BTreeMap<String, String>,
    retry_after_ms: Option<u64>,
    auth_rejected_before_handler: bool,
}

impl ApiFailure {
    fn new(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
            fields: Vec::new(),
            details: BTreeMap::new(),
            retry_after_ms: None,
            auth_rejected_before_handler: false,
        }
    }

    // Only extractor rejection proves that retrying a mutation cannot repeat
    // handler side effects. Handler/provider 401s must never carry this marker.
    fn before_handler(mut self) -> Self {
        self.auth_rejected_before_handler = self.status == StatusCode::UNAUTHORIZED;
        self
    }

    fn bad_request(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    fn bad_gateway(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, code, message)
    }

    fn conflict(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, message)
    }

    fn field(field: impl Into<String>, message: impl Into<String>) -> Self {
        let message = message.into();
        let mut failure = Self::bad_request("invalid_request", &message);
        failure.fields.push(FieldError { field: field.into(), message });
        failure
    }

    fn validation(error: silicon_browser_shared::ValidationError) -> Self {
        Self::bad_request("invalid_request", error.to_string())
    }

    fn unauthenticated() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthenticated", "authentication is required or expired")
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }

    fn discovery_unavailable() -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "search_unavailable", "search and fetch are not configured")
    }

    fn into_api_error(self) -> ApiError {
        ApiError {
            code: self.code,
            message: self.message,
            fields: self.fields,
            details: self.details,
            request_id: Some(Uuid::now_v7().to_string()),
            retry_after_ms: self.retry_after_ms,
        }
    }
}

impl IntoResponse for ApiFailure {
    fn into_response(self) -> Response {
        let status = self.status;
        let retry_after_ms = self.retry_after_ms;
        let auth_rejected_before_handler = self.auth_rejected_before_handler;
        let error = self.into_api_error();
        let request_id = error.request_id.clone().expect("API failures always carry a request id");
        let mut response = (status, Json(ApiErrorEnvelope { error })).into_response();
        if let Ok(value) = HeaderValue::from_str(&request_id) {
            response.headers_mut().insert("x-request-id", value);
        }
        if auth_rejected_before_handler {
            response.headers_mut().insert("x-sb-auth-rejected", HeaderValue::from_static("1"));
        }
        if let Some(milliseconds) = retry_after_ms {
            let seconds = milliseconds.div_ceil(1_000).max(1);
            if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
                response.headers_mut().insert("retry-after", value);
            }
        }
        response
    }
}

impl From<IdentityError> for ApiFailure {
    fn from(error: IdentityError) -> Self {
        match error {
            IdentityError::Unauthenticated => Self::unauthenticated(),
            IdentityError::Forbidden => Self::forbidden("not authorized for this organization"),
            IdentityError::CapabilityUnavailable(_) => Self::new(
                StatusCode::NOT_IMPLEMENTED,
                "identity_capability_unavailable",
                "IAM cannot provide that identity operation for this token",
            ),
            IdentityError::InvalidInput { field, reason } => Self::field(field, reason),
            IdentityError::Rejected { status, code, .. } => {
                let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
                let public_status = match status {
                    StatusCode::UNAUTHORIZED => StatusCode::UNAUTHORIZED,
                    StatusCode::FORBIDDEN => StatusCode::FORBIDDEN,
                    StatusCode::TOO_MANY_REQUESTS => StatusCode::TOO_MANY_REQUESTS,
                    _ => StatusCode::BAD_GATEWAY,
                };
                Self::new(public_status, "iam_rejected", format!("IAM rejected authentication ({code})"))
            }
            IdentityError::Upstream { kind, retry_after, .. } => {
                let status = if kind == UpstreamFailure::RateLimited {
                    StatusCode::TOO_MANY_REQUESTS
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                };
                let mut failure = Self::new(status, "iam_unavailable", "identity service is temporarily unavailable");
                failure.retry_after_ms =
                    retry_after.map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64);
                failure
            }
            IdentityError::Contract { .. } => {
                Self::new(StatusCode::BAD_GATEWAY, "iam_contract", "identity service returned an invalid response")
            }
        }
    }
}

impl From<StoreError> for ApiFailure {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::ReportConflict { code: "report_window_closed" } => Self::conflict(
                "report_window_closed",
                "command archive has started and cannot include additional reports",
            ),
            StoreError::ReportConflict { code } => Self::conflict(code, "command report conflicts with stored history"),
            StoreError::Invalid(message) => Self::bad_request("invalid_request", message),
            StoreError::NotFound { kind, .. } => {
                Self::new(StatusCode::NOT_FOUND, "not_found", format!("{kind} was not found"))
            }
            StoreError::Forbidden { kind, .. } => Self::forbidden(format!("not allowed to modify {kind}")),
            StoreError::ProfileRetired { .. } => Self::conflict("profile_retired", "profile is retired"),
            StoreError::ProfileBusy { session_id, actor_id, expires_at, .. } => {
                let mut failure = Self::conflict("profile_busy", "profile already has a live session");
                failure.details.insert("session_id".into(), session_id);
                failure.details.insert("actor_id".into(), actor_id);
                failure.details.insert("expires_at".into(), expires_at.to_rfc3339());
                failure
            }
            StoreError::SessionState { status, .. } => {
                let mut failure = Self::conflict("invalid_state", "resource is not in the required state");
                failure.details.insert("status".into(), status);
                failure
            }
            StoreError::Database(_) | StoreError::Migration(_) | StoreError::Corrupt { .. } | StoreError::Crypto(_) => {
                Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "an internal data operation failed")
            }
        }
    }
}

impl From<ProviderError> for ApiFailure {
    fn from(error: ProviderError) -> Self {
        match error {
            ProviderError::InvalidInput(message) => Self::bad_request("invalid_provider_request", message),
            ProviderError::Unsupported { feature, .. } => {
                Self::new(StatusCode::NOT_IMPLEMENTED, "provider_unsupported", format!("{feature} is unavailable"))
            }
            ProviderError::Http { status: 429, .. } => {
                Self::new(StatusCode::TOO_MANY_REQUESTS, "provider_rate_limited", "provider rate limit reached")
            }
            ProviderError::Http { .. } | ProviderError::InvalidResponse { .. } => {
                Self::bad_gateway("provider_failure", "provider returned an invalid response")
            }
            ProviderError::Transport { .. } => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "provider_unavailable",
                "provider is temporarily unavailable",
            ),
            ProviderError::Overloaded { capacity, .. } => {
                let mut failure = Self::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "provider_queue_full",
                    "provider scheduling queue is full",
                );
                failure.details.insert("capacity".into(), capacity.to_string());
                failure.retry_after_ms = Some(1_000);
                failure
            }
        }
    }
}

fn provider_error_kind(error: &ProviderError) -> &'static str {
    match error {
        ProviderError::InvalidInput(_) => "invalid_input",
        ProviderError::Unsupported { .. } => "unsupported",
        ProviderError::Transport { .. } => "transport",
        ProviderError::Http { .. } => "http",
        ProviderError::InvalidResponse { .. } => "invalid_response",
        ProviderError::Overloaded { .. } => "overloaded",
    }
}

fn recording_reconciliation_backoff(attempt: u32) -> Duration {
    let multiplier = 1_u64 << attempt.saturating_sub(1).min(5);
    Duration::from_secs(
        RECORDING_SOURCE_INITIAL_DELAY
            .as_secs()
            .saturating_mul(multiplier)
            .min(RECORDING_RECONCILIATION_MAX_BACKOFF.as_secs()),
    )
}

pub fn spawn_ttl_reaper(state: AppState, interval: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let ttl_state = state.clone();
        let profile_state = state.clone();
        let delivery_state = state.clone();
        let ttl_loop = async move {
            let mut timer = tokio::time::interval(interval);
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                timer.tick().await;
                if let Err(error) = ttl_state.reap_expired_sessions_once().await {
                    tracing::error!(error = %error, "TTL reaper iteration failed");
                }
            }
        };
        let profile_loop = async move {
            let mut timer = tokio::time::interval(interval);
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                timer.tick().await;
                if let Err(error) = profile_state.reconcile_profiles_once().await {
                    tracing::error!(error = %error, "profile reconciliation iteration failed");
                }
            }
        };
        let recording_loop = async move {
            let mut timer = tokio::time::interval(interval);
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                timer.tick().await;
                if let Err(error) = state.reconcile_recording_sources_once(Utc::now()).await {
                    tracing::error!(error = %error, "recording reconciliation iteration failed");
                }
            }
        };
        let delivery_loop = async move {
            let mut timer = tokio::time::interval(interval);
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                timer.tick().await;
                if let Err(error) = delivery_state.deliver_recordings_once().await {
                    tracing::error!(error = %error, "recording delivery iteration failed");
                }
            }
        };
        tokio::join!(ttl_loop, profile_loop, recording_loop, delivery_loop);
    })
}

#[cfg(test)]
mod tests {
    mod local_control {
        include!("server/local_control_tests.rs");
    }
    mod delivery_integration {
        include!("server/delivery_tests.rs");
    }
    mod usage_limits_contract {
        include!("server/usage_limits_tests.rs");
    }
    use std::collections::HashSet;
    use std::sync::Mutex as StdMutex;

    use async_trait::async_trait;
    use axum::body::to_bytes;
    use axum::http::Request;
    use chrono::TimeDelta;
    use serde_json::{Value, json};
    use silicon_browser_shared::{
        FetchItem, FetchStatus, IdentityKind, Profile, ProfileStatus, Recording, SearchResult, Session, Usage,
    };
    use tower::ServiceExt as _;

    use super::*;
    use crate::auth::FakeIdentityProvider;
    use crate::providers::{ProviderProfile, ProviderResult, SearchProvider};

    #[derive(Default)]
    struct FakeBrowser {
        state: StdMutex<FakeBrowserState>,
    }

    #[derive(Default)]
    struct FakeBrowserState {
        profiles: HashMap<String, ProviderProfile>,
        browsers: HashMap<String, ProviderBrowserSession>,
        stopped: Vec<String>,
        get_calls: usize,
        last_reconciliation_id: Option<String>,
        ambiguous_profile_create: bool,
        malformed_mutation_response: bool,
        next_profile_id: Option<String>,
        next_profile_user_id: Option<Option<String>>,
        next_update_profile_id: Option<String>,
        next_update_profile_user_id: Option<Option<String>>,
        ambiguous_browser_start: bool,
        empty_browser_reconciliation: bool,
        ambiguous_browser_stop: bool,
        unconfirmed_browser_stop: bool,
        terminal_decimals: Option<(String, String, String)>,
        hang_profile_reconciliation: bool,
        hang_recording_reconciliation: bool,
        hanging_browser_stops: HashSet<String>,
        finalize_as_ended_on_stop: Option<(String, String, Store)>,
        /// Number of terminal GET snapshots which should still omit the
        /// recording URL after stop, keyed by provider browser id.
        recording_missing_gets: HashMap<String, u32>,
    }

    impl FakeBrowser {
        fn stopped(&self) -> Vec<String> {
            self.state.lock().unwrap().stopped.clone()
        }
    }

    #[async_trait]
    impl BrowserProvider for FakeBrowser {
        async fn create_profile(&self, request: CreateBrowserProfile) -> ProviderResult<ProviderProfile> {
            let mut state = self.state.lock().unwrap();
            let user_id = state.next_profile_user_id.take().unwrap_or_else(|| Some(request.user_id.clone()));
            let profile = ProviderProfile {
                id: state
                    .next_profile_id
                    .take()
                    .unwrap_or_else(|| format!("provider-profile-{}", state.profiles.len() + 1)),
                created_at: None,
                updated_at: None,
                user_id,
                name: Some(request.name),
                last_used_at: None,
                cookie_domains: Vec::new(),
            };
            state.profiles.insert(request.user_id, profile.clone());
            if state.ambiguous_profile_create {
                if state.malformed_mutation_response {
                    return Err(provider_contract("malformed committed profile response"));
                }
                return Err(ProviderError::Http {
                    provider: "fake",
                    status: 503,
                    message: "redacted".into(),
                    retry_after: None,
                });
            }
            Ok(profile)
        }

        async fn update_profile(&self, id: &str, request: UpdateBrowserProfile) -> ProviderResult<ProviderProfile> {
            let mut state = self.state.lock().unwrap();
            let response_id = state.next_update_profile_id.take();
            let response_user_id = state.next_update_profile_user_id.take();
            let mut response = {
                let profile =
                    state.profiles.values_mut().find(|profile| profile.id == id).ok_or_else(|| {
                        ProviderError::InvalidResponse { provider: "fake", message: "missing".into() }
                    })?;
                if let Some(name) = request.name {
                    profile.name = Some(name);
                }
                profile.clone()
            };
            if let Some(id) = response_id {
                response.id = id;
            }
            if let Some(user_id) = response_user_id {
                response.user_id = user_id;
            }
            Ok(response)
        }

        async fn start_browser(&self, request: StartBrowser) -> ProviderResult<ProviderBrowserSession> {
            let mut state = self.state.lock().unwrap();
            let id = format!("provider-browser-{}", state.browsers.len() + 1);
            let provider = fake_provider_browser(&id, false);
            state.last_reconciliation_id = Some(request.reconciliation_id);
            state.browsers.insert(id, provider.clone());
            if state.ambiguous_browser_start {
                if state.malformed_mutation_response {
                    return Err(provider_contract("malformed committed browser response"));
                }
                return Err(ProviderError::Transport { provider: "fake", message: "ambiguous".into() });
            }
            Ok(provider)
        }

        async fn get_browser(&self, id: &str) -> ProviderResult<ProviderBrowserSession> {
            let hang = {
                let mut state = self.state.lock().unwrap();
                state.get_calls += 1;
                state.hang_recording_reconciliation
            };
            if hang {
                return std::future::pending().await;
            }
            let mut state = self.state.lock().unwrap();
            let materialize = match state.recording_missing_gets.get_mut(id) {
                Some(remaining) if *remaining != 0 => {
                    *remaining -= 1;
                    false
                }
                Some(_) => {
                    state.recording_missing_gets.remove(id);
                    true
                }
                None => false,
            };
            let browser = state
                .browsers
                .get_mut(id)
                .ok_or_else(|| ProviderError::InvalidResponse { provider: "fake", message: "missing".into() })?;
            if materialize && browser.finished_at.is_some() {
                browser.recording_url = Some(format!("https://provider.invalid/recording/{id}?secret=yes"));
            }
            Ok(browser.clone())
        }

        async fn stop_browser(&self, id: &str) -> ProviderResult<ProviderBrowserSession> {
            let (hang, unconfirmed, ambiguous, finalize_as_ended, terminal_decimals) = {
                let mut state = self.state.lock().unwrap();
                state.stopped.push(id.into());
                let hang = state.hanging_browser_stops.contains(id);
                let unconfirmed = state.unconfirmed_browser_stop;
                let ambiguous = state.ambiguous_browser_stop;
                if ambiguous {
                    state.ambiguous_browser_stop = false;
                }
                let finalize_as_ended =
                    state.finalize_as_ended_on_stop.as_ref().filter(|(provider_id, _, _)| provider_id == id).cloned();
                (hang, unconfirmed, ambiguous, finalize_as_ended, state.terminal_decimals.clone())
            };
            if hang {
                return std::future::pending().await;
            }
            if unconfirmed {
                return Err(ProviderError::Transport { provider: "fake", message: "ambiguous".into() });
            }
            let mut stopped = fake_provider_browser(id, true);
            if let Some((proxy_used_mb, proxy_cost, browser_cost)) = terminal_decimals {
                stopped.proxy_used_mb = proxy_used_mb;
                stopped.proxy_cost = proxy_cost;
                stopped.browser_cost = browser_cost;
            }
            if self.state.lock().unwrap().recording_missing_gets.contains_key(id) {
                stopped.recording_url = None;
            }
            self.state.lock().unwrap().browsers.insert(id.into(), stopped.clone());
            if let Some((_, session_id, store)) = finalize_as_ended {
                store.finalize_end_session("org-1", &session_id, Utc::now()).await.unwrap();
            }
            if ambiguous {
                if self.state.lock().unwrap().malformed_mutation_response {
                    return Err(provider_contract("malformed committed stop response"));
                }
                return Err(ProviderError::Http {
                    provider: "fake",
                    status: 503,
                    message: "redacted".into(),
                    retry_after: None,
                });
            }
            Ok(stopped)
        }

        async fn find_profile_by_user_id(&self, user_id: &str) -> ProviderResult<Option<ProviderProfile>> {
            let hang = {
                let state = self.state.lock().unwrap();
                state.hang_profile_reconciliation
            };
            if hang {
                return std::future::pending().await;
            }
            Ok(self.state.lock().unwrap().profiles.get(user_id).cloned())
        }

        async fn find_browser_by_session_id(
            &self,
            _session_id: &str,
        ) -> ProviderResult<Option<ProviderBrowserSession>> {
            if self.state.lock().unwrap().empty_browser_reconciliation {
                return Ok(None);
            }
            Err(ProviderError::Unsupported { provider: "fake", feature: "browser creation reconciliation" })
        }
    }

    fn fake_provider_browser(id: &str, stopped: bool) -> ProviderBrowserSession {
        ProviderBrowserSession {
            id: id.into(),
            status: if stopped { "stopped" } else { "active" }.into(),
            timeout_at: None,
            started_at: Some("2026-09-04T10:00:00Z".into()),
            live_url: Some(format!("https://provider.invalid/live/{id}?secret=yes")),
            cdp_url: Some(format!("wss://provider.invalid/cdp/{id}?secret=yes")),
            finished_at: stopped.then(|| "2026-09-04T10:01:30Z".into()),
            proxy_used_mb: if stopped { "1.25".into() } else { "0".into() },
            proxy_cost: if stopped { "0.5".into() } else { "0".into() },
            browser_cost: if stopped { "0.25".into() } else { "0".into() },
            agent_session_id: None,
            recording_url: stopped.then(|| format!("https://provider.invalid/recording/{id}?secret=yes")),
            recording_available: None,
            metadata: Default::default(),
        }
    }

    struct FakeSearch;

    #[async_trait]
    impl SearchProvider for FakeSearch {
        async fn search(&self, request: SearchRequest) -> ProviderResult<SearchResponse> {
            Ok(SearchResponse {
                results: vec![SearchResult {
                    rank: 1,
                    title: request.query,
                    url: "https://example.test".into(),
                    snippet: None,
                    published_at: None,
                }],
                page: request.page,
                queued_ms: 0,
            })
        }

        async fn fetch(&self, request: FetchRequest) -> ProviderResult<FetchResponse> {
            Ok(FetchResponse {
                items: request
                    .urls
                    .into_iter()
                    .map(|url| FetchItem {
                        url,
                        status: FetchStatus::Ok,
                        content: Some("page".into()),
                        links: Vec::new(),
                        image_links: Vec::new(),
                        error: None,
                        cached: false,
                    })
                    .collect(),
                queued_ms: 0,
            })
        }
    }

    struct Fixture {
        app: Router,
        identity: FakeIdentityProvider,
        browser: Arc<FakeBrowser>,
        store: Store,
        state: AppState,
    }

    async fn fixture() -> Fixture {
        let store = Store::in_memory().await.unwrap();
        let identity = FakeIdentityProvider::new();
        identity.allow_identity("oat_owner", principal("owner-1", IdentityKind::Silicon, true));
        identity.allow_identity("oat_viewer", principal("viewer-1", IdentityKind::Carbon, true));
        identity.allow_orgs("oat_owner", vec![OrganizationAccess { id: "org-1".into(), name: Some("The Org".into()) }]);
        let browser = Arc::new(FakeBrowser::default());
        let search_provider: Arc<dyn SearchProvider> = Arc::new(FakeSearch);
        let search = Arc::new(FairSearchPool::new(vec![search_provider]).unwrap());
        let state = AppState::new(
            "https://browser.example",
            store.clone(),
            SecretBox::new(&[7; 32]),
            Arc::new(identity.clone()),
            browser.clone(),
            Some(search),
        )
        .unwrap();
        Fixture { app: router(state.clone()), identity, browser, store, state }
    }

    fn owner() -> Identity {
        Identity {
            id: "owner-1".into(),
            name: "owner-1".into(),
            kind: IdentityKind::Silicon,
            tags: Vec::new(),
            verified_aliases: Vec::new(),
        }
    }

    async fn seed_expired_incognito(fixture: &Fixture, suffix: &str) -> Session {
        let started_at = Utc::now() - TimeDelta::minutes(20);
        let request = SessionCreate::incognito(
            format!("Expired {suffix}"),
            "lifecycle test",
            silicon_browser_shared::SessionTtl::Minutes15,
        );
        let reserved = fixture.store.reserve_session("org-1", &owner(), &request, started_at).await.unwrap();
        let provider = fake_provider_browser(&format!("provider-expired-{suffix}"), false);
        fixture.browser.state.lock().unwrap().browsers.insert(provider.id.clone(), provider.clone());
        fixture
            .store
            .activate_session(
                "org-1",
                &reserved.id,
                &provider_session(&provider).unwrap(),
                &fixture.state.secrets,
                started_at,
            )
            .await
            .unwrap()
    }

    async fn seed_pending_recording_resolution(fixture: &Fixture, suffix: &str, due_at: DateTime<Utc>) -> Session {
        let started_at = due_at - TimeDelta::minutes(1);
        let request = SessionCreate::incognito(
            format!("Await recording {suffix}"),
            "recording reconciliation test",
            silicon_browser_shared::SessionTtl::Minutes15,
        );
        let reserved = fixture.store.reserve_session("org-1", &owner(), &request, started_at).await.unwrap();
        let provider = fake_provider_browser(&format!("provider-recording-{suffix}"), false);
        fixture.browser.state.lock().unwrap().browsers.insert(provider.id.clone(), provider.clone());
        fixture
            .store
            .activate_session(
                "org-1",
                &reserved.id,
                &provider_session(&provider).unwrap(),
                &fixture.state.secrets,
                started_at,
            )
            .await
            .unwrap();
        fixture
            .store
            .begin_end_session(
                "org-1",
                &owner(),
                &reserved.id,
                &SessionEnd { note: "done".into() },
                &fixture.state.secrets,
            )
            .await
            .unwrap();
        fixture.browser.state.lock().unwrap().browsers.insert(
            provider.id.clone(),
            ProviderBrowserSession { recording_url: None, ..fake_provider_browser(&provider.id, true) },
        );
        let queued_at = due_at
            - TimeDelta::from_std(RECORDING_SOURCE_INITIAL_DELAY)
                .expect("the bounded initial recording delay fits chrono::TimeDelta");
        fixture.store.queue_recording_source_resolution("org-1", &reserved.id, 1_000, queued_at).await.unwrap();
        fixture.store.finalize_end_session("org-1", &reserved.id, due_at).await.unwrap()
    }

    fn principal(id: &str, kind: IdentityKind, public: bool) -> PrincipalIdentity {
        PrincipalIdentity {
            principal_id: Uuid::new_v4(),
            public_id: public.then(|| id.into()),
            tags: None,
            kind,
            org_id: "org-1".into(),
            membership_id: Uuid::new_v4(),
            authorization_epoch: 1,
            expires_at: Utc::now() + TimeDelta::hours(1),
        }
    }

    async fn request(
        app: &Router,
        method: &'static str,
        uri: &str,
        auth: Option<(&str, &str)>,
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some((bearer, org)) = auth {
            builder = builder.header(AUTHORIZATION, format!("Bearer {bearer}")).header("x-org-id", org);
        }
        if body.is_some() {
            builder = builder.header(CONTENT_TYPE, "application/json");
        }
        let response = app
            .clone()
            .oneshot(builder.body(Body::from(body.map(|body| body.to_string()).unwrap_or_default())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), 32 * 1024 * 1024).await.unwrap().to_vec();
        (status, headers, body)
    }

    fn data(body: &[u8]) -> Value {
        serde_json::from_slice::<Value>(body).unwrap()["data"].clone()
    }

    #[test]
    fn live_grant_crypto_context_is_bound_to_org_and_session() {
        let original = live_grant_context("org-1", "session-1");
        assert_ne!(original, live_grant_context("org-2", "session-1"));
        assert_ne!(original, live_grant_context("org-1", "session-2"));
    }

    #[tokio::test]
    async fn public_origin_requires_tls_except_for_loopback_development() {
        let state = fixture().await.state;
        let rebuild = |origin: &str| {
            AppState::new(
                origin,
                state.store.clone(),
                state.secrets.clone(),
                state.identity.clone(),
                state.browser.clone(),
                state.search.clone(),
            )
        };
        assert!(rebuild("http://backend.example").is_err());
        assert!(rebuild("http://10.0.0.1:8080").is_err());
        assert!(rebuild("https://backend.example").is_ok());
        assert!(rebuild("https://backend.example/path").is_err());
        assert!(rebuild("http://localhost:8080").is_ok());
        assert!(rebuild("http://127.0.0.1:8080").is_ok());
        assert!(rebuild("http://[::1]:8080").is_ok());
    }

    #[tokio::test]
    async fn production_api_exposes_pre_handler_auth_marker_to_exact_frontend_origin() {
        let app = production_router(fixture().await.state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/me")
                    .header("origin", "https://browser.example")
                    .header(AUTHORIZATION, "Bearer oat_expired")
                    .header("x-org-id", "org-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()["x-sb-auth-rejected"], "1");
        assert_eq!(response.headers()["access-control-allow-origin"], "https://browser.example");
        let exposed = response.headers()["access-control-expose-headers"].to_str().unwrap();
        assert!(exposed.split(',').any(|name| name.trim() == "x-sb-auth-rejected"));
        assert!(!response.headers().contains_key("access-control-allow-credentials"));
    }

    #[tokio::test]
    async fn production_api_preflight_allows_only_configured_origin_methods_and_headers() {
        let app = production_router(fixture().await.state);
        for method in ["POST", "PATCH"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("OPTIONS")
                        .uri("/api/v1/profiles")
                        .header("origin", "https://browser.example")
                        .header("access-control-request-method", method)
                        .header("access-control-request-headers", "authorization,content-type,x-org-id")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!(response.status().is_success());
            assert_eq!(response.headers()["access-control-allow-origin"], "https://browser.example");
            let methods = response.headers()["access-control-allow-methods"].to_str().unwrap();
            assert!(methods.split(',').any(|value| value.trim() == method));
            let headers = response.headers()["access-control-allow-headers"].to_str().unwrap();
            for name in ["authorization", "content-type", "x-org-id"] {
                assert!(headers.split(',').any(|value| value.trim() == name));
            }
            assert!(!headers.contains('*'));
            assert!(!response.headers().contains_key("access-control-allow-credentials"));
        }
        let response = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/api/v1/profiles")
                    .header("origin", "https://untrusted.example")
                    .header("access-control-request-method", "DELETE")
                    .header("access-control-request-headers", "x-untrusted")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(
            response.headers().get("access-control-allow-origin").and_then(|v| v.to_str().ok()),
            Some("https://untrusted.example")
        );
        assert!(!response.headers()["access-control-allow-methods"].to_str().unwrap().contains("DELETE"));
        assert!(!response.headers()["access-control-allow-headers"].to_str().unwrap().contains("x-untrusted"));
    }

    #[tokio::test]
    async fn production_backend_serves_json_api_and_no_frontend_html_or_assets() {
        let app = production_router(fixture().await.state);
        for path in ["/", "/sessions/example/live", "/assets/browser.js", "/assets/browser.css"] {
            let (status, headers, body) = request(&app, "GET", path, None, None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
            assert_eq!(headers[CONTENT_TYPE], "application/json");
            assert!(serde_json::from_slice::<Value>(&body).unwrap()["error"].is_object());
        }
        assert_eq!(request(&app, "GET", "/healthz", None, None).await.0, StatusCode::OK);
        assert_eq!(request(&app, "GET", "/api/v1/me", Some(("oat_owner", "org-1")), None).await.0, StatusCode::OK);
    }

    #[test]
    fn provider_runtime_validation_rejects_unsafe_ids_and_endpoints() {
        let valid = fake_provider_browser("browser-safe", false);
        assert!(provider_session(&valid).is_ok());

        let mut invalid = Vec::new();
        let mut bad_id = valid.clone();
        bad_id.id = "../escape".into();
        invalid.push(bad_id);
        let mut bad_cdp_scheme = valid.clone();
        bad_cdp_scheme.cdp_url = Some("file:///etc/passwd".into());
        invalid.push(bad_cdp_scheme);
        let mut cleartext_cdp = valid.clone();
        cleartext_cdp.cdp_url = Some("ws://provider.invalid/devtools?token=secret".into());
        invalid.push(cleartext_cdp);
        let mut cdp_userinfo = valid.clone();
        cdp_userinfo.cdp_url = Some("wss://user:secret@provider.invalid/devtools".into());
        invalid.push(cdp_userinfo);
        let mut cdp_fragment = valid.clone();
        cdp_fragment.cdp_url = Some("wss://provider.invalid/devtools#secret".into());
        invalid.push(cdp_fragment);
        let mut insecure_live = valid.clone();
        insecure_live.live_url = Some("http://provider.invalid/live".into());
        invalid.push(insecure_live);
        let mut live_userinfo = valid.clone();
        live_userinfo.live_url = Some("https://user:secret@provider.invalid/live".into());
        invalid.push(live_userinfo);
        let mut recording_userinfo = valid;
        recording_userinfo.recording_url = Some("https://user:secret@provider.invalid/recording".into());
        invalid.push(recording_userinfo);

        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.1.1",
            "0.0.0.0",
            "224.0.0.1",
            "[::1]",
            "[fd00::1]",
            "[fe80::1]",
            "[::]",
            "[ff02::1]",
            "[::ffff:127.0.0.1]",
        ] {
            let mut unsafe_cdp = fake_provider_browser("browser-safe", false);
            unsafe_cdp.cdp_url = Some(format!("wss://{address}/devtools?token=signed"));
            invalid.push(unsafe_cdp);

            let mut unsafe_live = fake_provider_browser("browser-safe", false);
            unsafe_live.live_url = Some(format!("https://{address}/live?token=signed"));
            invalid.push(unsafe_live);

            let mut unsafe_recording = fake_provider_browser("browser-safe", false);
            unsafe_recording.recording_url = Some(format!("https://{address}/recording?token=signed"));
            invalid.push(unsafe_recording);
        }

        for response in invalid {
            let failure = provider_session(&response).unwrap_err();
            assert_eq!(failure.status, StatusCode::BAD_GATEWAY);
            assert!(!failure.message.contains("secret"));
        }

        for status in ["starting", "stopped", "failed", "ACTIVE"] {
            let mut inactive = fake_provider_browser("browser-safe", false);
            inactive.status = status.into();
            assert!(provider_session(&inactive).is_err(), "accepted provider status {status}");
        }
        let mut already_finished = fake_provider_browser("browser-safe", false);
        already_finished.finished_at = Some(Utc::now().to_rfc3339());
        assert!(provider_session(&already_finished).is_err());
    }

    #[test]
    fn terminal_provider_validation_requires_the_documented_stopped_status() {
        let stopped = fake_provider_browser("browser-safe", true);
        assert!(validated_terminal_provider_session(stopped.clone(), "browser-safe").is_ok());

        // The Browser Use contract exposes exactly `active | stopped`. A
        // finished timestamp cannot turn an active or invented status into
        // authoritative proof that the remote browser stopped.
        for status in ["active", "finished", "completed", "ended", "terminated", "STOPPED"] {
            let mut undocumented = stopped.clone();
            undocumented.status = status.into();
            assert!(
                validated_terminal_provider_session(undocumented, "browser-safe").is_err(),
                "accepted undocumented terminal status {status}"
            );
        }

        // Status is the lifecycle authority. A provider may omit its nullable
        // finishedAt field; local duration then safely falls back to our clock.
        let mut stopped_without_finished_at = stopped;
        stopped_without_finished_at.finished_at = None;
        assert!(validated_terminal_provider_session(stopped_without_finished_at, "browser-safe").is_ok());
    }

    #[tokio::test]
    async fn auth_exchange_without_org_is_forwarded_to_iam() {
        let fixture = fixture().await;
        let (status, _, _body) = request(
            &fixture.app,
            "POST",
            "/api/v1/auth/exchange",
            None,
            Some(json!({"short_lived_token":"oac_single_use"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn auth_exchange_persists_public_identity_projection_without_exposing_secrets() {
        let fixture = fixture().await;
        let exchanged_principal = principal("public-owner", IdentityKind::Silicon, true);
        fixture.identity.allow_exchange(
            "oac_single_use",
            "org-1",
            ExchangedAuth {
                access_token: "oat_projected".into(),
                refresh_token: "ort_projected".into(),
                identity: exchanged_principal.clone(),
                scope: "app".into(),
            },
        );
        let (status, headers, body) = request(
            &fixture.app,
            "POST",
            "/api/v1/auth/exchange",
            None,
            Some(json!({"short_lived_token":"oac_single_use","org_id":"org-1"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers["cache-control"], "no-store");
        assert_eq!(data(&body)["identity"]["id"], "public-owner");
        assert_eq!(
            data(&body)["services"],
            json!(["profile", "proxy", "session", "recording", "usage", "search", "fetch"])
        );

        let mut later = exchanged_principal;
        later.public_id = None;
        fixture.identity.allow_identity("oat_projected", later);
        let (status, _, body) =
            request(&fixture.app, "GET", "/api/v1/me", Some(("oat_projected", "org-1")), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(data(&body)["id"], "public-owner");
        let encoded = String::from_utf8(body).unwrap();
        assert!(!encoded.contains("oat_projected"));
        assert!(!encoded.contains("ort_projected"));
    }

    #[tokio::test]
    async fn live_membership_tags_grant_and_revoke_profile_access_without_stale_projection() {
        let fixture = fixture().await;
        let (status, _, _) = request(
            &fixture.app,
            "POST",
            "/api/v1/profiles",
            Some(("oat_owner", "org-1")),
            Some(json!({"name":"Tag protected", "location":"in", "access":["growth"]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let mut viewer = principal("tagged-viewer", IdentityKind::Silicon, true);
        for (tags, expected_count) in [
            (None, 0),
            (Some(vec!["growth".into()]), 1),
            (Some(Vec::new()), 0),
            (Some(vec!["growth".into()]), 1),
            (None, 0),
            (Some(vec!["@owner-1".into()]), 0),
        ] {
            viewer.tags = tags;
            fixture.identity.allow_identity("oat_tagged", viewer.clone());
            let (status, _, body) =
                request(&fixture.app, "GET", "/api/v1/profiles", Some(("oat_tagged", "org-1")), None).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(data(&body).as_array().unwrap().len(), expected_count);
        }
    }

    #[tokio::test]
    async fn scoped_routes_require_bearer_and_org_with_enveloped_errors() {
        let fixture = fixture().await;
        let (status, headers, body) = request(&fixture.app, "GET", "/api/v1/profiles", None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(headers.contains_key("x-request-id"));
        let error: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["error"]["code"], "unauthenticated");

        let response = fixture
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/profiles")
                    .header(AUTHORIZATION, "Bearer oat_owner")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn invalid_proxy_and_profile_lookup_fail_before_provider_or_session_reservation() {
        let fixture = fixture().await;
        let auth = Some(("oat_owner", "org-1"));
        let (status, _, body) = request(
            &fixture.app,
            "POST",
            "/api/v1/profiles",
            auth,
            Some(json!({"name":"Unsupported","location":"zz","access":[]})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["error"]["fields"][0]["field"], "location");
        assert!(fixture.browser.state.lock().unwrap().profiles.is_empty());

        let (status, _, _) = request(
            &fixture.app,
            "POST",
            "/api/v1/sessions",
            auth,
            Some(json!({
                "profile_id":"missing-profile",
                "name":"Must not reserve",
                "description":"lookup fails",
                "ttl":"15m"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _, body) = request(&fixture.app, "GET", "/api/v1/sessions", auth, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(data(&body), json!([]));
    }

    #[tokio::test]
    async fn profile_create_rejects_missing_or_mismatched_provider_user_id_without_activation() {
        let fixture = fixture().await;
        let auth = Some(("oat_owner", "org-1"));

        fixture.browser.state.lock().unwrap().next_profile_user_id = Some(None);
        let (status, _, body) = request(
            &fixture.app,
            "POST",
            "/api/v1/profiles",
            auth,
            Some(json!({"name":"Missing correlation","location":"in","access":[]})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{}", String::from_utf8_lossy(&body));
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"], "provider_failure");

        fixture.browser.state.lock().unwrap().next_profile_user_id = Some(Some("another-local-profile".into()));
        let (status, _, body) = request(
            &fixture.app,
            "POST",
            "/api/v1/profiles",
            auth,
            Some(json!({"name":"Wrong correlation","location":"in","access":[]})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{}", String::from_utf8_lossy(&body));
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"], "provider_failure");

        assert!(fixture.store.provisioning_profiles().await.unwrap().is_empty());
        let (status, _, body) = request(&fixture.app, "GET", "/api/v1/profiles", auth, None).await;
        assert_eq!(status, StatusCode::OK);
        let profiles: Vec<Profile> = serde_json::from_value(data(&body)).unwrap();
        assert_eq!(profiles.len(), 2);
        assert!(profiles.iter().all(|profile| profile.status == ProfileStatus::Retired));
    }

    #[tokio::test]
    async fn profile_update_rejects_a_different_provider_id_or_user_id_echo() {
        let fixture = fixture().await;
        let auth = Some(("oat_owner", "org-1"));
        let (status, _, body) = request(
            &fixture.app,
            "POST",
            "/api/v1/profiles",
            auth,
            Some(json!({"name":"Original","location":"in","access":[]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let profile: Profile = serde_json::from_value(data(&body)).unwrap();

        fixture.browser.state.lock().unwrap().next_update_profile_id = Some("different-provider-profile".into());
        let (status, _, body) = request(
            &fixture.app,
            "PATCH",
            &format!("/api/v1/profiles/{}", profile.id),
            auth,
            Some(json!({"name":"Wrong resource"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{}", String::from_utf8_lossy(&body));

        fixture.browser.state.lock().unwrap().next_update_profile_user_id = Some(None);
        let (status, _, body) = request(
            &fixture.app,
            "PATCH",
            &format!("/api/v1/profiles/{}", profile.id),
            auth,
            Some(json!({"name":"Missing correlation"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{}", String::from_utf8_lossy(&body));

        let (status, _, body) =
            request(&fixture.app, "GET", &format!("/api/v1/profiles/{}", profile.id), auth, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(data(&body)["name"], "Original");
    }

    #[tokio::test]
    async fn retained_profile_provisioning_is_reconciled_from_exact_provider_user_id() {
        let fixture = fixture().await;
        let owner = Identity {
            id: "owner-1".into(),
            name: "owner-1".into(),
            kind: IdentityKind::Silicon,
            tags: Vec::new(),
            verified_aliases: Vec::new(),
        };
        let request = ProfileCreate { name: "Recover me".into(), location: "in".into(), access: Default::default() };
        let reserved = fixture.store.reserve_profile("org-1", &owner, &request, Utc::now()).await.unwrap();
        assert!(fixture.store.profiles("org-1", &owner).await.unwrap().is_empty());
        fixture.browser.state.lock().unwrap().profiles.insert(
            reserved.id.clone(),
            ProviderProfile {
                id: "provider-reconciled".into(),
                created_at: None,
                updated_at: None,
                user_id: Some(reserved.id.clone()),
                name: Some(request.name),
                last_used_at: None,
                cookie_domains: Vec::new(),
            },
        );

        assert_eq!(fixture.state.reconcile_profiles_once().await.unwrap(), 1);
        let active = fixture.store.profile("org-1", &owner, &reserved.id).await.unwrap();
        assert!(active.fingerprint.starts_with("sbf_"));
        assert_eq!(
            fixture.store.provider_profile_id("org-1", &owner, &reserved.id).await.unwrap(),
            "provider-reconciled"
        );
        assert_eq!(fixture.state.reconcile_profiles_once().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn unresolved_profile_prefix_cannot_starve_a_later_profile() {
        let fixture = fixture().await;
        let base = Utc::now();
        // These reservations have no upstream match. A capped query of the oldest
        // 100 rows must not prevent the later recoverable profile from activating.
        // Keep fairness independent of SQLite response time; hung-provider
        // deadlines are covered separately.
        for index in 0_i64..100 {
            fixture
                .store
                .reserve_profile(
                    "org-1",
                    &owner(),
                    &ProfileCreate {
                        name: format!("Unresolved {index}"),
                        location: "in".into(),
                        access: Default::default(),
                    },
                    base + TimeDelta::milliseconds(index),
                )
                .await
                .unwrap();
        }
        let later = fixture
            .store
            .reserve_profile(
                "org-1",
                &owner(),
                &ProfileCreate { name: "Later match".into(), location: "in".into(), access: Default::default() },
                base + TimeDelta::milliseconds(100),
            )
            .await
            .unwrap();
        fixture.browser.state.lock().unwrap().profiles.insert(
            later.id.clone(),
            ProviderProfile {
                id: "provider-later-match".into(),
                created_at: None,
                updated_at: None,
                user_id: Some(later.id.clone()),
                name: Some(later.name.clone()),
                last_used_at: None,
                cookie_domains: Vec::new(),
            },
        );
        let activated = fixture.state.reconcile_profiles_once().await.unwrap();

        assert_eq!(activated, 1);
        assert_eq!(
            fixture.store.provider_profile_id("org-1", &owner(), &later.id).await.unwrap(),
            "provider-later-match"
        );
    }

    #[tokio::test]
    async fn hanging_profile_reconciliation_is_bounded_and_does_not_delay_expiry() {
        let fixture = fixture().await;
        let expired = seed_expired_incognito(&fixture, "before-reconcile").await;
        fixture
            .store
            .reserve_profile(
                "org-1",
                &owner(),
                &ProfileCreate { name: "Pending".into(), location: "in".into(), access: Default::default() },
                Utc::now(),
            )
            .await
            .unwrap();
        fixture.browser.state.lock().unwrap().hang_profile_reconciliation = true;

        let count = tokio::time::timeout(
            Duration::from_millis(250),
            fixture.state.reap_expired_once_with_profile_budget(Duration::from_millis(10)),
        )
        .await
        .expect("profile reconciliation must not hold the maintenance loop")
        .unwrap();

        assert_eq!(count, 1);
        assert_eq!(fixture.browser.stopped(), vec!["provider-expired-before-reconcile"]);
        assert_eq!(fixture.store.session("org-1", &owner(), &expired.id).await.unwrap().status, SessionStatus::Expired);
    }

    #[test]
    fn ttl_stop_lease_exceeds_the_provider_confirmation_chain() {
        let provider_worst_case = Duration::from_secs(2 * 70);
        assert!(TTL_STOP_ATTEMPT_BUDGET > provider_worst_case);
        assert!(TTL_STOP_LEASE > TTL_STOP_ATTEMPT_BUDGET);
    }

    #[tokio::test]
    async fn ttl_reaper_never_preleases_work_which_cannot_start() {
        let fixture = fixture().await;
        let mut provider_ids = Vec::new();
        for index in 0..9 {
            let suffix = format!("hung-stop-{index}");
            seed_expired_incognito(&fixture, &suffix).await;
            provider_ids.push(format!("provider-expired-{suffix}"));
        }
        fixture.browser.state.lock().unwrap().hanging_browser_stops = provider_ids.iter().cloned().collect();

        let count = tokio::time::timeout(
            Duration::from_millis(500),
            fixture.state.reap_expired_sessions_once_with_budgets(Duration::from_millis(20), Duration::from_millis(30)),
        )
        .await
        .expect("the bounded stop batch must return")
        .unwrap();

        assert_eq!(count, usize::try_from(TTL_STOP_CONCURRENCY).unwrap());
        let started = fixture.browser.stopped();
        assert_eq!(started.len(), usize::try_from(TTL_STOP_CONCURRENCY).unwrap());
        let now = Utc::now();
        let unstarted = fixture
            .store
            .claim_expired_sessions(now, now + TimeDelta::minutes(4), 100, &fixture.state.secrets)
            .await
            .unwrap();
        assert_eq!(unstarted.len(), 1);
        assert!(!started.contains(&unstarted[0].runtime.provider_session_id));
    }

    #[tokio::test]
    async fn one_session_finalization_race_does_not_abort_the_remaining_reaper_batch() {
        let fixture = fixture().await;
        let raced = seed_expired_incognito(&fixture, "raced-finalize").await;
        let healthy = seed_expired_incognito(&fixture, "healthy-finalize").await;
        fixture.browser.state.lock().unwrap().finalize_as_ended_on_stop =
            Some(("provider-expired-raced-finalize".into(), raced.id.clone(), fixture.store.clone()));

        assert_eq!(fixture.state.reap_expired_once_with_profile_budget(Duration::from_millis(10)).await.unwrap(), 2);
        assert_eq!(fixture.store.session("org-1", &owner(), &raced.id).await.unwrap().status, SessionStatus::Ended);
        assert_eq!(fixture.store.session("org-1", &owner(), &healthy.id).await.unwrap().status, SessionStatus::Expired);
        let stopped = fixture.browser.stopped();
        assert!(stopped.contains(&"provider-expired-raced-finalize".into()));
        assert!(stopped.contains(&"provider-expired-healthy-finalize".into()));
    }

    #[tokio::test]
    async fn reaper_rechecks_a_waiting_claim_after_manual_finalization() {
        let fixture = fixture().await;
        let session = seed_expired_incognito(&fixture, "manual-race").await;
        let now = Utc::now();
        let claim = fixture
            .store
            .claim_expired_sessions(now, now + TimeDelta::minutes(4), 1, &fixture.state.secrets)
            .await
            .unwrap()
            .pop()
            .unwrap();
        let gate = fixture.state.session_locks.for_session("org-1", &session.id).await;
        let guard = gate.lock.lock().await;
        let state = fixture.state.clone();
        let claimed = claim.clone();
        let reaper = tokio::spawn(async move {
            state.reap_claimed_session(&claimed).await;
        });

        let runtime = fixture
            .store
            .begin_end_session(
                "org-1",
                &owner(),
                &session.id,
                &SessionEnd { note: "manual won".into() },
                &fixture.state.secrets,
            )
            .await
            .unwrap();
        stop_browser_confirmed(fixture.browser.as_ref(), &runtime.provider_session_id).await.unwrap();
        fixture.store.finalize_end_session("org-1", &session.id, Utc::now()).await.unwrap();
        drop(guard);
        tokio::time::timeout(Duration::from_millis(250), reaper).await.unwrap().unwrap();

        assert_eq!(fixture.browser.stopped(), vec!["provider-expired-manual-race"]);
        assert_eq!(fixture.store.session("org-1", &owner(), &session.id).await.unwrap().status, SessionStatus::Ended);
    }

    #[tokio::test]
    async fn manual_end_retries_until_the_recording_url_materializes() {
        let fixture = fixture().await;
        let auth = Some(("oat_owner", "org-1"));
        let (status, _, body) = request(
            &fixture.app,
            "POST",
            "/api/v1/sessions",
            auth,
            Some(json!({
                "incognito": true,
                "name": "Delayed recording",
                "description": "provider post-stop materialization",
                "ttl": "15m"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let session: Session = serde_json::from_value(data(&body)).unwrap();
        let provider_id = "provider-browser-1";
        fixture.browser.state.lock().unwrap().recording_missing_gets.insert(provider_id.into(), 1);

        let (status, _, body) = request(
            &fixture.app,
            "POST",
            &format!("/api/v1/sessions/{}/end", session.id),
            auth,
            Some(json!({"note":"done"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        assert_eq!(data(&body)["status"], "ended");
        assert_eq!(
            fixture.store.recording("org-1", &owner(), &session.id, &fixture.state.secrets).await.unwrap().status,
            silicon_browser_shared::RecordingStatus::Pending
        );

        let first_lookup = Utc::now() + TimeDelta::hours(1);
        assert_eq!(fixture.state.reconcile_recording_sources_once(first_lookup).await.unwrap(), 1);
        assert_eq!(fixture.state.reconcile_recording_sources_once(first_lookup).await.unwrap(), 0);
        assert_eq!(
            fixture.state.reconcile_recording_sources_once(first_lookup + TimeDelta::minutes(1)).await.unwrap(),
            1
        );
        assert_eq!(
            fixture.state.reconcile_recording_sources_once(first_lookup + TimeDelta::minutes(2)).await.unwrap(),
            0
        );
        assert_eq!(fixture.browser.state.lock().unwrap().get_calls, 2);
        assert_eq!(
            fixture.store.recording("org-1", &owner(), &session.id, &fixture.state.secrets).await.unwrap().status,
            silicon_browser_shared::RecordingStatus::Pending
        );
    }

    #[tokio::test]
    async fn explicit_unavailable_recording_stops_after_one_lookup() {
        let fixture = fixture().await;
        let now = Utc::now();
        let session = seed_pending_recording_resolution(&fixture, "unavailable", now).await;
        fixture
            .browser
            .state
            .lock()
            .unwrap()
            .browsers
            .get_mut("provider-recording-unavailable")
            .unwrap()
            .recording_available = Some(false);
        assert_eq!(fixture.state.reconcile_recording_sources_once(now).await.unwrap(), 1);
        assert_eq!(fixture.state.reconcile_recording_sources_once(now + TimeDelta::hours(24)).await.unwrap(), 0);
        assert_eq!(fixture.browser.state.lock().unwrap().get_calls, 1);
        assert_eq!(
            fixture.store.recording("org-1", &owner(), &session.id, &fixture.state.secrets).await.unwrap().status,
            silicon_browser_shared::RecordingStatus::Failed
        );
    }

    #[tokio::test]
    async fn available_or_undisclosed_recording_readiness_keeps_polling() {
        for readiness in [None, Some(true)] {
            let fixture = fixture().await;
            let now = Utc::now();
            let session = seed_pending_recording_resolution(&fixture, "waiting", now).await;
            fixture
                .browser
                .state
                .lock()
                .unwrap()
                .browsers
                .get_mut("provider-recording-waiting")
                .unwrap()
                .recording_available = readiness;
            assert_eq!(fixture.state.reconcile_recording_sources_once(now).await.unwrap(), 1);
            assert_eq!(fixture.state.reconcile_recording_sources_once(now + TimeDelta::hours(1)).await.unwrap(), 1);
            assert_eq!(fixture.browser.state.lock().unwrap().get_calls, 2);
            assert_eq!(
                fixture.store.recording("org-1", &owner(), &session.id, &fixture.state.secrets).await.unwrap().status,
                silicon_browser_shared::RecordingStatus::Pending
            );
        }
    }

    #[tokio::test]
    async fn materialized_recording_url_takes_precedence_over_unavailable_flag() {
        let fixture = fixture().await;
        let now = Utc::now();
        let session = seed_pending_recording_resolution(&fixture, "url-ready", now).await;
        {
            let mut state = fixture.browser.state.lock().unwrap();
            let provider = state.browsers.get_mut("provider-recording-url-ready").unwrap();
            provider.recording_available = Some(false);
            provider.recording_url = Some("https://provider.invalid/recording/final?secret=yes".into());
        }
        assert_eq!(fixture.state.reconcile_recording_sources_once(now).await.unwrap(), 1);
        assert_eq!(fixture.state.reconcile_recording_sources_once(now + TimeDelta::hours(1)).await.unwrap(), 0);
        assert_eq!(fixture.browser.state.lock().unwrap().get_calls, 1);
        assert_eq!(
            fixture.store.recording("org-1", &owner(), &session.id, &fixture.state.secrets).await.unwrap().status,
            silicon_browser_shared::RecordingStatus::Pending
        );
    }

    #[tokio::test]
    async fn ttl_recording_resolution_is_bounded_and_eventually_fails_honestly() {
        let fixture = fixture().await;
        let session = seed_expired_incognito(&fixture, "missing-recording").await;
        let provider_id = "provider-expired-missing-recording";
        fixture
            .browser
            .state
            .lock()
            .unwrap()
            .recording_missing_gets
            .insert(provider_id.into(), RECORDING_RECONCILIATION_MAX_ATTEMPTS + 1);

        assert_eq!(fixture.state.reap_expired_once_with_profile_budget(Duration::from_millis(10)).await.unwrap(), 1);
        assert_eq!(fixture.store.session("org-1", &owner(), &session.id).await.unwrap().status, SessionStatus::Expired);
        assert_eq!(
            fixture.store.recording("org-1", &owner(), &session.id, &fixture.state.secrets).await.unwrap().status,
            silicon_browser_shared::RecordingStatus::Pending
        );

        let first_lookup = Utc::now() + TimeDelta::hours(1);
        for attempt in 0..RECORDING_RECONCILIATION_MAX_ATTEMPTS {
            assert_eq!(
                fixture
                    .state
                    .reconcile_recording_sources_once(first_lookup + TimeDelta::hours(i64::from(attempt)))
                    .await
                    .unwrap(),
                1
            );
        }
        assert_eq!(
            fixture.state.reconcile_recording_sources_once(first_lookup + TimeDelta::hours(24)).await.unwrap(),
            0
        );
        assert_eq!(
            fixture.store.recording("org-1", &owner(), &session.id, &fixture.state.secrets).await.unwrap().status,
            silicon_browser_shared::RecordingStatus::Failed
        );
        assert_eq!(
            fixture.browser.state.lock().unwrap().get_calls,
            usize::try_from(RECORDING_RECONCILIATION_MAX_ATTEMPTS).unwrap()
        );
    }

    #[tokio::test]
    async fn cancelled_recording_lookups_still_obey_the_durable_attempt_cap() {
        let fixture = fixture().await;
        let session = seed_expired_incognito(&fixture, "cancelled-recording").await;
        let provider_id = "provider-expired-cancelled-recording";
        fixture.browser.state.lock().unwrap().recording_missing_gets.insert(provider_id.into(), 100);
        assert_eq!(fixture.state.reap_expired_once_with_profile_budget(Duration::from_millis(10)).await.unwrap(), 1);

        // Claim leases without resolving or rescheduling them, modeling a
        // worker cancellation/process crash after each durable increment.
        let first_lookup = Utc::now() + TimeDelta::hours(1);
        for attempt in 0..RECORDING_RECONCILIATION_MAX_ATTEMPTS {
            let now = first_lookup + TimeDelta::hours(i64::from(attempt));
            let claim = fixture
                .store
                .claim_recording_source_resolutions(now, now + TimeDelta::minutes(2), 1)
                .await
                .unwrap()
                .pop()
                .unwrap();
            assert_eq!(claim.attempt, attempt + 1);
        }

        assert_eq!(
            fixture.state.reconcile_recording_sources_once(first_lookup + TimeDelta::hours(24)).await.unwrap(),
            1
        );
        assert_eq!(fixture.browser.state.lock().unwrap().get_calls, 0);
        assert_eq!(
            fixture.store.recording("org-1", &owner(), &session.id, &fixture.state.secrets).await.unwrap().status,
            silicon_browser_shared::RecordingStatus::Failed
        );
    }

    #[tokio::test]
    async fn recording_reconciliation_does_not_consume_unstarted_claim_attempts() {
        let fixture = fixture().await;
        let due_at = Utc::now();
        let mut sessions = Vec::new();
        for index in 0..9 {
            sessions.push(seed_pending_recording_resolution(&fixture, &index.to_string(), due_at).await);
        }
        fixture.browser.state.lock().unwrap().hang_recording_reconciliation = true;

        let count = tokio::time::timeout(
            Duration::from_millis(500),
            fixture.state.reconcile_recording_sources_once_with_budgets(
                due_at,
                Duration::from_millis(20),
                Duration::from_millis(30),
            ),
        )
        .await
        .expect("the bounded recording batch must return")
        .unwrap();

        assert_eq!(count, usize::try_from(RECORDING_RECONCILIATION_BATCH).unwrap());
        assert_eq!(fixture.browser.state.lock().unwrap().get_calls, count);
        let unstarted = fixture
            .store
            .claim_recording_source_resolutions(due_at, due_at + TimeDelta::minutes(2), 100)
            .await
            .unwrap();
        assert_eq!(unstarted.len(), 1);
        assert!(sessions.iter().any(|session| session.id == unstarted[0].session_id));
        assert_eq!(unstarted[0].attempt, 1);
    }

    #[tokio::test]
    async fn hanging_recording_lookup_does_not_delay_ttl_expiry_loop() {
        let fixture = fixture().await;
        let started_at = Utc::now() - TimeDelta::minutes(1);
        let request = SessionCreate::incognito(
            "Await recording",
            "maintenance isolation",
            silicon_browser_shared::SessionTtl::Minutes15,
        );
        let awaiting = fixture.store.reserve_session("org-1", &owner(), &request, started_at).await.unwrap();
        let provider = fake_provider_browser("provider-await-recording", false);
        fixture.browser.state.lock().unwrap().browsers.insert(provider.id.clone(), provider.clone());
        fixture
            .store
            .activate_session(
                "org-1",
                &awaiting.id,
                &provider_session(&provider).unwrap(),
                &fixture.state.secrets,
                started_at,
            )
            .await
            .unwrap();
        fixture
            .store
            .begin_end_session(
                "org-1",
                &owner(),
                &awaiting.id,
                &SessionEnd { note: "done".into() },
                &fixture.state.secrets,
            )
            .await
            .unwrap();
        fixture.browser.state.lock().unwrap().browsers.insert(
            provider.id.clone(),
            ProviderBrowserSession { recording_url: None, ..fake_provider_browser(&provider.id, true) },
        );
        fixture
            .store
            .queue_recording_source_resolution("org-1", &awaiting.id, 1_000, Utc::now() - TimeDelta::minutes(1))
            .await
            .unwrap();
        fixture.store.finalize_end_session("org-1", &awaiting.id, Utc::now()).await.unwrap();

        let expiring = seed_expired_incognito(&fixture, "while-recording-hangs").await;
        fixture.browser.state.lock().unwrap().hang_recording_reconciliation = true;
        let maintenance = spawn_ttl_reaper(fixture.state.clone(), Duration::from_millis(10));
        tokio::time::timeout(Duration::from_millis(500), async {
            loop {
                let expired = fixture.store.session("org-1", &owner(), &expiring.id).await.unwrap().status
                    == SessionStatus::Expired;
                let recording_lookup_started = fixture.browser.state.lock().unwrap().get_calls >= 1;
                if expired && recording_lookup_started {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("TTL expiry must run independently of recording lookup latency");
        maintenance.abort();
    }

    #[tokio::test]
    async fn provider_profile_success_with_local_activation_failure_remains_reconcilable() {
        let fixture = fixture().await;
        let auth = Some(("oat_owner", "org-1"));
        let (status, _, body) = request(
            &fixture.app,
            "POST",
            "/api/v1/profiles",
            auth,
            Some(json!({"name":"First","location":"in","access":[]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));

        // Force a local unique-constraint activation error after the fake
        // provider has accepted and correlated the second create.
        fixture.browser.state.lock().unwrap().next_profile_id = Some("provider-profile-1".into());
        let (status, _, body) = request(
            &fixture.app,
            "POST",
            "/api/v1/profiles",
            auth,
            Some(json!({"name":"Retained","location":"in","access":[]})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{}", String::from_utf8_lossy(&body));
        assert_eq!(fixture.store.provisioning_profiles().await.unwrap().len(), 1);
        // Provisioning placeholders are internal and are not advertised as
        // active profiles while background reconciliation is pending.
        let owner = Identity {
            id: "owner-1".into(),
            name: "owner-1".into(),
            kind: IdentityKind::Silicon,
            tags: Vec::new(),
            verified_aliases: Vec::new(),
        };
        assert_eq!(fixture.store.profiles("org-1", &owner).await.unwrap().len(), 1);
        assert_eq!(fixture.browser.state.lock().unwrap().profiles.len(), 2);
    }

    #[tokio::test]
    async fn failed_local_session_activation_holds_slot_until_provider_stop_is_confirmed() {
        let fixture = fixture().await;
        let owner = Identity {
            id: "owner-1".into(),
            name: "owner-1".into(),
            kind: IdentityKind::Silicon,
            tags: Vec::new(),
            verified_aliases: Vec::new(),
        };
        let profile = fixture
            .store
            .create_profile(
                "org-1",
                &owner,
                &ProfileCreate { name: "Lifecycle".into(), location: "in".into(), access: Default::default() },
                "provider-profile-lifecycle",
                "sbf_lifecycle",
                Utc::now(),
            )
            .await
            .unwrap();
        let request = SessionCreate::with_profile(
            &profile.id,
            "Activation failure",
            "retain remote runtime",
            silicon_browser_shared::SessionTtl::Minutes30,
        );
        let reserved = fixture.store.reserve_session("org-1", &owner, &request, Utc::now()).await.unwrap();
        let provider = fake_provider_browser("provider-browser-recovery", false);
        let runtime = provider_session(&provider).unwrap();
        {
            let mut browser = fixture.browser.state.lock().unwrap();
            browser.browsers.insert(provider.id.clone(), provider);
            browser.unconfirmed_browser_stop = true;
        }

        // This directly exercises the compensating path reached after a local
        // activate_session error. The first stop remains ambiguous and active.
        recover_failed_session_start(
            &fixture.state,
            "org-1",
            &reserved,
            &runtime.id,
            Some(&runtime),
            "injected local activation failure",
        )
        .await;
        assert!(matches!(
            fixture.store.reserve_session("org-1", &owner, &request, Utc::now()).await,
            Err(StoreError::ProfileBusy { .. })
        ));

        fixture.browser.state.lock().unwrap().unconfirmed_browser_stop = false;
        recover_failed_session_start(
            &fixture.state,
            "org-1",
            &reserved,
            &runtime.id,
            Some(&runtime),
            "injected local activation failure",
        )
        .await;
        assert!(fixture.store.reserve_session("org-1", &owner, &request, Utc::now()).await.is_ok());
        assert_eq!(
            fixture.store.recording("org-1", &owner, &reserved.id, &fixture.state.secrets).await.unwrap().status,
            silicon_browser_shared::RecordingStatus::Pending
        );
    }

    #[tokio::test]
    async fn profile_session_run_live_redeem_end_recording_and_usage_match_client_wire() {
        let fixture = fixture().await;
        let auth = Some(("oat_owner", "org-1"));
        let (status, _, body) = request(
            &fixture.app,
            "POST",
            "/api/v1/profiles",
            auth,
            Some(json!({"name":"Primary","location":"in","access":[]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let profile: Profile = serde_json::from_value(data(&body)).unwrap();
        assert!(profile.fingerprint.starts_with("sbf_"));
        assert!(!profile.fingerprint.contains("provider-profile"));

        let (status, _, body) = request(
            &fixture.app,
            "POST",
            "/api/v1/sessions",
            auth,
            Some(json!({
                "profile_id": profile.id,
                "name": "Checkout",
                "description": "Test the purchase path",
                "ttl": "30m"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let session: Session = serde_json::from_value(data(&body)).unwrap();

        // Browser Use charges the configured maximum duration up front. An
        // active sample can therefore cost more than the authoritative bill
        // returned after unused time is refunded at stop.
        {
            let mut browser = fixture.browser.state.lock().unwrap();
            let active = browser.browsers.values_mut().next().unwrap();
            active.proxy_used_mb = "+.4000000000004".into();
            active.proxy_cost = "+0.2000000000004".into();
            active.browser_cost = "1.0000000000004".into();
        }

        let (status, _, _) =
            request(&fixture.app, "GET", &format!("/api/v1/sessions/{}", session.id), auth, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(fixture.browser.state.lock().unwrap().get_calls, 1);

        let active_usage = fixture.store.usage("org-1", &owner(), &session.id).await.unwrap();
        assert_eq!(active_usage.proxy_bytes_unclassified, 400_000);
        assert_eq!(active_usage.cost.browser.micros, 1_000_000);
        assert_eq!(active_usage.cost.proxy_unclassified.micros, 200_000);

        let report = serde_json::json!({"command_id":Uuid::now_v7(), "command":"open https://example.test",
            "flags":["--json", "value with spaces"], "started_at":Utc::now(), "finished_at":Utc::now(),
            "exit_code":0,"truncated":false});
        let (status, _, _) =
            request(&fixture.app, "POST", &format!("/api/v1/sessions/{}/commands", session.id), auth, Some(report))
                .await;
        assert_eq!(status, StatusCode::OK);

        let (status, _, body) =
            request(&fixture.app, "GET", &format!("/api/v1/sessions/{}/logs", session.id), auth, None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(data(&body)[0]["command"].as_str().unwrap().contains("'value with spaces'"));

        let (status, _, body) =
            request(&fixture.app, "POST", &format!("/api/v1/sessions/{}/live", session.id), auth, Some(Value::Null))
                .await;
        assert_eq!(status, StatusCode::OK);
        let public_live = data(&body)["url"].as_str().unwrap().to_owned();
        assert!(public_live.starts_with(&format!("https://browser.example/sessions/{}/live#grant=", session.id)));
        assert!(!public_live.contains("provider.invalid"));
        let grant = public_live.split_once("#grant=").unwrap().1;

        let (status, headers, body) = request(
            &fixture.app,
            "POST",
            &format!("/api/v1/sessions/{}/live/redeem", session.id),
            Some(("oat_viewer", "org-1")),
            Some(json!({"grant":grant})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        assert_eq!(headers["cache-control"], "no-store");
        assert!(data(&body)["url"].as_str().unwrap().contains("provider.invalid/live"));

        // Simulate an ambiguous provider 5xx after stop committed. The endpoint
        // must GET-confirm the terminal snapshot and finish locally.
        {
            let mut browser = fixture.browser.state.lock().unwrap();
            browser.ambiguous_browser_stop = true;
            // Browser Use's decimal schema permits a leading plus, an omitted
            // integer zero, and arbitrary fractional precision. A valid
            // terminal snapshot must not leave the local session in `ending`.
            browser.terminal_decimals =
                Some(("+.5000000000004".into(), "+0.1000000000004".into(), ".333333500000".into()));
        }

        let (status, _, body) = request(
            &fixture.app,
            "POST",
            &format!("/api/v1/sessions/{}/end", session.id),
            auth,
            Some(json!({"note":"done"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        assert_eq!(data(&body)["status"], "ended");
        assert_eq!(fixture.browser.stopped().len(), 1);
        assert_eq!(fixture.browser.state.lock().unwrap().get_calls, 2);

        let (status, _, body) =
            request(&fixture.app, "GET", &format!("/api/v1/recordings/{}", session.id), auth, None).await;
        assert_eq!(status, StatusCode::OK);
        let recording: Recording = serde_json::from_value(data(&body)).unwrap();
        assert_eq!(recording.status, silicon_browser_shared::RecordingStatus::Pending);
        assert!(recording.briefcase_link.is_none());

        let (status, _, body) =
            request(&fixture.app, "GET", &format!("/api/v1/usage/{}", session.id), auth, None).await;
        assert_eq!(status, StatusCode::OK);
        let usage: Usage = serde_json::from_value(data(&body)).unwrap();
        assert_eq!(usage.proxy_bytes_in, 0);
        assert_eq!(usage.proxy_bytes_out, 0);
        assert_eq!(usage.proxy_bytes_unclassified, 500_000);
        assert_eq!(usage.cost.browser.micros, 333_334);
        assert_eq!(usage.cost.proxy_unclassified.micros, 100_000);
        assert_eq!(usage.cost.total.micros, 433_334);

        let (status, headers, _) = request(&fixture.app, "GET", "/api/v1/usage/org", auth, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers["x-sb-usage-scope"], "organization");
    }

    /// Test group: an active IAM organization member can see the true
    /// aggregate bill without gaining a session list or an identity oracle.
    #[tokio::test]
    async fn organization_usage_includes_hidden_sessions_but_not_other_orgs() {
        async fn seed(
            fixture: &Fixture,
            org_id: &str,
            initiator: &Identity,
            suffix: &str,
            browser_millis: u64,
            browser_cost: &str,
            started_at: DateTime<Utc>,
        ) -> Session {
            let request = SessionCreate::incognito(
                format!("Usage {suffix}"),
                "organization usage authorization test",
                silicon_browser_shared::SessionTtl::Minutes15,
            );
            let reserved = fixture.store.reserve_session(org_id, initiator, &request, started_at).await.unwrap();
            let provider = fake_provider_browser(&format!("provider-usage-{suffix}"), false);
            fixture
                .store
                .activate_session(
                    org_id,
                    &reserved.id,
                    &provider_session(&provider).unwrap(),
                    &fixture.state.secrets,
                    started_at,
                )
                .await
                .unwrap();
            fixture
                .store
                .record_usage(
                    org_id,
                    &reserved.id,
                    &UsageSample {
                        browser_millis,
                        proxy_bytes_in: None,
                        proxy_bytes_out: None,
                        proxy_bytes_unclassified: None,
                        browser_cost: browser_cost.into(),
                        proxy_cost: "0".into(),
                        currency: "USD".into(),
                        sampled_at: started_at,
                    },
                )
                .await
                .unwrap();
            reserved
        }

        let fixture = fixture().await;
        let hidden_owner = Identity {
            id: "hidden-owner".into(),
            name: "hidden-owner".into(),
            kind: IdentityKind::Silicon,
            tags: Vec::new(),
            verified_aliases: Vec::new(),
        };
        let started_at = Utc::now() - TimeDelta::minutes(1);
        seed(&fixture, "org-1", &owner(), "owner", 60_000, "0.10", started_at).await;
        seed(&fixture, "org-1", &hidden_owner, "hidden", 120_000, "0.20", started_at).await;
        seed(&fixture, "org-2", &hidden_owner, "other-org", 900_000, "9", started_at).await;

        let auth = Some(("oat_viewer", "org-1"));
        let (status, _, body) = request(&fixture.app, "GET", "/api/v1/usage", auth, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(data(&body), json!([]), "session usage must remain ACL-scoped");

        let (status, headers, body) = request(&fixture.app, "GET", "/api/v1/usage/org", auth, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers["x-sb-usage-scope"], "organization");
        assert_eq!(data(&body)["sessions"], 2);
        assert_eq!(data(&body)["browser_seconds"], 180);
        assert_eq!(data(&body)["cost"]["total"]["micros"], 300_000);

        let date = started_at.format("%d-%m-%Y");
        let uri = format!("/api/v1/usage/org?filter=between:{date}%3D{date}");
        let (status, _, body) = request(&fixture.app, "GET", &uri, auth, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(data(&body)["sessions"], 2);

        let (status, _, body) =
            request(&fixture.app, "GET", "/api/v1/usage/org?filter=for:%40hidden-owner", auth, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let error: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["error"]["code"], "invalid_filter");
        assert_eq!(error["error"]["message"], "organization usage supports only the between date window");
    }

    #[tokio::test]
    async fn discovery_uses_scoped_actor_pool_and_returns_public_envelope() {
        let fixture = fixture().await;
        let (status, _, body) = request(
            &fixture.app,
            "POST",
            "/api/v1/search",
            Some(("oat_owner", "org-1")),
            Some(json!({"query":"browser testing","purpose":"contract test","type":"web","page":0})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        assert_eq!(data(&body)["results"][0]["title"], "browser testing");
    }

    #[tokio::test]
    async fn delayed_stop_retry_records_provider_duration_with_or_without_recording_source() {
        for missing_source in [false, true] {
            let fixture = fixture().await;
            let started_at = Utc::now() - TimeDelta::minutes(5);
            let reserved = fixture
                .store
                .reserve_session(
                    "org-1",
                    &owner(),
                    &SessionCreate::incognito(
                        "Delayed retry",
                        "stop already happened remotely",
                        silicon_browser_shared::SessionTtl::Minutes15,
                    ),
                    started_at,
                )
                .await
                .unwrap();
            let provider = fake_provider_browser("provider-delayed-stop", false);
            fixture
                .store
                .activate_session(
                    "org-1",
                    &reserved.id,
                    &provider_session(&provider).unwrap(),
                    &fixture.state.secrets,
                    started_at,
                )
                .await
                .unwrap();
            // Model the durable state left by an earlier uncertain stop. The
            // retry observes a 90-second provider run, not five minutes of use.
            fixture
                .store
                .begin_end_session(
                    "org-1",
                    &owner(),
                    &reserved.id,
                    &SessionEnd { note: "previous uncertain stop".into() },
                    &fixture.state.secrets,
                )
                .await
                .unwrap();
            if missing_source {
                fixture.browser.state.lock().unwrap().recording_missing_gets.insert(provider.id.clone(), 1);
            }
            let (status, _, _) = request(
                &fixture.app,
                "POST",
                &format!("/api/v1/sessions/{}/end", reserved.id),
                Some(("oat_owner", "org-1")),
                Some(json!({"note":"retry"})),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            let recording =
                fixture.store.recording("org-1", &owner(), &reserved.id, &fixture.state.secrets).await.unwrap();
            let usage = fixture.store.usage("org-1", &owner(), &reserved.id).await.unwrap();
            assert_eq!(recording.duration_seconds, 90);
            assert_eq!(recording.duration_seconds, usage.browser_seconds);
        }
    }

    #[tokio::test]
    async fn incognito_terminal_usage_keeps_provider_proxy_bytes_and_cost() {
        let fixture = fixture().await;
        let auth = Some(("oat_owner", "org-1"));
        let (status, _, body) = request(&fixture.app, "POST", "/api/v1/sessions", auth,
            Some(json!({"incognito":true,"name":"Metering discrepancy","description":"preserve reported counters","ttl":"15m"}))).await;
        assert_eq!(status, StatusCode::OK);
        let session: Session = serde_json::from_value(data(&body)).unwrap();
        fixture.browser.state.lock().unwrap().terminal_decimals = Some((
            "1.1601905822753905812".into(),
            "0.00022659972310066222289062500".into(),
            "0.0003333333333333333333333333333".into(),
        ));
        let (status, _, _) = request(
            &fixture.app,
            "POST",
            &format!("/api/v1/sessions/{}/end", session.id),
            auth,
            Some(json!({"note":"done"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _, body) =
            request(&fixture.app, "GET", &format!("/api/v1/usage/{}", session.id), auth, None).await;
        assert_eq!(status, StatusCode::OK);
        let usage = data(&body);
        assert_eq!(usage["proxy_bytes_in"], 0);
        assert_eq!(usage["proxy_bytes_out"], 0);
        assert_eq!(usage["proxy_bytes_unclassified"], 1_160_191);
        assert_eq!(usage["cost"]["proxy_unclassified"]["micros"], 227);
        assert_eq!(usage["cost"]["total"]["micros"], 560);
    }

    #[tokio::test]
    async fn profile_create_reconciles_but_ambiguous_browser_start_retains_the_slot() {
        assert_uncertain_creates_preserve_remote_resources(false, false).await;
    }

    #[tokio::test]
    async fn malformed_committed_creates_recover_profile_and_retain_browser_slot() {
        assert_uncertain_creates_preserve_remote_resources(true, false).await;
    }

    #[tokio::test]
    async fn empty_metadata_search_after_ambiguous_start_retains_profile_slot() {
        assert_uncertain_creates_preserve_remote_resources(false, true).await;
    }

    #[tokio::test]
    async fn malformed_committed_stop_is_confirmed_by_a_terminal_lookup() {
        let browser = FakeBrowser::default();
        {
            let mut state = browser.state.lock().unwrap();
            state.browsers.insert("provider-test".into(), fake_provider_browser("provider-test", false));
            state.ambiguous_browser_stop = true;
            state.malformed_mutation_response = true;
        }
        let stopped = stop_browser_confirmed(&browser, "provider-test").await.unwrap();
        assert_eq!(stopped.status, "stopped");
        assert_eq!(browser.state.lock().unwrap().get_calls, 1);
    }

    async fn assert_uncertain_creates_preserve_remote_resources(malformed: bool, empty_lookup: bool) {
        let fixture = fixture().await;
        {
            let mut state = fixture.browser.state.lock().unwrap();
            state.ambiguous_profile_create = true;
            state.malformed_mutation_response = malformed;
        }
        let (status, _, body) = request(
            &fixture.app,
            "POST",
            "/api/v1/profiles",
            Some(("oat_owner", "org-1")),
            Some(json!({"name":"Recovered","location":"in","access":[]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let profile: Profile = serde_json::from_value(data(&body)).unwrap();

        {
            let mut state = fixture.browser.state.lock().unwrap();
            state.ambiguous_profile_create = false;
            state.ambiguous_browser_start = true;
            state.empty_browser_reconciliation = empty_lookup;
        }
        let (status, _, body) = request(
            &fixture.app,
            "POST",
            "/api/v1/sessions",
            Some(("oat_owner", "org-1")),
            Some(json!({
                "profile_id":profile.id,
                "name":"Recovered",
                "description":"ambiguous provider response",
                "ttl":"15m"
            })),
        )
        .await;
        let expected_status = if malformed { StatusCode::BAD_GATEWAY } else { StatusCode::SERVICE_UNAVAILABLE };
        let expected_code = if malformed { "provider_failure" } else { "provider_unavailable" };
        assert_eq!(status, expected_status, "{}", String::from_utf8_lossy(&body));
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"], expected_code);
        assert!(fixture.browser.state.lock().unwrap().last_reconciliation_id.is_some());

        // The upstream POST may still commit despite an empty or failed lookup,
        // so the profile slot remains unavailable until the reservation's
        // TTL rather than allowing a second remote browser to overlap it.
        fixture.browser.state.lock().unwrap().ambiguous_browser_start = false;
        let (status, _, body) = request(
            &fixture.app,
            "POST",
            "/api/v1/sessions",
            Some(("oat_owner", "org-1")),
            Some(json!({
                "profile_id":profile.id,
                "name":"Must remain blocked",
                "description":"potential remote browser still exists",
                "ttl":"15m"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{}", String::from_utf8_lossy(&body));
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"], "profile_busy");
    }
}
