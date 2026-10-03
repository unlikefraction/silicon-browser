//! Delivery orchestration: durable claims surround each external mutation.
use super::*;
use crate::delivery_auth::obo::ConsentComplete;
use crate::{
    delivery_auth::{DeliveryAuth, DeliveryAuthError},
    providers::{BriefcaseClient, BriefcaseEntry, BriefcaseUploadManifest, BriefcaseUploadState, OnBehalfOfGrant},
    recording_delivery::{DeliveryError, RecordingDelivery},
    store::{RecordingArtifactKind, RecordingDeliveryClaim},
};
use secrecy::ExposeSecret as _;
use silicon_browser_shared::{DeliveryAuthorization, DeliveryAuthorizationState};

const MAX_ATTEMPTS: u32 = 8;
const ITEM_BUDGET: Duration = Duration::from_secs(480);

pub(super) struct RecordingDeliveryServices {
    auth: DeliveryAuth,
    transfer: RecordingDelivery,
    issuer: String,
}

#[derive(Debug, thiserror::Error)]
enum DeliveryAttemptError {
    #[error(transparent)]
    Auth(#[from] DeliveryAuthError),
    #[error(transparent)]
    Source(#[from] DeliveryError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Store(#[from] StoreError),
}
pub(super) struct DeliveryBinding {
    pub principal_id: String,
    pub membership_id: String,
    pub destination_org: String,
    pub destination_actor: String,
}
struct DeliveryAccess {
    org_id: String,
    token: OnBehalfOfGrant,
    credential_version: String,
}
impl RecordingDeliveryServices {
    async fn access(
        &self,
        claim: &RecordingDeliveryClaim,
        endpoint: &str,
    ) -> Result<(BriefcaseClient, DeliveryAccess), DeliveryAttemptError> {
        let (principal, membership) = claim
            .principal_id
            .as_deref()
            .zip(claim.membership_id.as_deref())
            .ok_or(DeliveryAuthError::NeedsAuthorization)?;
        let tokens = self
            .auth
            .recording_tokens(
                &claim.org_id,
                &claim.actor_id,
                principal,
                membership,
                endpoint == "briefcase.uploads.commit",
            )
            .await?;
        if Some(tokens.org_id.as_str()) != claim.destination_org.as_deref()
            || Some(tokens.actor_id.as_str()) != claim.destination_actor.as_deref()
        {
            return Err(DeliveryAuthError::NeedsAuthorization.into());
        }
        let client =
            self.transfer.briefcase().with_testing_secret(tokens.testing_secret.as_ref().map(|s| s.expose_secret()))?;
        let token = match endpoint {
            "briefcase.uploads.reserve" => tokens.reserve,
            "briefcase.uploads.commit" => tokens.commit,
            "briefcase.uploads.status" => tokens.status,
            "briefcase.entries.list" => tokens.list,
            _ => return Err(DeliveryAuthError::NeedsAuthorization.into()),
        };
        Ok((client, DeliveryAccess { org_id: tokens.org_id, token, credential_version: tokens.credential_version }))
    }
    async fn checked<T>(
        &self,
        claim: &RecordingDeliveryClaim,
        access: &DeliveryAccess,
        result: Result<T, ProviderError>,
    ) -> Result<T, DeliveryAttemptError> {
        match result {
            Err(ProviderError::Http { status: 401, .. }) => {
                if let Some((principal, membership)) = claim.principal_id.as_deref().zip(claim.membership_id.as_deref())
                {
                    self.auth
                        .invalidate_storage(&claim.org_id, principal, membership, &access.credential_version)
                        .await?;
                }
                Err(DeliveryAuthError::NeedsAuthorization.into())
            }
            result => result.map_err(Into::into),
        }
    }
}

impl AppState {
    /// Require storage configuration before allowing a paid browser session.
    pub fn require_recording_delivery(mut self) -> Self {
        self.recording_delivery_required = true;
        self
    }
    pub fn with_recording_delivery(
        mut self,
        briefcase: BriefcaseClient,
        issuer: String,
        audience: String,
    ) -> Result<Self, String> {
        silicon_browser_shared::app_id(&issuer, "issuer").map_err(|error| error.to_string())?;
        silicon_browser_shared::app_id(&audience, "audience").map_err(|error| error.to_string())?;
        self.recording_delivery = Some(Arc::new(RecordingDeliveryServices {
            auth: DeliveryAuth::new(self.store.clone(), self.secrets.clone(), self.identity.clone(), audience.clone()),
            transfer: RecordingDelivery::new(briefcase, self.browser.clone()),
            issuer,
        }));
        Ok(self)
    }

    pub(super) async fn require_recording_authorization(
        &self,
        scope: &Scope,
    ) -> Result<Option<DeliveryBinding>, ApiFailure> {
        // Unconfigured AppState supports isolated tests and explicit partial deployments.
        let Some(delivery) = &self.recording_delivery else {
            return if self.recording_delivery_required {
                Err(ApiFailure::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "recording_delivery_unavailable",
                    "recording storage is not configured",
                ))
            } else {
                Ok(None)
            };
        };
        let (destination_org, destination_actor) = delivery
            .auth
            .storage_destination(
                &scope.org_id,
                &scope.identity.id,
                &scope.principal.principal_id,
                &scope.principal.membership_id,
                true,
            )
            .await?;
        let binding = DeliveryBinding {
            principal_id: scope.principal.principal_id.clone(),
            membership_id: scope.principal.membership_id.clone(),
            destination_org,
            destination_actor,
        };
        Ok(Some(binding))
    }

    /// One bounded pass, independent of browser stop and source-readiness loops.
    pub async fn deliver_recordings_once(&self) -> Result<usize, StoreError> {
        let Some(delivery) = &self.recording_delivery else {
            return Ok(0);
        };
        match tokio::time::timeout(Duration::from_secs(90), delivery.auth.recover_pending_once()).await {
            Ok(Ok(_)) => {}
            _ => tracing::warn!("recording authorization recovery will be retried"),
        }
        let now = Utc::now();
        let claims = self.store.claim_recording_deliveries(now, now + TimeDelta::seconds(600), 2).await?;
        let count = claims.len();
        let mut tasks = tokio::task::JoinSet::new();
        for claim in claims {
            let state = self.clone();
            tasks.spawn(async move {
                let retained = claim.clone();
                match tokio::time::timeout(ITEM_BUDGET, state.deliver_recording_artifact(claim)).await {
                    Ok(Ok(())) => {},
                    Ok(Err(_)) => tracing::error!(session_id = %retained.session_id, "recording delivery persistence failed; lease will recover"),
                    Err(_) => { let _ = state.retry_recording_delivery(&retained, "delivery_timeout", true).await; },
                }
            });
        }
        while let Some(result) = tasks.join_next().await {
            if result.is_err() {
                tracing::error!("recording delivery task failed; lease will recover");
            }
        }
        Ok(count)
    }

    async fn deliver_recording_artifact(&self, claim: RecordingDeliveryClaim) -> Result<(), StoreError> {
        let delivery = self.recording_delivery.as_ref().expect("worker requires configured delivery");
        if claim.attempt > MAX_ATTEMPTS {
            self.store.fail_recording_delivery(&claim, "delivery_attempts_exhausted", Utc::now()).await?;
            return Ok(());
        }
        let (Some(principal), Some(membership)) = (&claim.principal_id, &claim.membership_id) else {
            return self.retry_recording_delivery(&claim, "recording_owner_binding_missing", false).await;
        };
        if !delivery
            .auth
            .storage_binding(&claim.org_id, &claim.actor_id, principal, membership, false)
            .await
            .is_ok_and(|binding| binding.0 == *principal && binding.1 == *membership)
        {
            return self.retry_recording_delivery(&claim, "recording_authorization_required", false).await;
        }
        match self.upload_recording_artifact(&claim).await {
            Ok(Some(receipt)) => {
                self.store.complete_recording_delivery(&claim, &receipt, &self.secrets, Utc::now()).await?;
            }
            Ok(None) => {}
            Err(DeliveryAttemptError::Auth(
                DeliveryAuthError::NeedsAuthorization
                | DeliveryAuthError::Identity(IdentityError::Unauthenticated | IdentityError::Forbidden),
            )) => {
                self.retry_recording_delivery(&claim, "recording_authorization_required", false).await?;
            }
            Err(DeliveryAttemptError::Auth(DeliveryAuthError::Busy)) => {
                self.retry_recording_delivery(&claim, "recording_authorization_refreshing", false).await?;
            }
            Err(DeliveryAttemptError::Source(DeliveryError::Unavailable)) => {
                self.store.fail_recording_delivery(&claim, "native_recording_unavailable", Utc::now()).await?;
            }
            Err(DeliveryAttemptError::Source(DeliveryError::InvalidSource)) => {
                self.store.fail_recording_delivery(&claim, "recording_source_invalid", Utc::now()).await?;
            }
            Err(DeliveryAttemptError::Source(DeliveryError::TooLarge)) => {
                self.store.fail_recording_delivery(&claim, "recording_size_limit", Utc::now()).await?;
            }
            Err(DeliveryAttemptError::Store(error)) => return Err(error),
            Err(_) => {
                self.retry_recording_delivery(&claim, "briefcase_upload_unconfirmed", true).await?;
            }
        }
        Ok(())
    }

    async fn upload_recording_artifact(
        &self,
        claim: &RecordingDeliveryClaim,
    ) -> Result<Option<BriefcaseEntry>, DeliveryAttemptError> {
        let delivery = self.recording_delivery.as_ref().expect("configured delivery");
        let (client, access) = delivery.access(claim, "briefcase.uploads.status").await?;
        let mut status = match delivery
            .checked(
                claim,
                &access,
                client.upload_status(&access.org_id, &delivery.issuer, &access.token, claim.upload_operation_id).await,
            )
            .await
        {
            Ok(status) => Some(status),
            Err(DeliveryAttemptError::Provider(ProviderError::Http { status: 404, .. })) => None,
            Err(error) => return Err(error),
        };
        // Status comes first: an uncertain commit can already be complete even after
        // the browser's source URL has expired. Never reserve a new operation to retry it.
        let mut manifest =
            claim.body_sha256.as_ref().zip(claim.size_bytes).map(|(digest, size)| BriefcaseUploadManifest {
                operation_id: claim.upload_operation_id,
                parent_path: String::new(),
                name: match claim.kind {
                    RecordingArtifactKind::Video => format!("{}.mp4", claim.session_id),
                    RecordingArtifactKind::Commands => format!("{}-commands.jsonl", claim.session_id),
                },
                content_type: match claim.kind {
                    RecordingArtifactKind::Video => "video/mp4",
                    RecordingArtifactKind::Commands => "application/x-ndjson",
                }
                .into(),
                size,
                sha256: digest.clone(),
            });
        if status.as_ref().is_none_or(|s| s.state == BriefcaseUploadState::Reserved) {
            let artifact = match claim.kind {
                RecordingArtifactKind::Video => match &claim.provider_session_id {
                    Some(id) => delivery.transfer.prepare_video(&claim.session_id, id).await?,
                    None => return Err(DeliveryError::Unavailable.into()),
                },
                RecordingArtifactKind::Commands => {
                    delivery
                        .transfer
                        .prepare_log_pages(&claim.session_id, |after| {
                            let store = self.store.clone();
                            let claim = claim.clone();
                            let secrets = self.secrets.clone();
                            async move {
                                store
                                    .recording_delivery_logs(&claim, after, 128, &secrets)
                                    .await
                                    .map_err(|_| DeliveryError::Io)
                            }
                        })
                        .await?
                }
            };
            if !self.store.bind_recording_delivery(claim, &artifact.body_sha256, artifact.size, Utc::now()).await? {
                if self.store.recording_delivery_is_current(claim, Utc::now()).await? {
                    self.store.fail_recording_delivery(claim, "recording_source_changed", Utc::now()).await?;
                }
                return Ok(None);
            }
            manifest = Some(artifact.manifest(claim.upload_operation_id));
            if !self.store.begin_recording_upload(claim, Utc::now()).await? {
                return Ok(None);
            }
            let (client, access) = delivery.access(claim, "briefcase.uploads.reserve").await?;
            let reservation = delivery
                .checked(
                    claim,
                    &access,
                    client
                        .reserve_upload(
                            &access.org_id,
                            &delivery.issuer,
                            &access.token,
                            manifest.as_ref().expect("staged manifest"),
                        )
                        .await,
                )
                .await?;
            status = if reservation.status.state == BriefcaseUploadState::Reserved {
                Some(
                    client
                        .transfer_upload(
                            &access.org_id,
                            &reservation,
                            artifact.into_file(),
                            manifest.as_ref().expect("staged manifest").size,
                        )
                        .await?,
                )
            } else {
                Some(reservation.status)
            };
        }
        let Some(mut status) = status else {
            return Err(DeliveryAuthError::NeedsAuthorization.into());
        };
        let Some(manifest) = manifest else {
            return Err(DeliveryAuthError::NeedsAuthorization.into());
        };
        if status.state == BriefcaseUploadState::Staged {
            if !self.store.begin_recording_upload(claim, Utc::now()).await? {
                return Ok(None);
            }
            let (client, access) = delivery.access(claim, "briefcase.uploads.commit").await?;
            status = delivery
                .checked(
                    claim,
                    &access,
                    client
                        .commit_upload(
                            &access.org_id,
                            &delivery.issuer,
                            &access.token,
                            claim.upload_operation_id,
                            status.upload_id,
                        )
                        .await,
                )
                .await?;
        }
        if status.state == BriefcaseUploadState::Committed {
            let (client, access) = delivery.access(claim, "briefcase.entries.list").await?;
            let entry = delivery
                .checked(
                    claim,
                    &access,
                    client
                        .resolve_upload_entry(
                            &access.org_id,
                            &delivery.issuer,
                            &access.token,
                            &manifest,
                            status.published_entry_id.ok_or(DeliveryAuthError::NeedsAuthorization)?,
                        )
                        .await,
                )
                .await?;
            return Ok(Some(entry));
        }
        if matches!(status.state, BriefcaseUploadState::Cancelled | BriefcaseUploadState::Expired) {
            self.store.fail_recording_delivery(claim, "briefcase_upload_expired", Utc::now()).await?;
        } else {
            self.retry_recording_delivery(claim, "briefcase_upload_unconfirmed", true).await?;
        }
        Ok(None)
    }

    async fn retry_recording_delivery(
        &self,
        claim: &RecordingDeliveryClaim,
        reason: &'static str,
        count: bool,
    ) -> Result<(), StoreError> {
        if count && claim.attempt >= MAX_ATTEMPTS {
            self.store.fail_recording_delivery(claim, reason, Utc::now()).await?;
        } else {
            let seconds = if count { 15_i64 * (1_i64 << claim.attempt.saturating_sub(1).min(5)) } else { 60 };
            self.store.defer_recording_delivery(claim, Utc::now() + TimeDelta::seconds(seconds), reason, count).await?;
        }
        Ok(())
    }
}

pub(super) async fn authorization_status(
    State(state): State<AppState>,
    scope: Scope,
) -> Result<impl IntoResponse, ApiFailure> {
    let status = match &state.recording_delivery {
        Some(delivery) => delivery.auth.storage_status(&scope.org_id, &scope.principal).await?,
        None => DeliveryAuthorization {
            configured: false,
            enabled: false,
            state: DeliveryAuthorizationState::Unavailable,
            actor_id: scope.identity.id,
        },
    };
    Ok(success(status))
}

pub(super) async fn authorize() -> ApiFailure {
    ApiFailure::new(
        StatusCode::GONE,
        "feature_consent_required",
        "Recording access requires separate Briefcase approval; start a delivery authorization.",
    )
}

pub(super) async fn start_consent(
    State(state): State<AppState>,
    scope: Scope,
    Bearer(bearer): Bearer,
    headers: HeaderMap,
    payload: Result<Json<serde_json::Value>, JsonRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let payload = json_payload(payload)?;
    let popup = payload == serde_json::json!({"popup":true});
    if payload != serde_json::json!({}) && !popup {
        return Err(ApiFailure::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Expected an empty object or popup: true.",
        ));
    }
    let callback = popup.then(|| format!("{}/auth/obo/callback", state.public_origin.trim_end_matches('/')));
    let key = required_header(&headers, "idempotency-key", "idempotency key is required")?;
    let delivery = state.recording_delivery.as_ref().ok_or_else(|| {
        ApiFailure::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "recording_delivery_unavailable",
            "Recording storage is not configured.",
        )
    })?;
    Ok((
        [(http::header::CACHE_CONTROL, "no-store")],
        success(
            delivery.auth.start_consent(&scope.org_id, &scope.principal, &bearer, &key, callback.as_deref()).await?,
        ),
    ))
}
pub(super) async fn consent_status(
    State(state): State<AppState>,
    scope: Scope,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiFailure> {
    let delivery = state.recording_delivery.as_ref().ok_or_else(|| {
        ApiFailure::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "recording_delivery_unavailable",
            "Recording storage is not configured.",
        )
    })?;
    Ok((
        [(http::header::CACHE_CONTROL, "no-store")],
        success(delivery.auth.consent_status(&scope.org_id, &scope.principal, &id).await?),
    ))
}
pub(super) async fn complete_consent(
    State(state): State<AppState>,
    scope: Scope,
    Path(id): Path<String>,
    payload: Result<Json<ConsentComplete>, JsonRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let delivery = state.recording_delivery.as_ref().ok_or_else(|| {
        ApiFailure::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "recording_delivery_unavailable",
            "Recording storage is not configured.",
        )
    })?;
    let response = delivery.auth.complete_consent(&scope.org_id, &scope.principal, &id, json_payload(payload)?).await?;
    state.store.wake_recording_deliveries(&scope.org_id, &scope.identity.id, Utc::now()).await?;
    Ok(([(http::header::CACHE_CONTROL, "no-store")], success(response)))
}

