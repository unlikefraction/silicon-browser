use async_trait::async_trait;

use super::error::{ProviderError, ProviderResult};

/// A short-lived grant authorising Briefcase work as the session initiator.
///
/// It intentionally has no accessor returning `&str`; only a concrete OBO adapter should be
/// given access to the inner value in this module.
#[derive(Clone, PartialEq, Eq)]
pub struct OnBehalfOfGrant(String);

impl OnBehalfOfGrant {
    pub fn new(value: impl Into<String>) -> ProviderResult<Self> {
        let value = value.into();
        if value.trim().is_empty() || value.contains(['\r', '\n']) {
            return Err(ProviderError::InvalidInput("invalid OBO grant".into()));
        }
        Ok(Self(value))
    }

    /// Available to the eventual Briefcase adapter, but not exposed outside this crate.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for OnBehalfOfGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OnBehalfOfGrant([REDACTED])")
    }
}

#[derive(Clone)]
pub enum ArtifactSource {
    Bytes(Vec<u8>),
    /// A short-lived upstream download URL. Implementations must never log it.
    RemoteUrl(String),
}

impl std::fmt::Debug for ArtifactSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bytes(bytes) => f.debug_tuple("Bytes").field(&format_args!("{} bytes", bytes.len())).finish(),
            Self::RemoteUrl(_) => f.write_str("RemoteUrl([REDACTED])"),
        }
    }
}

impl ArtifactSource {
    pub(crate) fn remote_url(&self) -> Option<&str> {
        match self {
            Self::RemoteUrl(url) => Some(url),
            Self::Bytes(_) => None,
        }
    }

    pub(crate) fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(bytes) => Some(bytes),
            Self::RemoteUrl(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PutArtifact {
    pub org_id: String,
    pub actor_id: String,
    /// Relative Briefcase path, for example `private/<actor>/sb/<session>/recording.mp4`.
    pub path: String,
    pub content_type: String,
    pub source: ArtifactSource,
    pub obo: OnBehalfOfGrant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashArtifact {
    pub org_id: String,
    pub actor_id: String,
    pub path: String,
    pub obo: OnBehalfOfGrant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtifactState {
    Stored,
    Trashed,
    /// The operation is durably pending elsewhere; the adapter did not claim success.
    Deferred,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactReceipt {
    pub state: ArtifactState,
    pub location: Option<String>,
    pub reason: Option<String>,
}

impl ArtifactReceipt {
    fn deferred(reason: &str) -> Self {
        Self { state: ArtifactState::Deferred, location: None, reason: Some(reason.to_owned()) }
    }
}

#[async_trait]
pub trait ArtifactStore: Send + Sync {
    async fn put_on_behalf(&self, request: PutArtifact) -> ProviderResult<ArtifactReceipt>;
    async fn trash_on_behalf(&self, request: TrashArtifact) -> ProviderResult<ArtifactReceipt>;
}

/// Explicit placeholder while Briefcase and its OBO handshake are unavailable.
///
/// Returning a `Deferred` receipt, rather than `Ok(Stored)` or a fatal error, lets session
/// finalisation enqueue the work without either losing it or lying about storage.
#[derive(Clone, Debug)]
pub struct DeferredArtifactStore {
    reason: String,
}

impl Default for DeferredArtifactStore {
    fn default() -> Self {
        Self::new("Briefcase OBO is not configured")
    }
}

impl DeferredArtifactStore {
    pub fn new(reason: impl Into<String>) -> Self {
        Self { reason: reason.into() }
    }
}

#[async_trait]
impl ArtifactStore for DeferredArtifactStore {
    async fn put_on_behalf(&self, request: PutArtifact) -> ProviderResult<ArtifactReceipt> {
        validate_artifact_identity(&request.org_id, &request.actor_id)?;
        validate_owned_artifact_path(&request.actor_id, &request.path)?;
        if request.content_type.trim().is_empty() {
            return Err(ProviderError::InvalidInput("artifact content type is empty".into()));
        }
        // Touch the opaque inputs so future refactors cannot accidentally make validation imply
        // that data was transferred. The receipt below remains explicitly deferred.
        let _ = request.obo.expose();
        let _ = request.source.bytes().or_else(|| request.source.remote_url().map(str::as_bytes));
        Ok(ArtifactReceipt::deferred(&self.reason))
    }

    async fn trash_on_behalf(&self, request: TrashArtifact) -> ProviderResult<ArtifactReceipt> {
        validate_artifact_identity(&request.org_id, &request.actor_id)?;
        validate_owned_artifact_path(&request.actor_id, &request.path)?;
        let _ = request.obo.expose();
        Ok(ArtifactReceipt::deferred(&self.reason))
    }
}

fn validate_artifact_identity(org_id: &str, actor_id: &str) -> ProviderResult<()> {
    let safe = |value: &str| {
        !value.is_empty()
            && value.len() <= 255
            && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.'))
    };
    if !safe(org_id) || !safe(actor_id) {
        return Err(ProviderError::InvalidInput("invalid artifact owner".into()));
    }
    Ok(())
}

fn validate_artifact_path(path: &str) -> ProviderResult<()> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\0')
        || path.split('/').any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(ProviderError::InvalidInput("artifact path must be a safe relative path".into()));
    }
    Ok(())
}

fn validate_owned_artifact_path(actor_id: &str, path: &str) -> ProviderResult<()> {
    validate_artifact_path(path)?;
    let prefix = format!("private/{actor_id}/sb/");
    if !path.starts_with(&prefix) || path.len() == prefix.len() {
        return Err(ProviderError::InvalidInput("artifact path is outside the OBO actor's private sb prefix".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant() -> OnBehalfOfGrant {
        OnBehalfOfGrant::new("short-lived-obo").unwrap()
    }

    #[tokio::test]
    async fn test_group_deferred_artifacts_never_claim_storage() {
        let store = DeferredArtifactStore::default();
        let receipt = store
            .put_on_behalf(PutArtifact {
                org_id: "tos".into(),
                actor_id: "worker:tos".into(),
                path: "private/worker:tos/sb/session/recording.mp4".into(),
                content_type: "video/mp4".into(),
                source: ArtifactSource::RemoteUrl("https://signed.invalid/secret".into()),
                obo: grant(),
            })
            .await
            .unwrap();
        assert_eq!(receipt.state, ArtifactState::Deferred);
        assert!(receipt.location.is_none());
        assert!(receipt.reason.is_some());
    }

    #[tokio::test]
    async fn test_group_deferred_artifacts_reject_path_traversal() {
        let store = DeferredArtifactStore::default();
        let result = store
            .trash_on_behalf(TrashArtifact {
                org_id: "tos".into(),
                actor_id: "worker:tos".into(),
                path: "private/worker:tos/../another-user/recording.mp4".into(),
                obo: grant(),
            })
            .await;
        assert!(matches!(result, Err(ProviderError::InvalidInput(_))));

        let result = store
            .trash_on_behalf(TrashArtifact {
                org_id: "tos".into(),
                actor_id: "worker:tos".into(),
                path: "private/another-worker/sb/session/recording.mp4".into(),
                obo: grant(),
            })
            .await;
        assert!(matches!(result, Err(ProviderError::InvalidInput(_))));
    }

    #[test]
    fn test_group_artifact_secrets_are_redacted_in_debug_output() {
        let grant = OnBehalfOfGrant::new("very-secret").unwrap();
        assert!(!format!("{grant:?}").contains("very-secret"));
        let source = ArtifactSource::RemoteUrl("https://example.test/?token=secret".into());
        assert!(!format!("{source:?}").contains("token=secret"));
    }
}
