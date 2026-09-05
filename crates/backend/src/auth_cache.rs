//! Short-lived authorization snapshots for the metadata API. Browser traffic never uses this cache.

use std::collections::HashMap;
use std::future::Future;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use chrono::Utc;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::auth::{IdentityError, PrincipalIdentity, UpstreamFailure};

const MAX_ENTRIES: usize = 4096;
const STRIPES: usize = 128;
const TTL: Duration = Duration::from_secs(15);

struct Entry {
    identity: PrincipalIdentity,
    until: Instant,
    generation: u64,
}

/// A verified IAM webhook invalidates all snapshots. Missing webhook delivery is bounded by
/// the fifteen-second TTL; token expiry is always honored. Token bytes are never retained.
pub struct AuthorizationCache {
    entries: RwLock<HashMap<[u8; 32], Entry>>,
    generation: AtomicU64,
    flights: [Mutex<()>; STRIPES],
}

impl Default for AuthorizationCache {
    fn default() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            generation: AtomicU64::new(0),
            flights: std::array::from_fn(|_| Mutex::new(())),
        }
    }
}

impl AuthorizationCache {
    pub fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.entries.write().unwrap_or_else(|error| error.into_inner()).clear();
    }

    fn cached(&self, key: &[u8; 32]) -> Option<PrincipalIdentity> {
        let entries = self.entries.read().unwrap_or_else(|error| error.into_inner());
        entries
            .get(key)
            .filter(|entry| {
                entry.generation == self.generation.load(Ordering::SeqCst)
                    && Instant::now() < entry.until
                    && Utc::now() < entry.identity.expires_at
            })
            .map(|entry| entry.identity.clone())
    }

    pub async fn resolve<F, Fut>(
        &self,
        bearer: &str,
        org: &str,
        mut fetch: F,
    ) -> Result<PrincipalIdentity, IdentityError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<PrincipalIdentity, IdentityError>>,
    {
        let mut hash = Sha256::new();
        hash.update((bearer.len() as u64).to_be_bytes());
        hash.update(bearer.as_bytes());
        hash.update(org.as_bytes());
        let key: [u8; 32] = hash.finalize().into();
        if let Some(identity) = self.cached(&key) {
            return Ok(identity);
        }
        let _flight = self.flights[usize::from(key[0]) % STRIPES].lock().await;
        if let Some(identity) = self.cached(&key) {
            return Ok(identity);
        }
        // A change arriving while IAM is answering fences that response. Retry once against
        // the new generation; sustained change asks the caller to retry rather than using stale data.
        for _ in 0..2 {
            let generation = self.generation.load(Ordering::SeqCst);
            let identity = fetch().await?;
            if self.generation.load(Ordering::SeqCst) != generation {
                continue;
            }
            if identity.expires_at <= Utc::now() {
                return Err(IdentityError::Unauthenticated);
            }
            let mut entries = self.entries.write().unwrap_or_else(|error| error.into_inner());
            if self.generation.load(Ordering::SeqCst) != generation {
                continue;
            }
            if entries.len() >= MAX_ENTRIES {
                entries.retain(|_, entry| entry.until > Instant::now() && entry.generation == generation);
                if entries.len() >= MAX_ENTRIES {
                    entries.clear();
                }
            }
            entries.insert(key, Entry { identity: identity.clone(), until: Instant::now() + TTL, generation });
            return Ok(identity);
        }
        Err(IdentityError::Upstream {
            kind: UpstreamFailure::Unavailable,
            request_id: None,
            retry_after: Some(Duration::from_secs(1)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silicon_browser_shared::IdentityKind;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use uuid::Uuid;

    fn identity() -> PrincipalIdentity {
        PrincipalIdentity {
            principal_id: Uuid::nil(),
            public_id: Some("test".into()),
            tags: None,
            kind: IdentityKind::Carbon,
            org_id: "tos".into(),
            membership_id: Uuid::nil(),
            authorization_epoch: 1,
            expires_at: Utc::now() + chrono::Duration::minutes(5),
        }
    }

    /// Test group: concurrent metadata calls collapse, org boundaries hold, and signed changes invalidate.
    #[tokio::test]
    async fn concurrent_calls_share_one_fetch_and_invalidation_requires_fresh_auth() {
        let cache = Arc::new(AuthorizationCache::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..500 {
            let cache = cache.clone();
            let calls = calls.clone();
            tasks.spawn(async move {
                cache
                    .resolve("oat_same", "tos", || async {
                        calls.fetch_add(1, Ordering::SeqCst);
                        tokio::task::yield_now().await;
                        Ok(identity())
                    })
                    .await
                    .unwrap()
            });
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        cache.invalidate();
        assert_eq!(
            cache.resolve("oat_same", "tos", || async { Err(IdentityError::Unauthenticated) }).await,
            Err(IdentityError::Unauthenticated)
        );
        assert_eq!(
            cache.resolve("oat_same", "other", || async { Err(IdentityError::Forbidden) }).await,
            Err(IdentityError::Forbidden)
        );
    }

    #[tokio::test]
    async fn change_during_fetch_and_expired_tokens_are_never_cached() {
        let cache = AuthorizationCache::default();
        let calls = AtomicUsize::new(0);
        let result = cache
            .resolve("oat_stale", "tos", || async {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    cache.invalidate();
                    Ok(identity())
                } else {
                    Err(IdentityError::Unauthenticated)
                }
            })
            .await;
        assert_eq!(result, Err(IdentityError::Unauthenticated));
        let result = cache
            .resolve("oat_expired", "tos", || async {
                let mut value = identity();
                value.expires_at = Utc::now() - chrono::Duration::seconds(1);
                Ok(value)
            })
            .await;
        assert_eq!(result, Err(IdentityError::Unauthenticated));
    }
}
