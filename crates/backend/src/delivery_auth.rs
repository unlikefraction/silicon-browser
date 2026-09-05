//! Backend-owned IAM families for recording delivery. CLI refresh tokens never enter this store.
use crate::{
    auth::{
        DeliveryTokenExchange, ExchangeRequest, IdentityError, IdentityProvider, PrincipalIdentity, RecordingProof,
        RecordingProofRequest, RefreshRequest,
    },
    crypto::SecretBox,
    store::Store,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use silicon_browser_shared::{DeliveryAuthorization, DeliveryAuthorizationState as State};
use sqlx::FromRow;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum DeliveryAuthError {
    #[error(transparent)]
    Identity(#[from] IdentityError),
    #[error("delivery authorization storage is unavailable")]
    Storage,
    #[error("delivery authorization transition is in progress")]
    Busy,
    #[error("recording delivery requires fresh authorization")]
    NeedsAuthorization,
}
type Result<T> = std::result::Result<T, DeliveryAuthError>;

#[derive(Clone)]
pub struct DeliveryAuth {
    store: Store,
    secrets: SecretBox,
    identity: Arc<dyn IdentityProvider>,
}
#[derive(FromRow)]
struct Grant {
    id: String,
    org_id: String,
    actor_id: String,
    principal_id: String,
    membership_id: String,
    actor_kind: String,
    enabled: bool,
    state: String,
    operation: Option<String>,
    mutation_key: Option<String>,
    encrypted_payload: String,
    access_expires_at: i64,
}
#[derive(Default, Serialize, Deserialize)]
struct Credentials {
    slt: Option<String>,
    access: Option<String>,
    refresh: Option<String>,
}

impl DeliveryAuth {
    pub fn new(store: Store, secrets: SecretBox, identity: Arc<dyn IdentityProvider>) -> Self {
        Self { store, secrets, identity }
    }
    fn context(row: &Grant) -> String {
        format!(
            "delivery/{}/{}/{}/{}/{}/credentials",
            row.org_id, row.actor_id, row.principal_id, row.membership_id, row.id
        )
    }
    fn open(&self, row: &Grant) -> Result<Credentials> {
        let raw = self
            .secrets
            .open_for(&Self::context(row), &row.encrypted_payload)
            .map_err(|_| DeliveryAuthError::Storage)?;
        serde_json::from_str(&raw).map_err(|_| DeliveryAuthError::Storage)
    }
    fn seal(&self, row: &Grant, value: &Credentials) -> Result<String> {
        self.secrets
            .seal_for(&Self::context(row), &serde_json::to_string(value).map_err(|_| DeliveryAuthError::Storage)?)
            .map_err(|_| DeliveryAuthError::Storage)
    }
    async fn get(&self, id: &str) -> Result<Grant> {
        sqlx::query_as("SELECT * FROM delivery_credentials WHERE id=?")
            .bind(id)
            .fetch_one(self.store.pool())
            .await
            .map_err(|_| DeliveryAuthError::Storage)
    }
    fn public(row: Option<&Grant>, actor: &str) -> DeliveryAuthorization {
        let state = match row.map(|r| r.state.as_str()) {
            Some("active") => State::Active,
            Some("refreshing") => State::Refreshing,
            Some("pending") => State::Pending,
            Some("needs_auth") => State::NeedsAuth,
            Some("revoking") => State::Revoking,
            _ => State::Disabled,
        };
        DeliveryAuthorization {
            configured: true,
            enabled: row.is_some_and(|r| r.enabled),
            state,
            actor_id: actor.into(),
        }
    }
    pub async fn status(&self, org: &str, actor: &str) -> Result<DeliveryAuthorization> {
        let row:Option<Grant>=sqlx::query_as("SELECT * FROM delivery_credentials WHERE org_id=? AND actor_id=? ORDER BY enabled DESC,created_at DESC,rowid DESC LIMIT 1")
            .bind(org).bind(actor).fetch_optional(self.store.pool()).await.map_err(|_|DeliveryAuthError::Storage)?;
        Ok(Self::public(row.as_ref(), actor))
    }
    pub async fn status_for_principal(&self, org: &str, expected: &PrincipalIdentity) -> Result<DeliveryAuthorization> {
        let actor = expected.public_id.as_deref().ok_or(IdentityError::Forbidden)?;
        if expected.org_id != org {
            return Err(IdentityError::Forbidden.into());
        }
        let row:Option<Grant>=sqlx::query_as("SELECT * FROM delivery_credentials WHERE org_id=? AND principal_id=? ORDER BY enabled DESC,created_at DESC,rowid DESC LIMIT 1")
            .bind(org).bind(expected.principal_id.to_string()).fetch_optional(self.store.pool()).await.map_err(|_|DeliveryAuthError::Storage)?;
        if row.as_ref().is_some_and(|row| !Self::matches(row, expected)) {
            return Ok(DeliveryAuthorization {
                configured: true,
                enabled: false,
                state: State::NeedsAuth,
                actor_id: actor.into(),
            });
        }
        Ok(Self::public(row.as_ref(), actor))
    }

    pub async fn disable_for_principal(
        &self,
        org: &str,
        expected: &PrincipalIdentity,
    ) -> Result<DeliveryAuthorization> {
        let actor = expected.public_id.as_deref().ok_or(IdentityError::Forbidden)?;
        if expected.org_id != org {
            return Err(IdentityError::Forbidden.into());
        }
        sqlx::query("UPDATE delivery_credentials SET enabled=0,state='revoking',operation=COALESCE(operation,'revoke'),mutation_key=COALESCE(mutation_key,?),updated_at=? WHERE org_id=? AND actor_id=? AND principal_id=? AND membership_id=? AND enabled=1")
            .bind(Uuid::new_v4().to_string()).bind(Utc::now().timestamp()).bind(org).bind(actor).bind(expected.principal_id.to_string()).bind(expected.membership_id.to_string()).execute(self.store.pool()).await.map_err(|_|DeliveryAuthError::Storage)?;
        self.status_for_principal(org, expected).await
    }

    pub async fn enroll(&self, org: &str, expected: &PrincipalIdentity, slt: &str) -> Result<DeliveryAuthorization> {
        use silicon_browser_shared::Validate;
        silicon_browser_shared::DeliveryAuthorizationRequest { short_lived_token: slt.into() }
            .validate()
            .map_err(|_| IdentityError::InvalidInput { field: "short_lived_token", reason: "invalid IAM SLT" })?;
        let actor = expected.public_id.as_deref().filter(|v| !v.is_empty()).ok_or(IdentityError::Forbidden)?;
        if expected.org_id != org
            || expected.principal_id.is_nil()
            || expected.membership_id.is_nil()
            || expected.expires_at <= Utc::now()
        {
            return Err(IdentityError::Forbidden.into());
        }
        let digest = hex::encode(Sha256::digest(slt.as_bytes()));
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT id FROM delivery_credentials WHERE org_id=? AND actor_id=? AND enrollment_digest=?",
        )
        .bind(org)
        .bind(actor)
        .bind(&digest)
        .fetch_optional(self.store.pool())
        .await
        .map_err(|_| DeliveryAuthError::Storage)?;
        let id = if let Some(id) = existing {
            id
        } else {
            let now = Utc::now().timestamp();
            let id = Uuid::now_v7().to_string();
            let row = Grant {
                id: id.clone(),
                org_id: org.into(),
                actor_id: actor.into(),
                principal_id: expected.principal_id.to_string(),
                membership_id: expected.membership_id.to_string(),
                actor_kind: serde_json::to_string(&expected.kind).map_err(|_| DeliveryAuthError::Storage)?,
                enabled: true,
                state: "pending".into(),
                operation: Some("exchange".into()),
                mutation_key: Some(Uuid::new_v4().to_string()),
                encrypted_payload: String::new(),
                access_expires_at: 0,
            };
            let sealed = self.seal(&row, &Credentials { slt: Some(slt.into()), ..Default::default() })?;
            let mut tx = self.store.pool().begin().await.map_err(|_| DeliveryAuthError::Storage)?;
            // Superseded families are revoked, including exchanges that were already in flight.
            sqlx::query("UPDATE delivery_credentials SET enabled=0,state='revoking',operation=COALESCE(operation,'revoke'),mutation_key=COALESCE(mutation_key,?),updated_at=? WHERE org_id=? AND principal_id=? AND enabled=1")
                .bind(Uuid::new_v4().to_string()).bind(now).bind(org).bind(expected.principal_id.to_string()).execute(&mut *tx).await.map_err(|_|DeliveryAuthError::Storage)?;
            sqlx::query("INSERT INTO delivery_credentials(id,org_id,actor_id,principal_id,membership_id,actor_kind,enabled,state,operation,mutation_key,enrollment_digest,encrypted_payload,created_at,updated_at) VALUES(?,?,?,?,?,?,1,'pending','exchange',?,?,?,?,?)")
                .bind(&id).bind(org).bind(actor).bind(&row.principal_id).bind(&row.membership_id).bind(&row.actor_kind).bind(&row.mutation_key).bind(&digest).bind(sealed).bind(now).bind(now).execute(&mut *tx).await.map_err(|_|DeliveryAuthError::Storage)?;
            tx.commit().await.map_err(|_| DeliveryAuthError::Storage)?;
            id
        };
        let row = self.get(&id).await?;
        if !Self::matches(&row, expected) {
            return Err(IdentityError::Forbidden.into());
        }
        if row.operation.is_some() {
            self.process(&id).await?;
        }
        Ok(Self::public(Some(&self.get(&id).await?), actor))
    }
    pub async fn disable(&self, org: &str, actor: &str) -> Result<DeliveryAuthorization> {
        // Compatibility only: never broaden a name lookup across multiple owners.
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM delivery_credentials WHERE org_id=? AND actor_id=? AND enabled=1 LIMIT 2",
        )
        .bind(org)
        .bind(actor)
        .fetch_all(self.store.pool())
        .await
        .map_err(|_| DeliveryAuthError::Storage)?;
        if ids.len() > 1 {
            return Err(DeliveryAuthError::NeedsAuthorization);
        }
        if let Some(id) = ids.first() {
            sqlx::query("UPDATE delivery_credentials SET enabled=0,state='revoking',operation=COALESCE(operation,'revoke'),mutation_key=COALESCE(mutation_key,?),updated_at=? WHERE id=? AND enabled=1")
                .bind(Uuid::new_v4().to_string()).bind(Utc::now().timestamp()).bind(id).execute(self.store.pool()).await.map_err(|_|DeliveryAuthError::Storage)?;
        }
        self.status(org, actor).await
    }
    fn matches(row: &Grant, p: &PrincipalIdentity) -> bool {
        row.org_id == p.org_id
            && p.public_id.as_deref() == Some(&row.actor_id)
            && row.principal_id == p.principal_id.to_string()
            && row.membership_id == p.membership_id.to_string()
            && serde_json::to_string(&p.kind).ok().as_deref() == Some(&row.actor_kind)
    }
    pub async fn authorized_binding(&self, org: &str, actor: &str) -> Result<(String, String)> {
        let rows:Vec<(String,String)>=sqlx::query_as("SELECT principal_id,membership_id FROM delivery_credentials WHERE org_id=? AND actor_id=? AND enabled=1 AND state IN ('active','refreshing') LIMIT 2")
            .bind(org).bind(actor).fetch_all(self.store.pool()).await.map_err(|_|DeliveryAuthError::Storage)?;
        if rows.len() != 1 {
            return Err(DeliveryAuthError::NeedsAuthorization);
        }
        Ok(rows.into_iter().next().expect("one grant checked above"))
    }

    pub async fn authorized_binding_for_principal(
        &self,
        org: &str,
        actor: &str,
        principal: &str,
        membership: &str,
    ) -> Result<(String, String)> {
        let row:Option<(String,String)>=sqlx::query_as("SELECT principal_id,membership_id FROM delivery_credentials WHERE org_id=? AND actor_id=? AND principal_id=? AND membership_id=? AND enabled=1 AND state IN ('active','refreshing')")
            .bind(org).bind(actor).bind(principal).bind(membership).fetch_optional(self.store.pool()).await.map_err(|_|DeliveryAuthError::Storage)?;
        row.ok_or(DeliveryAuthError::NeedsAuthorization)
    }

    pub async fn issue_recording_proof_for_principal(
        &self,
        principal_id: &str,
        membership_id: &str,
        request: RecordingProofRequest,
    ) -> Result<RecordingProof> {
        self.issue_bound_proof(request, Some((principal_id, membership_id))).await
    }

    /// For an immediate current-actor operation. Durable session jobs must use
    /// `issue_recording_proof_for_principal` with their persisted IAM binding.
    pub async fn issue_recording_proof(&self, request: RecordingProofRequest) -> Result<RecordingProof> {
        self.issue_bound_proof(request, None).await
    }

    async fn issue_bound_proof(
        &self,
        request: RecordingProofRequest,
        expected: Option<(&str, &str)>,
    ) -> Result<RecordingProof> {
        let rows:Vec<Grant>=sqlx::query_as("SELECT * FROM delivery_credentials WHERE org_id=? AND actor_id=? AND enabled=1 AND (? IS NULL OR principal_id=?) AND (? IS NULL OR membership_id=?) LIMIT 2")
            .bind(&request.expected_org_id).bind(&request.expected_actor_id)
            .bind(expected.map(|v|v.0)).bind(expected.map(|v|v.0)).bind(expected.map(|v|v.1)).bind(expected.map(|v|v.1))
            .fetch_all(self.store.pool()).await.map_err(|_|DeliveryAuthError::Storage)?;
        if rows.len() != 1 {
            return Err(DeliveryAuthError::NeedsAuthorization);
        }
        let mut row = rows.into_iter().next().expect("one grant checked above");
        if expected
            .is_some_and(|(principal, membership)| row.principal_id != principal || row.membership_id != membership)
        {
            return Err(DeliveryAuthError::NeedsAuthorization);
        }
        if row.state == "needs_auth" {
            return Err(DeliveryAuthError::NeedsAuthorization);
        }
        if row.operation.is_none() && row.access_expires_at <= Utc::now().timestamp() + 90 {
            sqlx::query("UPDATE delivery_credentials SET operation='refresh',state='refreshing',mutation_key=?,updated_at=? WHERE id=? AND enabled=1 AND operation IS NULL")
                .bind(Uuid::new_v4().to_string()).bind(Utc::now().timestamp()).bind(&row.id).execute(self.store.pool()).await.map_err(|_|DeliveryAuthError::Storage)?;
        }
        row = self.get(&row.id).await?;
        if row.operation.is_some() {
            self.process(&row.id).await?;
            row = self.get(&row.id).await?;
            if row.enabled && row.operation.as_deref() == Some("refresh") {
                // A late replay recovered its new ORT; commit that credential
                // first, then rotate it under a separately persisted mutation.
                self.process(&row.id).await?;
                row = self.get(&row.id).await?;
            }
        }
        if !row.enabled || row.state != "active" {
            return Err(DeliveryAuthError::NeedsAuthorization);
        }
        for attempt in 0..2 {
            let credentials = self.open(&row)?;
            let token = credentials.access.as_deref().ok_or(DeliveryAuthError::NeedsAuthorization)?;
            let result = async {
                let current = self.identity.identify(token, &row.org_id).await?;
                if !Self::matches(&row, &current) {
                    return Err(IdentityError::Forbidden);
                }
                self.identity.issue_recording_proof(token, request.clone()).await
            }
            .await;
            match result {
                Ok(proof) => {
                    // A disable racing network work must not hand a new proof to the worker.
                    if !self.get(&row.id).await?.enabled {
                        return Err(DeliveryAuthError::NeedsAuthorization);
                    }
                    return Ok(proof);
                }
                Err(IdentityError::Unauthenticated) if attempt == 0 => {
                    // IAM may revoke sibling-family OATs while leaving this owned ORT
                    // valid. Persist the refresh intent before rotating; never re-enroll.
                    self.observe_failure(&row, &IdentityError::Unauthenticated).await?;
                    row = self.get(&row.id).await?;
                    if !row.enabled {
                        return Err(DeliveryAuthError::NeedsAuthorization);
                    }
                    if row.operation.as_deref() == Some("refresh") {
                        self.process(&row.id).await?;
                        row = self.get(&row.id).await?;
                    }
                    if !row.enabled || row.state != "active" || row.operation.is_some() {
                        return Err(DeliveryAuthError::NeedsAuthorization);
                    }
                }
                Err(error) => {
                    // A second rejection after refresh must not create a refresh loop.
                    let observed = if matches!(error, IdentityError::Unauthenticated) {
                        &IdentityError::Forbidden
                    } else {
                        &error
                    };
                    self.observe_failure(&row, observed).await?;
                    return Err(error.into());
                }
            }
        }
        Err(DeliveryAuthError::NeedsAuthorization)
    }
    async fn observe_failure(&self, row: &Grant, error: &IdentityError) -> Result<()> {
        if matches!(error, IdentityError::Unauthenticated) {
            sqlx::query("UPDATE delivery_credentials SET state='refreshing',operation='refresh',mutation_key=?,updated_at=? WHERE id=? AND encrypted_payload=? AND enabled=1 AND operation IS NULL")
                .bind(Uuid::new_v4().to_string()).bind(Utc::now().timestamp()).bind(&row.id).bind(&row.encrypted_payload).execute(self.store.pool()).await.map_err(|_| DeliveryAuthError::Storage)?;
        } else if matches!(error, IdentityError::Forbidden) {
            sqlx::query("UPDATE delivery_credentials SET state='needs_auth',updated_at=? WHERE id=? AND encrypted_payload=? AND enabled=1 AND operation IS NULL")
                .bind(Utc::now().timestamp()).bind(&row.id).bind(&row.encrypted_payload).execute(self.store.pool()).await.map_err(|_| DeliveryAuthError::Storage)?;
        }
        Ok(())
    }

    pub async fn recover_pending_once(&self) -> Result<usize> {
        let ids:Vec<String>=sqlx::query_scalar("SELECT id FROM delivery_credentials WHERE operation IS NOT NULL AND lease_until<=? ORDER BY updated_at LIMIT 16")
            .bind(Utc::now().timestamp()).fetch_all(self.store.pool()).await.map_err(|_|DeliveryAuthError::Storage)?;
        let mut completed = 0;
        for id in ids {
            match self.process(&id).await {
                Ok(()) => completed += 1,
                Err(DeliveryAuthError::Storage) => return Err(DeliveryAuthError::Storage),
                Err(_) => {}
            }
        }
        Ok(completed)
    }
    async fn process(&self, id: &str) -> Result<()> {
        let now = Utc::now().timestamp();
        let owner = Uuid::new_v4().to_string();
        let changed=sqlx::query("UPDATE delivery_credentials SET lease_owner=?,lease_until=? WHERE id=? AND operation IS NOT NULL AND lease_until<=?")
            .bind(&owner).bind(now+90).bind(id).bind(now).execute(self.store.pool()).await.map_err(|_|DeliveryAuthError::Storage)?.rows_affected();
        if changed == 0 {
            return Err(DeliveryAuthError::Busy);
        }
        let row = self.get(id).await?;
        let credentials = self.open(&row)?;
        let key = row.mutation_key.as_deref().ok_or(DeliveryAuthError::Storage)?;
        let operation = row.operation.as_deref().ok_or(DeliveryAuthError::Storage)?;
        let result: std::result::Result<Option<DeliveryTokenExchange>, IdentityError> = match operation {
            "exchange" => self
                .identity
                .exchange_delivery_token(ExchangeRequest {
                    short_lived_token: credentials.slt.clone().ok_or(DeliveryAuthError::Storage)?,
                    required_org_id: row.org_id.clone(),
                    idempotency_key: key.into(),
                })
                .await
                .map(Some),
            "refresh" => self
                .identity
                .refresh_delivery_token(RefreshRequest {
                    refresh_token: credentials.refresh.clone().ok_or(DeliveryAuthError::Storage)?,
                    required_org_id: row.org_id.clone(),
                    idempotency_key: key.into(),
                })
                .await
                .map(Some),
            "revoke" => match credentials.refresh.as_deref() {
                Some(token) => self.identity.revoke_application_token(token, key).await.map(|_| None),
                None => Ok(None),
            },
            _ => return Err(DeliveryAuthError::Storage),
        };
        match result {
            Ok(Some(result)) => {
                let auth = result.auth;
                let active = result.access_active;
                let bound = Self::matches(&row, &auth.identity)
                    && auth.identity.expires_at > Utc::now()
                    && ["obo.issue", "memberships.read", "roles.read"]
                        .iter()
                        .all(|s| auth.scope.split_whitespace().any(|v| v == *s));
                let payload = self.seal(
                    &row,
                    &Credentials { slt: None, access: Some(auth.access_token), refresh: Some(auth.refresh_token) },
                )?;
                // SQL reads current enabled, not the pre-network snapshot, to honor concurrent disable.
                sqlx::query("UPDATE delivery_credentials SET encrypted_payload=?,access_expires_at=?,enabled=CASE WHEN ? THEN enabled ELSE 0 END,state=CASE WHEN enabled=1 AND ? THEN ? ELSE 'revoking' END,operation=CASE WHEN enabled=1 AND ? THEN ? ELSE 'revoke' END,mutation_key=CASE WHEN enabled=1 AND ? AND ? THEN NULL ELSE ? END,lease_owner=NULL,lease_until=0,updated_at=? WHERE id=? AND lease_owner=?")
                    .bind(payload).bind(if active {auth.identity.expires_at.timestamp()} else {0}).bind(bound).bind(bound).bind(if active {"active"} else {"refreshing"}).bind(bound).bind(if active {None} else {Some("refresh")}).bind(bound).bind(active).bind(Uuid::new_v4().to_string()).bind(Utc::now().timestamp()).bind(id).bind(&owner).execute(self.store.pool()).await.map_err(|_|DeliveryAuthError::Storage)?;
                if !bound {
                    return Err(IdentityError::Forbidden.into());
                }
            }
            Ok(None) => {
                let empty = self.seal(&row, &Credentials::default())?;
                sqlx::query("UPDATE delivery_credentials SET encrypted_payload=?,enabled=0,state='disabled',operation=NULL,mutation_key=NULL,lease_owner=NULL,lease_until=0,updated_at=? WHERE id=? AND lease_owner=?")
                    .bind(empty).bind(Utc::now().timestamp()).bind(id).bind(&owner).execute(self.store.pool()).await.map_err(|_|DeliveryAuthError::Storage)?;
            }
            Err(error) => {
                let terminal = matches!(&error, IdentityError::Unauthenticated | IdentityError::Forbidden)
                    || matches!(&error,IdentityError::Rejected{status:400,code,..} if code=="invalid_grant");
                if terminal && operation != "revoke" {
                    // Never retain an expired/rejected SLT or access token. Keep only
                    // the family credential needed for explicit/superseded revocation.
                    let retained =
                        self.seal(&row, &Credentials { refresh: credentials.refresh, ..Default::default() })?;
                    sqlx::query("UPDATE delivery_credentials SET encrypted_payload=?,state=CASE WHEN enabled=1 THEN 'needs_auth' ELSE 'revoking' END,operation=CASE WHEN enabled=1 THEN NULL ELSE 'revoke' END,mutation_key=CASE WHEN enabled=1 THEN NULL ELSE ? END,lease_owner=NULL,lease_until=0,updated_at=? WHERE id=? AND lease_owner=?")
                        .bind(retained).bind(Uuid::new_v4().to_string()).bind(Utc::now().timestamp()).bind(id).bind(&owner).execute(self.store.pool()).await.map_err(|_|DeliveryAuthError::Storage)?;
                } else {
                    sqlx::query("UPDATE delivery_credentials SET lease_owner=NULL,lease_until=?,updated_at=? WHERE id=? AND lease_owner=?")
                        .bind(Utc::now().timestamp()+5).bind(Utc::now().timestamp()).bind(id).bind(&owner).execute(self.store.pool()).await.map_err(|_|DeliveryAuthError::Storage)?;
                }
                return Err(error.into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{ExchangedAuth, OrganizationAccess, UpstreamFailure};
    use silicon_browser_shared::IdentityKind;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };
    struct Mock {
        expected: PrincipalIdentity,
        calls: Mutex<Vec<(String, String)>>,
        fail_exchange: AtomicBool,
        fail_refresh: AtomicBool,
        wrong_actor: AtomicBool,
        hold_refresh: AtomicBool,
        revoked: AtomicBool,
        access_revoked: AtomicBool,
        reject_refreshed_access: AtomicBool,
        late_replay: AtomicBool,
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    impl Mock {
        fn new() -> Self {
            Self {
                expected: PrincipalIdentity {
                    principal_id: Uuid::from_u128(1),
                    public_id: Some("actor".into()),
                    tags: Some(vec![]),
                    kind: IdentityKind::Silicon,
                    org_id: "org".into(),
                    membership_id: Uuid::from_u128(2),
                    authorization_epoch: 1,
                    expires_at: Utc::now() + chrono::TimeDelta::hours(1),
                },
                calls: Mutex::new(vec![]),
                fail_exchange: AtomicBool::new(false),
                fail_refresh: AtomicBool::new(false),
                wrong_actor: AtomicBool::new(false),
                hold_refresh: AtomicBool::new(false),
                revoked: AtomicBool::new(false),
                access_revoked: AtomicBool::new(false),
                reject_refreshed_access: AtomicBool::new(false),
                late_replay: AtomicBool::new(false),
                entered: tokio::sync::Notify::new(),
                release: tokio::sync::Notify::new(),
            }
        }
        fn auth(&self) -> ExchangedAuth {
            let mut identity = self.expected.clone();
            if self.wrong_actor.load(Ordering::SeqCst) {
                identity.principal_id = Uuid::from_u128(9);
            }
            ExchangedAuth {
                access_token: "oat_owned_backend".into(),
                refresh_token: "ort_owned_backend".into(),
                identity,
                scope: "obo.issue roles.read memberships.read".into(),
            }
        }
        fn transient() -> IdentityError {
            IdentityError::Upstream { kind: UpstreamFailure::Transport, request_id: None, retry_after: None }
        }
    }
    #[async_trait::async_trait]
    impl IdentityProvider for Mock {
        async fn identify(&self, _: &str, _: &str) -> std::result::Result<PrincipalIdentity, IdentityError> {
            if self.revoked.load(Ordering::SeqCst) || self.access_revoked.load(Ordering::SeqCst) {
                return Err(IdentityError::Unauthenticated);
            }
            Ok(self.expected.clone())
        }
        async fn orgs(&self, _: &str) -> std::result::Result<Vec<OrganizationAccess>, IdentityError> {
            Ok(vec![])
        }
        async fn exchange_short_lived_token(
            &self,
            r: ExchangeRequest,
        ) -> std::result::Result<ExchangedAuth, IdentityError> {
            self.calls.lock().unwrap().push(("exchange".into(), r.idempotency_key));
            if self.fail_exchange.swap(false, Ordering::SeqCst) {
                return Err(Self::transient());
            }
            Ok(self.auth())
        }
        async fn refresh(&self, r: RefreshRequest) -> std::result::Result<ExchangedAuth, IdentityError> {
            assert!(matches!(r.refresh_token.as_str(), "ort_owned_backend" | "ort_recovered_backend"));
            self.calls.lock().unwrap().push(("refresh".into(), r.idempotency_key));
            if self.hold_refresh.load(Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            if self.revoked.load(Ordering::SeqCst) {
                return Err(IdentityError::Unauthenticated);
            }
            self.access_revoked.store(self.reject_refreshed_access.load(Ordering::SeqCst), Ordering::SeqCst);
            if self.fail_refresh.swap(false, Ordering::SeqCst) {
                return Err(Self::transient());
            }
            Ok(self.auth())
        }
        async fn refresh_delivery_token(
            &self,
            r: RefreshRequest,
        ) -> std::result::Result<DeliveryTokenExchange, IdentityError> {
            if self.late_replay.swap(false, Ordering::SeqCst) {
                self.calls.lock().unwrap().push(("late_replay".into(), r.idempotency_key));
                let mut auth = self.auth();
                auth.refresh_token = "ort_recovered_backend".into();
                return Ok(DeliveryTokenExchange { auth, access_active: false });
            }
            self.refresh(r).await.map(|auth| DeliveryTokenExchange { auth, access_active: true })
        }
        async fn revoke_application_token(&self, token: &str, key: &str) -> std::result::Result<(), IdentityError> {
            assert_eq!(token, "ort_owned_backend");
            self.calls.lock().unwrap().push(("revoke".into(), key.into()));
            Ok(())
        }
        async fn issue_recording_proof(
            &self,
            token: &str,
            _: RecordingProofRequest,
        ) -> std::result::Result<RecordingProof, IdentityError> {
            assert_eq!(token, "oat_owned_backend");
            Ok(RecordingProof {
                grant: crate::providers::OnBehalfOfGrant::new("obo_owned_proof").unwrap(),
                proof_id: Uuid::new_v4(),
                expires_at: Utc::now() + chrono::TimeDelta::seconds(60),
            })
        }
    }
    async fn fixture() -> (DeliveryAuth, Arc<Mock>) {
        let store = Store::connect("sqlite::memory:").await.unwrap();
        let mock = Arc::new(Mock::new());
        (DeliveryAuth::new(store, SecretBox::new(&[37; 32]), mock.clone()), mock)
    }
    fn slt() -> String {
        format!("oac_{}", "a".repeat(43))
    }
    fn request() -> RecordingProofRequest {
        RecordingProofRequest {
            expected_org_id: "org".into(),
            expected_actor_id: "actor".into(),
            audience: "org>briefcase".into(),
            path: "".into(),
            name: "video.mp4".into(),
            content_type: "video/mp4".into(),
            body_sha256: "a".repeat(64),
            idempotency_key: Uuid::new_v4().to_string(),
        }
    }
    async fn expire(service: &DeliveryAuth) {
        sqlx::query("UPDATE delivery_credentials SET access_expires_at=0,lease_until=0")
            .execute(service.store.pool())
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn enrollment_is_encrypted_bound_and_same_slt_replay_reuses_family() {
        let (service, mock) = fixture().await;
        let status = service.enroll("org", &mock.expected, &slt()).await.unwrap();
        assert!(status.enabled);
        assert_eq!(status.state, State::Active);
        service.enroll("org", &mock.expected, &slt()).await.unwrap();
        assert_eq!(mock.calls.lock().unwrap().len(), 1);
        let row: Grant =
            sqlx::query_as("SELECT * FROM delivery_credentials").fetch_one(service.store.pool()).await.unwrap();
        assert!(!row.encrypted_payload.contains("ort_"));
        assert!(!row.encrypted_payload.contains("oat_"));
        assert!(service.open(&row).unwrap().slt.is_none());
        let mut swapped = row;
        swapped.actor_id = "other".into();
        assert!(matches!(service.open(&swapped), Err(DeliveryAuthError::Storage)));
        service.issue_recording_proof(request()).await.unwrap();
        let mut other = request();
        other.expected_actor_id = "other".into();
        assert!(matches!(service.issue_recording_proof(other).await, Err(DeliveryAuthError::NeedsAuthorization)));
    }
    #[tokio::test]
    async fn uncertain_exchange_and_refresh_recover_exact_keys_after_restart() {
        let (service, mock) = fixture().await;
        mock.fail_exchange.store(true, Ordering::SeqCst);
        assert!(service.enroll("org", &mock.expected, &slt()).await.is_err());
        expire(&service).await;
        let restarted = DeliveryAuth::new(service.store.clone(), service.secrets.clone(), mock.clone());
        assert_eq!(restarted.recover_pending_once().await.unwrap(), 1);
        mock.fail_refresh.store(true, Ordering::SeqCst);
        expire(&restarted).await;
        assert!(restarted.issue_recording_proof(request()).await.is_err());
        expire(&restarted).await;
        restarted.recover_pending_once().await.unwrap();
        let calls = mock.calls.lock().unwrap();
        assert_eq!(calls[0], calls[1]);
        assert_eq!(calls[2], calls[3]);
        assert_ne!(calls[0].1, calls[2].1);
    }
    #[tokio::test]
    async fn wrong_principal_exchange_is_never_enabled_and_family_is_revoked() {
        let (service, mock) = fixture().await;
        mock.wrong_actor.store(true, Ordering::SeqCst);
        assert!(matches!(
            service.enroll("org", &mock.expected, &slt()).await,
            Err(DeliveryAuthError::Identity(IdentityError::Forbidden))
        ));
        assert!(!service.status("org", "actor").await.unwrap().enabled);
        service.recover_pending_once().await.unwrap();
        assert_eq!(mock.calls.lock().unwrap().last().unwrap().0, "revoke");
    }
    #[tokio::test]
    async fn concurrent_refresh_has_one_owner_and_disable_wins_inflight_rotation() {
        let (service, mock) = fixture().await;
        service.enroll("org", &mock.expected, &slt()).await.unwrap();
        expire(&service).await;
        mock.hold_refresh.store(true, Ordering::SeqCst);
        let running = service.clone();
        let task = tokio::spawn(async move { running.issue_recording_proof(request()).await });
        mock.entered.notified().await;
        assert!(matches!(service.issue_recording_proof(request()).await, Err(DeliveryAuthError::Busy)));
        assert!(!service.disable("org", "actor").await.unwrap().enabled);
        mock.release.notify_one();
        assert!(matches!(task.await.unwrap(), Err(DeliveryAuthError::NeedsAuthorization)));
        service.recover_pending_once().await.unwrap();
        assert_eq!(service.status("org", "actor").await.unwrap().state, State::Disabled);
        let row: Grant =
            sqlx::query_as("SELECT * FROM delivery_credentials").fetch_one(service.store.pool()).await.unwrap();
        let credentials = service.open(&row).unwrap();
        assert!(credentials.refresh.is_none());
        assert!(credentials.access.is_none());
    }
    #[tokio::test]
    async fn live_revocation_requires_new_authorization_without_refreshing_cli_tokens() {
        let (service, mock) = fixture().await;
        service.enroll("org", &mock.expected, &slt()).await.unwrap();
        mock.revoked.store(true, Ordering::SeqCst);
        assert!(matches!(
            service.issue_recording_proof(request()).await,
            Err(DeliveryAuthError::Identity(IdentityError::Unauthenticated))
        ));
        assert_eq!(service.status("org", "actor").await.unwrap().state, State::NeedsAuth);
        assert!(matches!(service.issue_recording_proof(request()).await, Err(DeliveryAuthError::NeedsAuthorization)));
        assert_eq!(mock.calls.lock().unwrap().len(), 2);
    }
    #[tokio::test]
    async fn sibling_family_access_revocation_refreshes_only_owned_family_once() {
        let (service, mock) = fixture().await;
        service.enroll("org", &mock.expected, &slt()).await.unwrap();
        mock.access_revoked.store(true, Ordering::SeqCst);
        service.issue_recording_proof(request()).await.unwrap();
        assert_eq!(service.status("org", "actor").await.unwrap().state, State::Active);
        let calls = mock.calls.lock().unwrap();
        assert_eq!(calls.iter().map(|v| v.0.as_str()).collect::<Vec<_>>(), ["exchange", "refresh"]);
        assert_ne!(calls[0].1, calls[1].1);
    }
    #[tokio::test]
    async fn fresh_access_rejection_after_recovery_does_not_loop() {
        let (service, mock) = fixture().await;
        service.enroll("org", &mock.expected, &slt()).await.unwrap();
        mock.access_revoked.store(true, Ordering::SeqCst);
        mock.reject_refreshed_access.store(true, Ordering::SeqCst);
        assert!(service.issue_recording_proof(request()).await.is_err());
        assert_eq!(service.status("org", "actor").await.unwrap().state, State::NeedsAuth);
        assert!(service.issue_recording_proof(request()).await.is_err());
        assert_eq!(mock.calls.lock().unwrap().len(), 2);
    }
    #[tokio::test]
    async fn historical_job_requires_original_principal_and_membership_before_any_exchange() {
        let (service, mock) = fixture().await;
        service.enroll("org", &mock.expected, &slt()).await.unwrap();
        let (principal, membership) = service.authorized_binding("org", "actor").await.unwrap();
        expire(&service).await;
        assert!(matches!(
            service.issue_recording_proof_for_principal(&Uuid::from_u128(99).to_string(), &membership, request()).await,
            Err(DeliveryAuthError::NeedsAuthorization)
        ));
        assert!(matches!(
            service.issue_recording_proof_for_principal(&principal, &Uuid::from_u128(99).to_string(), request()).await,
            Err(DeliveryAuthError::NeedsAuthorization)
        ));
        assert_eq!(mock.calls.lock().unwrap().len(), 1);
        service.issue_recording_proof_for_principal(&principal, &membership, request()).await.unwrap();
    }
    #[tokio::test]
    async fn stale_token_failure_does_not_disable_a_newly_rotated_pair() {
        let (service, mock) = fixture().await;
        service.enroll("org", &mock.expected, &slt()).await.unwrap();
        let old: Grant =
            sqlx::query_as("SELECT * FROM delivery_credentials").fetch_one(service.store.pool()).await.unwrap();
        expire(&service).await;
        service.issue_recording_proof(request()).await.unwrap();
        service.observe_failure(&old, &IdentityError::Unauthenticated).await.unwrap();
        assert_eq!(service.status("org", "actor").await.unwrap().state, State::Active);
    }
    #[tokio::test]
    async fn reused_public_id_keeps_families_and_status_scoped_to_immutable_principals() {
        let (service, mock) = fixture().await;
        service.enroll("org", &mock.expected, &slt()).await.unwrap();
        let mut replacement_mock = Mock::new();
        replacement_mock.expected.principal_id = Uuid::from_u128(98);
        replacement_mock.expected.membership_id = Uuid::from_u128(99);
        let replacement_mock = Arc::new(replacement_mock);
        let replacement = DeliveryAuth::new(service.store.clone(), service.secrets.clone(), replacement_mock.clone());
        assert!(!replacement.status_for_principal("org", &replacement_mock.expected).await.unwrap().enabled);
        replacement.disable_for_principal("org", &replacement_mock.expected).await.unwrap();
        assert!(service.status_for_principal("org", &mock.expected).await.unwrap().enabled);
        replacement.enroll("org", &replacement_mock.expected, &format!("oac_{}", "b".repeat(43))).await.unwrap();
        assert!(service.status_for_principal("org", &mock.expected).await.unwrap().enabled);
        assert!(replacement.status_for_principal("org", &replacement_mock.expected).await.unwrap().enabled);
        assert!(matches!(service.authorized_binding("org", "actor").await, Err(DeliveryAuthError::NeedsAuthorization)));
        service
            .issue_recording_proof_for_principal(
                &mock.expected.principal_id.to_string(),
                &mock.expected.membership_id.to_string(),
                request(),
            )
            .await
            .unwrap();
        replacement.disable_for_principal("org", &replacement_mock.expected).await.unwrap();
        replacement.recover_pending_once().await.unwrap();
        assert!(service.status_for_principal("org", &mock.expected).await.unwrap().enabled);
        assert_eq!(mock.calls.lock().unwrap().len(), 1);
        assert_eq!(replacement_mock.calls.lock().unwrap().last().unwrap().0, "revoke");
    }
    #[tokio::test]
    async fn late_replay_persists_recovered_ort_before_a_new_durable_refresh() {
        let (service, mock) = fixture().await;
        service.enroll("org", &mock.expected, &slt()).await.unwrap();
        let id: String =
            sqlx::query_scalar("SELECT id FROM delivery_credentials").fetch_one(service.store.pool()).await.unwrap();
        let original_key = Uuid::new_v4().to_string();
        sqlx::query(
            "UPDATE delivery_credentials SET operation='refresh',state='refreshing',mutation_key=?,access_expires_at=0",
        )
        .bind(&original_key)
        .execute(service.store.pool())
        .await
        .unwrap();
        mock.late_replay.store(true, Ordering::SeqCst);
        service.process(&id).await.unwrap();
        let row = service.get(&id).await.unwrap();
        assert_eq!(row.operation.as_deref(), Some("refresh"));
        assert_eq!(row.access_expires_at, 0);
        assert_eq!(service.open(&row).unwrap().refresh.as_deref(), Some("ort_recovered_backend"));
        assert_ne!(row.mutation_key.as_deref(), Some(original_key.as_str()));
        let restarted = DeliveryAuth::new(service.store.clone(), service.secrets.clone(), mock.clone());
        restarted.recover_pending_once().await.unwrap();
        assert_eq!(restarted.status("org", "actor").await.unwrap().state, State::Active);
        restarted.issue_recording_proof(request()).await.unwrap();
        let calls = mock.calls.lock().unwrap();
        assert_eq!(calls[1], ("late_replay".into(), original_key));
        assert_eq!(calls[2].0, "refresh");
    }
}