pub(super) async fn disable_authorization(
    State(state): State<AppState>,
    scope: Scope,
) -> Result<impl IntoResponse, ApiFailure> {
    let delivery = state.recording_delivery.as_ref().ok_or_else(|| {
        ApiFailure::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "recording_delivery_unavailable",
            "recording delivery is not configured",
        )
    })?;
    Ok(success(delivery.auth.disable_storage(&scope.org_id, &scope.principal).await?))
}

pub(super) async fn retry_recording(
    State(state): State<AppState>,
    scope: Scope,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ApiFailure> {
    // Visibility does not grant authority to write into the initiator's Briefcase.
    let recording = state.store.recording(&scope.org_id, &scope.identity, &session_id, &state.secrets).await?;
    if recording.owner_id != scope.identity.id {
        return Err(ApiFailure::new(
            StatusCode::FORBIDDEN,
            "recording_owner_required",
            "only the initiating identity can retry delivery",
        ));
    }
    let Some(binding) = state.require_recording_authorization(&scope).await? else {
        return Err(ApiFailure::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "recording_delivery_unavailable",
            "recording storage is not configured",
        ));
    };
    if !state
        .store
        .retry_failed_recording_delivery(
            &scope.org_id,
            &session_id,
            &scope.identity.id,
            &binding.principal_id,
            &binding.membership_id,
            Utc::now(),
        )
        .await?
    {
        return Err(ApiFailure::conflict(
            "recording_not_retryable",
            "no retryable delivery exists for this initiating membership",
        ));
    }
    Ok(success(state.store.recording(&scope.org_id, &scope.identity, &session_id, &state.secrets).await?))
}

impl From<DeliveryAuthError> for ApiFailure {
    fn from(value: DeliveryAuthError) -> Self {
        match value {
            DeliveryAuthError::Identity(error) => Self::from(error),
            DeliveryAuthError::InvalidConsent => Self::new(
                StatusCode::BAD_REQUEST,
                "invalid_recording_consent",
                "The approval code or state is invalid. Your sign-in remains active.",
            ),
            DeliveryAuthError::NeedsAuthorization => Self::conflict(
                "recording_authorization_required",
                "Reconnect recording access to save your sessions in Briefcase.",
            ),
            DeliveryAuthError::Busy => Self::conflict(
                "recording_authorization_busy",
                "recording authorization is being updated; retry shortly",
            ),
            DeliveryAuthError::Storage => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "recording_authorization_unavailable",
                "recording authorization storage is unavailable",
            ),
        }
    }
}
