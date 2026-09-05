//! Delivery orchestration: durable claims surround each external mutation.
use super::*;
use crate::{
    auth::RecordingProofRequest,
    delivery_auth::{DeliveryAuth, DeliveryAuthError},
    providers::BriefcaseClient,
    recording_delivery::{DeliveryError, RecordingDelivery},
    store::{RecordingArtifactKind, RecordingDeliveryClaim},
};
use silicon_browser_shared::{DeliveryAuthorization, DeliveryAuthorizationRequest, DeliveryAuthorizationState};

const MAX_ATTEMPTS: u32 = 8;
const ITEM_BUDGET: Duration = Duration::from_secs(480);

pub(super) struct RecordingDeliveryServices {
    auth: DeliveryAuth,
    transfer: RecordingDelivery,
    issuer: String,
    audience: String,
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
        let valid = |id: &str| {
            id.split_once('>').is_some_and(|(org, app)| {
                !org.is_empty() && !app.is_empty() && !app.contains('>') && !id.chars().any(char::is_whitespace)
            })
        };
        if !valid(&issuer)
            || !valid(&audience)
            || issuer.split_once('>').map(|p| p.0) != audience.split_once('>').map(|p| p.0)
        {
            return Err(
                "recording issuer and Briefcase audience must be canonical applications in the same organization"
                    .into(),
            );
        }
        self.recording_delivery = Some(Arc::new(RecordingDeliveryServices {
            auth: DeliveryAuth::new(self.store.clone(), self.secrets.clone(), self.identity.clone()),
            transfer: RecordingDelivery::new(briefcase, self.browser.clone()),
            issuer,
            audience,
        }));
        Ok(self)
    }

    pub(super) async fn require_recording_authorization(
        &self,
        scope: &Scope,
    ) -> Result<Option<(String, String)>, ApiFailure> {
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
        if delivery.issuer.split_once('>').map(|p| p.0) != Some(scope.org_id.as_str()) {
            return Err(ApiFailure::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "recording_delivery_unavailable",
                "recording storage is not configured for this organization",
            ));
        }
        let binding = delivery
            .auth
            .validate_authorized_binding_for_principal(
                &scope.org_id,
                &scope.identity.id,
                &scope.principal.principal_id.to_string(),
                &scope.principal.membership_id.to_string(),
            )
            .await?;
        if binding.0 != scope.principal.principal_id.to_string()
            || binding.1 != scope.principal.membership_id.to_string()
        {
            return Err(ApiFailure::conflict(
                "recording_authorization_required",
                "recording delivery needs a fresh SLT for this membership",
            ));
        }
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
            .authorized_binding_for_principal(&claim.org_id, &claim.actor_id, principal, membership)
            .await
            .is_ok_and(|binding| binding.0 == *principal && binding.1 == *membership)
        {
            return self.retry_recording_delivery(&claim, "recording_authorization_required", false).await;
        }
        let staged = match claim.kind {
            RecordingArtifactKind::Video => match &claim.provider_session_id {
                Some(id) => delivery.transfer.prepare_video(&claim.session_id, id).await,
                None => Err(DeliveryError::Unavailable),
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
                    .await
            }
        };
        let artifact = match staged {
            Ok(value) => value,
            Err(DeliveryError::Unavailable) => {
                self.store.fail_recording_delivery(&claim, "native_recording_unavailable", Utc::now()).await?;
                return Ok(());
            }
            Err(DeliveryError::InvalidSource) => {
                self.store.fail_recording_delivery(&claim, "recording_source_invalid", Utc::now()).await?;
                return Ok(());
            }
            Err(DeliveryError::TooLarge) => {
                self.store.fail_recording_delivery(&claim, "recording_size_limit", Utc::now()).await?;
                return Ok(());
            }
            Err(_) => return self.retry_recording_delivery(&claim, "recording_source_unavailable", true).await,
        };
        if !self.store.bind_recording_delivery(&claim, &artifact.body_sha256, artifact.size, Utc::now()).await? {
            if self.store.recording_delivery_is_current(&claim, Utc::now()).await? {
                self.store.fail_recording_delivery(&claim, "recording_source_changed", Utc::now()).await?;
            }
            return Ok(());
        }
        let request = RecordingProofRequest {
            expected_org_id: claim.org_id.clone(),
            expected_actor_id: claim.actor_id.clone(),
            audience: delivery.audience.clone(),
            path: String::new(),
            name: artifact.name.clone(),
            content_type: artifact.content_type.into(),
            body_sha256: artifact.body_sha256.clone(),
            idempotency_key: format!("recording-{}-{}-{}", claim.session_id, claim.lease_id, claim.kind.as_str()),
        };
        let proof = match delivery.auth.issue_recording_proof_for_principal(principal, membership, request).await {
            Ok(proof) => proof,
            Err(DeliveryAuthError::NeedsAuthorization)
            | Err(DeliveryAuthError::Identity(IdentityError::Unauthenticated | IdentityError::Forbidden)) => {
                return self.retry_recording_delivery(&claim, "recording_authorization_required", false).await;
            }
            Err(DeliveryAuthError::Busy) => {
                return self.retry_recording_delivery(&claim, "recording_authorization_refreshing", false).await;
            }
            Err(_) => return self.retry_recording_delivery(&claim, "recording_proof_unavailable", true).await,
        };
        if proof.expires_at <= Utc::now() + TimeDelta::seconds(5) {
            return self.retry_recording_delivery(&claim, "recording_proof_expired", true).await;
        }
        // Cancellation/lease checks immediately precede the single network mutation.
        if !delivery
            .auth
            .authorized_binding_for_principal(&claim.org_id, &claim.actor_id, principal, membership)
            .await
            .is_ok_and(|binding| binding.0 == *principal && binding.1 == *membership)
        {
            return self.retry_recording_delivery(&claim, "recording_authorization_required", false).await;
        }
        if !self.store.begin_recording_upload(&claim, Utc::now()).await? {
            return Ok(());
        }
        match delivery.transfer.upload(artifact, &proof.grant, &claim.org_id, &delivery.issuer).await {
            Ok(receipt) => {
                self.store.complete_recording_delivery(&claim, &receipt, &self.secrets, Utc::now()).await?;
            }
            Err(_) => self.retry_recording_delivery(&claim, "briefcase_upload_unconfirmed", true).await?,
        }
        Ok(())
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
        Some(delivery) => delivery.auth.live_status_for_principal(&scope.org_id, &scope.principal).await?,
        None => DeliveryAuthorization {
            configured: false,
            enabled: false,
            state: DeliveryAuthorizationState::Unavailable,
            actor_id: scope.identity.id,
        },
    };
    Ok(success(status))
}

pub(super) async fn authorize(
    State(state): State<AppState>,
    scope: Scope,
    payload: Result<Json<DeliveryAuthorizationRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let request = json_payload(payload)?;
    request.validate().map_err(ApiFailure::validation)?;
    let delivery = state.recording_delivery.as_ref().ok_or_else(|| {
        ApiFailure::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "recording_delivery_unavailable",
            "recording delivery is not configured",
        )
    })?;
    let authorization = delivery.auth.enroll(&scope.org_id, &scope.principal, &request.short_lived_token).await?;
    state.store.wake_recording_deliveries(&scope.org_id, &scope.identity.id, Utc::now()).await?;
    Ok(success(authorization))
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
    Ok(success(delivery.auth.disable_for_principal(&scope.org_id, &scope.principal).await?))
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
    let Some((principal, membership)) = state.require_recording_authorization(&scope).await? else {
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
            &principal,
            &membership,
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
