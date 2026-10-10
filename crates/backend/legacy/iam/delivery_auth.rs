//! Feature-scoped IAM OBO consent. Ordinary application credentials are never used for uploads.
use crate::{
    auth::{IdentityError, IdentityProvider, PrincipalIdentity},
    crypto::SecretBox,
    providers::OnBehalfOfGrant,
    store::Store,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use silicon_browser_shared::{
    DeliveryAuthorization, DeliveryAuthorizationConsent, DeliveryAuthorizationRequest,
    DeliveryAuthorizationState as State,
};
use silicon_iam_client::models::{OboAuthorizationEndpoint, OboAuthorizationRequest, OboConsentDetail, OboTokenPair};
use sqlx::FromRow;
use std::sync::Arc;
use uuid::Uuid;

pub use crate::providers::BRIEFCASE_RECORDING_ENDPOINTS as RECORDING_ENDPOINTS;

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
    audience: String,
}
#[derive(FromRow)]
struct Grant {
    id: String,
    org_id: String,
    actor_id: String,
    world: String,
    enabled: bool,
    state: String,
    operation: Option<String>,
    mutation_key: Option<String>,
    encrypted_payload: String,
    destination_org: Option<String>,
    destination_actor: Option<String>,
}
// SDK wire models contain secrets in Debug; this private encrypted payload deliberately has no Debug.
#[derive(Default, Serialize, Deserialize)]
struct Credentials {
    request: Option<OboAuthorizationRequest>,
    consent: Option<OboConsentDetail>,
    code: Option<String>,
    code_digest: Option<String>,
    pairs: Vec<OboTokenPair>,
}

pub struct DeliveryBinding {
    pub principal_id: String,
    pub membership_id: String,
    pub destination_org: String,
    pub destination_actor: String,
}

pub struct DeliveryAccess {
    pub token: OnBehalfOfGrant,
    pub org_id: String,
    pub actor_id: String,
    pub testing_app_secret: Option<String>,
}

impl DeliveryAuth {
    pub fn new(store: Store, secrets: SecretBox, identity: Arc<dyn IdentityProvider>, audience: String) -> Self {
        Self { store, secrets, identity, audience }
    }
    fn context(row: &Grant) -> String {
        format!("delivery-obo/{}/{}/{}/{}/credentials", row.world, row.org_id, row.actor_id, row.id)
    }
    fn open(&self, row: &Grant) -> Result<Credentials> {
        if row.world != self.identity.delivery_world() {
            return Err(DeliveryAuthError::NeedsAuthorization);
        }
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
        sqlx::query_as("SELECT * FROM delivery_obo WHERE id=? AND world=?")
            .bind(id)
            .bind(self.identity.delivery_world())
            .fetch_optional(self.store.pool())
            .await
            .map_err(|_| DeliveryAuthError::Storage)?
            .ok_or(DeliveryAuthError::NeedsAuthorization)
    }
    fn matches(row: &Grant, p: &PrincipalIdentity) -> bool {
        row.org_id == p.org_id
            && p.public_id.as_deref() == Some(&row.actor_id)
            && row.actor_id == p.principal_id
            && p.membership_id == format!("{}[{}]", row.actor_id, row.org_id)
            && silicon_browser_shared::actor_id(&row.actor_id, "actor").ok() == Some(p.kind)
    }
    fn public(row: Option<&Grant>, actor: &str) -> DeliveryAuthorization {
        DeliveryAuthorization {
            configured: true,
            enabled: row.is_some_and(|row| row.enabled),
            actor_id: actor.into(),
            state: match row.map(|r| r.state.as_str()) {
                Some("active") => State::Active,
                Some("refreshing") => State::Refreshing,
                Some("pending") => State::Pending,
                Some("needs_auth") => State::NeedsAuth,
                _ => State::Disabled,
            },
        }
    }
    async fn current(&self, org: &str, actor: &str) -> Result<Grant> {
        sqlx::query_as("SELECT * FROM delivery_obo WHERE org_id=? AND actor_id=? AND world=? AND enabled=1")
            .bind(org)
            .bind(actor)
            .bind(self.identity.delivery_world())
            .fetch_optional(self.store.pool())
            .await
            .map_err(|_| DeliveryAuthError::Storage)?
            .ok_or(DeliveryAuthError::NeedsAuthorization)
    }
    pub async fn status_for_principal(&self, org: &str, expected: &PrincipalIdentity) -> Result<DeliveryAuthorization> {
        if expected.org_id != org {
            return Err(IdentityError::Forbidden.into());
        }
        let row: Option<Grant> = sqlx::query_as("SELECT * FROM delivery_obo WHERE org_id=? AND actor_id=? AND world=? ORDER BY enabled DESC,created_at DESC,rowid DESC LIMIT 1")
            .bind(org).bind(&expected.principal_id).bind(self.identity.delivery_world()).fetch_optional(self.store.pool()).await.map_err(|_| DeliveryAuthError::Storage)?;
        if row.as_ref().is_some_and(|r| !Self::matches(r, expected)) {
            return Err(IdentityError::Forbidden.into());
        }
        Ok(Self::public(row.as_ref(), &expected.principal_id))
    }
    pub async fn live_status_for_principal(
        &self,
        org: &str,
        expected: &PrincipalIdentity,
    ) -> Result<DeliveryAuthorization> {
        // Receiving providers verify every operation. Ordinary login expiry never revokes this consent.
        self.status_for_principal(org, expected).await
    }
    pub async fn disable_for_principal(
        &self,
        org: &str,
        expected: &PrincipalIdentity,
    ) -> Result<DeliveryAuthorization> {
        if expected.org_id != org
            || expected.public_id.as_deref() != Some(&expected.principal_id)
            || expected.membership_id != format!("{}[{org}]", expected.principal_id)
        {
            return Err(IdentityError::Forbidden.into());
        }
        // Disconnect locally. Revoking durable IAM consent is an explicit action in IAM's grant manager.
        sqlx::query(
            "UPDATE delivery_obo SET enabled=0,state='disabled',updated_at=? WHERE org_id=? AND actor_id=? AND world=?",
        )
        .bind(Utc::now().timestamp())
        .bind(org)
        .bind(&expected.principal_id)
        .bind(self.identity.delivery_world())
        .execute(self.store.pool())
        .await
        .map_err(|_| DeliveryAuthError::Storage)?;
        self.status_for_principal(org, expected).await
    }
    pub async fn begin(
        &self,
        expected: &PrincipalIdentity,
        subject_token: &str,
        key: Uuid,
    ) -> Result<DeliveryAuthorizationConsent> {
        let actor = expected.public_id.as_deref().ok_or(IdentityError::Forbidden)?;
        if key.is_nil()
            || actor != expected.principal_id
            || expected.membership_id != format!("{actor}[{}]", expected.org_id)
            || expected.expires_at <= Utc::now()
        {
            return Err(IdentityError::Forbidden.into());
        }
        let now = Utc::now().timestamp();
        let row = Grant {
            id: key.to_string(),
            org_id: expected.org_id.clone(),
            actor_id: actor.into(),
            world: self.identity.delivery_world(),
            enabled: false,
            state: "pending".into(),
            operation: Some("authorize".into()),
            mutation_key: Some(key.to_string()),
            encrypted_payload: String::new(),
            destination_org: None,
            destination_actor: None,
        };
        let credentials = Credentials {
            request: Some(OboAuthorizationRequest {
                redirect_uri: None,
                state: None,
                subject_token: subject_token.into(),
                org_id: expected.org_id.clone(),
                endpoints: RECORDING_ENDPOINTS
                    .iter()
                    .map(|endpoint| OboAuthorizationEndpoint {
                        audience: self.audience.clone(),
                        endpoint_id: (*endpoint).into(),
                    })
                    .collect(),
            }),
            ..Default::default()
        };
        sqlx::query("INSERT OR IGNORE INTO delivery_obo(id,org_id,actor_id,world,operation,mutation_key,encrypted_payload,created_at,updated_at) VALUES(?,?,?,?,'authorize',?,?,?,?)")
            .bind(&row.id).bind(&row.org_id).bind(&row.actor_id).bind(&row.world).bind(&row.mutation_key).bind(self.seal(&row, &credentials)?)
            .bind(now).bind(now).execute(self.store.pool()).await.map_err(|_| DeliveryAuthError::Storage)?;
        let row = self.get(&row.id).await?;
        if !Self::matches(&row, expected) || row.state == "disabled" {
            return Err(IdentityError::Forbidden.into());
        }
        if row.operation.is_some() {
            self.process(&row.id).await?;
        }
        let row = self.get(&row.id).await?;
        let consent = self.open(&row)?.consent.ok_or(DeliveryAuthError::NeedsAuthorization)?;
        Ok(DeliveryAuthorizationConsent {
            authorization_id: consent.id,
            authorization_url: consent.authorization_url.ok_or(DeliveryAuthError::NeedsAuthorization)?,
            expires_at: DateTime::from_timestamp(consent.expires_at.unix_timestamp(), 0)
                .ok_or(DeliveryAuthError::Storage)?,
        })
    }
    pub async fn complete(
        &self,
        expected: &PrincipalIdentity,
        request: &DeliveryAuthorizationRequest,
    ) -> Result<DeliveryAuthorization> {
        use silicon_browser_shared::Validate;
        request.validate().map_err(|_| IdentityError::Forbidden)?;
        let row: Grant = sqlx::query_as("SELECT * FROM delivery_obo WHERE authorization_id=? AND org_id=? AND actor_id=? AND world=? AND state <> 'disabled'")
            .bind(request.authorization_id.to_string()).bind(&expected.org_id).bind(&expected.principal_id).bind(self.identity.delivery_world())
            .fetch_optional(self.store.pool()).await.map_err(|_| DeliveryAuthError::Storage)?.ok_or(IdentityError::Forbidden)?;
        let mut credentials = self.open(&row)?;
        let digest = hex::encode(Sha256::digest(request.authorization_code.as_bytes()));
        if credentials.code_digest.as_ref().is_some_and(|previous| previous != &digest) {
            return Err(IdentityError::Forbidden.into());
        }
        if !Self::matches(&row, expected) {
            return Err(IdentityError::Forbidden.into());
        }
        if row.state == "active" {
            return Ok(Self::public(Some(&row), &row.actor_id));
        }
        if let Some(code) = &credentials.code {
            if code != &request.authorization_code {
                return Err(IdentityError::Forbidden.into());
            }
        } else {
            if row.operation.is_some() {
                return Err(DeliveryAuthError::Busy);
            }
            credentials.code = Some(request.authorization_code.clone());
            credentials.code_digest = Some(digest);
            let changed = sqlx::query("UPDATE delivery_obo SET encrypted_payload=?,operation='exchange',mutation_key=?,updated_at=? WHERE id=? AND operation IS NULL AND state='pending' AND encrypted_payload=?")
                .bind(self.seal(&row, &credentials)?).bind(Uuid::new_v4().to_string()).bind(Utc::now().timestamp()).bind(&row.id).bind(&row.encrypted_payload)
                .execute(self.store.pool()).await.map_err(|_| DeliveryAuthError::Storage)?.rows_affected();
            if changed != 1 {
                return Err(DeliveryAuthError::Busy);
            }
        }
        self.process(&row.id).await?;
        self.status_for_principal(&expected.org_id, expected).await
    }
    pub async fn authorized_binding_for_principal(
        &self,
        org: &str,
        actor: &str,
        principal: &str,
        membership: &str,
    ) -> Result<(String, String)> {
        if actor != principal || membership != format!("{actor}[{org}]") {
            return Err(DeliveryAuthError::NeedsAuthorization);
        }
        let row = self.current(org, actor).await?;
        if !matches!(row.state.as_str(), "active" | "refreshing") {
            return Err(DeliveryAuthError::NeedsAuthorization);
        }
        Ok((principal.into(), membership.into()))
    }
    pub async fn validate_authorized_binding_for_principal(
        &self,
        org: &str,
        actor: &str,
        principal: &str,
        membership: &str,
    ) -> Result<DeliveryBinding> {
        let binding = self.authorized_binding_for_principal(org, actor, principal, membership).await?;
        // Each root is independent: recheck all required permissions before starting paid work.
        let mut selected = None;
        for endpoint in RECORDING_ENDPOINTS {
            let access = self.access(org, actor, endpoint, None, true).await.map_err(|error| match error {
                DeliveryAuthError::Identity(IdentityError::Unauthenticated | IdentityError::Forbidden) => {
                    DeliveryAuthError::NeedsAuthorization
                }
                other => other,
            })?;
            selected = Some(access);
        }
        let access = selected.ok_or(DeliveryAuthError::NeedsAuthorization)?;
        Ok(DeliveryBinding {
            principal_id: binding.0,
            membership_id: binding.1,
            destination_org: access.org_id,
            destination_actor: access.actor_id,
        })
    }
    pub async fn access(
        &self,
        org: &str,
        actor: &str,
        endpoint: &str,
        destination: Option<(&str, &str)>,
        force_refresh: bool,
    ) -> Result<DeliveryAccess> {
        if !RECORDING_ENDPOINTS.contains(&endpoint) {
            return Err(IdentityError::Forbidden.into());
        }
        let mut row = self.current(org, actor).await?;
        if !matches!(row.state.as_str(), "active" | "refreshing") {
            return Err(DeliveryAuthError::NeedsAuthorization);
        }
        if let Some((org, actor)) = destination
            && (row.destination_org.as_deref() != Some(org) || row.destination_actor.as_deref() != Some(actor))
        {
            return Err(DeliveryAuthError::NeedsAuthorization);
        }
        if row.operation.is_some() {
            self.process(&row.id).await?;
            row = self.get(&row.id).await?;
        }
        for _ in 0..2 {
            let credentials = self.open(&row)?;
            let pair = credentials
                .pairs
                .iter()
                .find(|p| p.endpoint_id == endpoint)
                .ok_or(DeliveryAuthError::NeedsAuthorization)?;
            if !force_refresh && pair.expires_at.unix_timestamp() > Utc::now().timestamp() + 30 {
                break;
            }
            let changed = sqlx::query("UPDATE delivery_obo SET operation=?,mutation_key=?,state='refreshing',updated_at=? WHERE id=? AND enabled=1 AND operation IS NULL AND encrypted_payload=?")
                .bind(format!("refresh:{endpoint}")).bind(Uuid::new_v4().to_string()).bind(Utc::now().timestamp()).bind(&row.id).bind(&row.encrypted_payload)
                .execute(self.store.pool()).await.map_err(|_| DeliveryAuthError::Storage)?.rows_affected();
            if changed != 1 {
                return Err(DeliveryAuthError::Busy);
            }
            self.process(&row.id).await?;
            row = self.get(&row.id).await?;
            let refreshed = self.open(&row)?;
            if refreshed
                .pairs
                .iter()
                .any(|p| p.endpoint_id == endpoint && p.expires_at.unix_timestamp() > Utc::now().timestamp() + 5)
            {
                break;
            }
        }
        if !row.enabled || row.state != "active" {
            return Err(DeliveryAuthError::NeedsAuthorization);
        }
        let credentials = self.open(&row)?;
        let pair = credentials
            .pairs
            .into_iter()
            .find(|p| p.endpoint_id == endpoint)
            .ok_or(DeliveryAuthError::NeedsAuthorization)?;
        if pair.expires_at.unix_timestamp() <= Utc::now().timestamp() + 5 {
            return Err(DeliveryAuthError::NeedsAuthorization);
        }
        Ok(DeliveryAccess {
            token: OnBehalfOfGrant::new(pair.access_token).map_err(|_| DeliveryAuthError::Storage)?,
            org_id: pair.org_id,
            actor_id: pair.actor.ok_or(DeliveryAuthError::NeedsAuthorization)?.public_id,
            testing_app_secret: pair.testing_context.map(|c| c.app_secret),
        })
    }
    fn validate_pairs(&self, pairs: &[OboTokenPair]) -> Result<(String, String)> {
        if pairs.len() != RECORDING_ENDPOINTS.len() {
            return Err(IdentityError::Forbidden.into());
        }
        let first = &pairs[0];
        let actor = first.actor.as_ref().ok_or(IdentityError::Forbidden)?;
        for endpoint in RECORDING_ENDPOINTS {
            let roots: Vec<_> = pairs.iter().filter(|p| p.endpoint_id == endpoint).collect();
            if roots.len() != 1 {
                return Err(IdentityError::Forbidden.into());
            }
            let pair = roots[0];
            if pair.audience != self.audience
                || pair.org_id != first.org_id
                || pair.actor.as_ref().map(|a| (&a.public_id, &a.type_field))
                    != Some((&actor.public_id, &actor.type_field))
                || pair.testing_context.is_some() != self.identity.is_testing()
                || pair.testing_context.as_ref().is_some_and(|c| c.app_id != self.audience)
                || !pair.scope.split_whitespace().any(|s| s == format!("obo:{}:{}", self.audience, endpoint))
            {
                return Err(IdentityError::Forbidden.into());
            }
        }
        Ok((first.org_id.clone(), actor.public_id.clone()))
    }
    pub async fn recover_pending_once(&self) -> Result<usize> {
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM delivery_obo WHERE world=? AND operation IS NOT NULL AND lease_until<=? AND state <> 'disabled' ORDER BY updated_at LIMIT 16")
            .bind(self.identity.delivery_world()).bind(Utc::now().timestamp()).fetch_all(self.store.pool()).await.map_err(|_| DeliveryAuthError::Storage)?;
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
        let owner = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();
        let changed = sqlx::query("UPDATE delivery_obo SET lease_owner=?,lease_until=? WHERE id=? AND world=? AND operation IS NOT NULL AND state <> 'disabled' AND lease_until<=?")
            .bind(&owner).bind(now+90).bind(id).bind(self.identity.delivery_world()).bind(now).execute(self.store.pool()).await.map_err(|_| DeliveryAuthError::Storage)?.rows_affected();
        if changed != 1 {
            return Err(DeliveryAuthError::Busy);
        }
        let row = self.get(id).await?;
        let mut credentials = self.open(&row)?;
        let key = row.mutation_key.as_deref().ok_or(DeliveryAuthError::Storage)?;
        let operation = row.operation.as_deref().ok_or(DeliveryAuthError::Storage)?;
        let result = tokio::time::timeout(std::time::Duration::from_secs(60), async {
            if operation == "authorize" {
                credentials.consent = Some(
                    self.identity
                        .authorize_delivery(credentials.request.clone().ok_or(DeliveryAuthError::Storage)?, key)
                        .await?,
                );
                credentials.request = None;
            } else if operation == "exchange" {
                let consent = credentials.consent.as_ref().ok_or(DeliveryAuthError::Storage)?;
                credentials.pairs = self
                    .identity
                    .exchange_delivery_code(
                        consent.id,
                        credentials.code.as_deref().ok_or(DeliveryAuthError::Storage)?,
                        key,
                    )
                    .await?
                    .items;
                self.validate_pairs(&credentials.pairs)?;
                credentials.code = None;
            } else if let Some(endpoint) = operation.strip_prefix("refresh:") {
                let old = credentials
                    .pairs
                    .iter_mut()
                    .find(|p| p.endpoint_id == endpoint)
                    .ok_or(DeliveryAuthError::Storage)?;
                let mut result = self.identity.refresh_obo(&old.refresh_token, key).await?.items;
                if result.len() != 1 {
                    return Err(IdentityError::Forbidden.into());
                }
                let next = result.remove(0);
                if next.grant_id != old.grant_id
                    || next.audience != old.audience
                    || next.endpoint_id != old.endpoint_id
                    || next.org_id != old.org_id
                    || serde_json::to_value(&next.actor).ok() != serde_json::to_value(&old.actor).ok()
                    || next.scope != old.scope
                {
                    return Err(IdentityError::Forbidden.into());
                }
                *old = next;
                self.validate_pairs(&credentials.pairs)?;
            } else {
                return Err(DeliveryAuthError::Storage);
            }
            Ok(())
        })
        .await
        .unwrap_or(Err(DeliveryAuthError::Busy));
        if let Err(error) = result {
            let terminal = matches!(
                &error,
                DeliveryAuthError::Identity(IdentityError::Unauthenticated | IdentityError::Forbidden)
            ) || matches!(
                &error,
                DeliveryAuthError::Identity(IdentityError::Rejected { status: 400 | 410 | 412, .. })
            );
            sqlx::query("UPDATE delivery_obo SET state=CASE WHEN ? AND state <> 'disabled' THEN 'needs_auth' ELSE state END,operation=CASE WHEN ? THEN NULL ELSE operation END,mutation_key=CASE WHEN ? THEN NULL ELSE mutation_key END,lease_owner=NULL,lease_until=?,updated_at=? WHERE id=? AND lease_owner=?")
                .bind(terminal).bind(terminal).bind(terminal).bind(now+5).bind(now).bind(id).bind(&owner).execute(self.store.pool()).await.map_err(|_| DeliveryAuthError::Storage)?;
            return Err(error);
        }
        let destination =
            if credentials.pairs.is_empty() { None } else { Some(self.validate_pairs(&credentials.pairs)?) };
        let sealed = self.seal(&row, &credentials)?;
        let mut tx = self.store.pool().begin_with("BEGIN IMMEDIATE").await.map_err(|_| DeliveryAuthError::Storage)?;
        let state: Option<String> = sqlx::query_scalar("SELECT state FROM delivery_obo WHERE id=? AND lease_owner=?")
            .bind(id)
            .bind(&owner)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| DeliveryAuthError::Storage)?;
        let enable = destination.is_some() && state.as_deref().is_some_and(|s| s != "disabled");
        if enable {
            sqlx::query("UPDATE delivery_obo SET enabled=0,state='disabled',updated_at=? WHERE org_id=? AND actor_id=? AND world=? AND enabled=1 AND id<>?")
                .bind(now).bind(&row.org_id).bind(&row.actor_id).bind(&row.world).bind(id).execute(&mut *tx).await.map_err(|_| DeliveryAuthError::Storage)?;
        }
        let changed = sqlx::query("UPDATE delivery_obo SET encrypted_payload=?,authorization_id=?,destination_org=?,destination_actor=?,enabled=?,state=CASE WHEN state='disabled' THEN state WHEN ? THEN 'active' ELSE 'pending' END,operation=NULL,mutation_key=NULL,lease_owner=NULL,lease_until=0,updated_at=? WHERE id=? AND lease_owner=?")
            .bind(sealed).bind(credentials.consent.as_ref().map(|c| c.id.to_string())).bind(destination.as_ref().map(|d| &d.0)).bind(destination.as_ref().map(|d| &d.1)).bind(enable).bind(enable)
            .bind(now).bind(id).bind(&owner).execute(&mut *tx).await.map_err(|_| DeliveryAuthError::Storage)?.rows_affected();
        if changed != 1 {
            return Err(DeliveryAuthError::Busy);
        }
        tx.commit().await.map_err(|_| DeliveryAuthError::Storage)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{ExchangeRequest, ExchangedAuth, OrganizationAccess, RefreshRequest, UpstreamFailure};
    use serde_json::json;
    use silicon_browser_shared::IdentityKind;
    use silicon_iam_client::models::OboTokenResponse;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    };

    struct Mock {
        calls: Mutex<Vec<(String, String, String)>>,
        fail_authorize: AtomicBool,
        fail_refresh: AtomicBool,
        hold_refresh: AtomicBool,
        wrong_context: AtomicBool,
        missing_root: AtomicBool,
        generation: AtomicU64,
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    impl Mock {
        fn new() -> Self {
            Self {
                calls: Mutex::new(vec![]),
                fail_authorize: AtomicBool::new(false),
                fail_refresh: AtomicBool::new(false),
                hold_refresh: AtomicBool::new(false),
                wrong_context: AtomicBool::new(false),
                missing_root: AtomicBool::new(false),
                generation: AtomicU64::new(0),
                entered: tokio::sync::Notify::new(),
                release: tokio::sync::Notify::new(),
            }
        }
        fn pair(&self, endpoint: &str) -> OboTokenPair {
            let index = RECORDING_ENDPOINTS.iter().position(|e| *e == endpoint).unwrap();
            let mut value = json!({"grant_id":Uuid::from_u128(index as u128 + 100),
                "access_token":format!("oba_{endpoint}_secret"),"refresh_token":format!("obr_{endpoint}_secret"),
                "token_type":"Bearer","expires_in":3600,"expires_at":(Utc::now()+chrono::TimeDelta::hours(1)).to_rfc3339(),
                "audience":"briefcase","endpoint_id":endpoint,"org_id":"storage-org",
                "actor":{"type":"carbon","public_id":"c:provider"},"scope":format!("obo:briefcase:{endpoint}")});
            if self.is_testing() {
                value["testing_context"] = json!({"app_id":"briefcase","app_secret":format!("ask_{}","S".repeat(43)),"iam_test_key":"T".repeat(32)});
            }
            serde_json::from_value(value).unwrap()
        }
        fn transient() -> IdentityError {
            IdentityError::Upstream { kind: UpstreamFailure::Transport, request_id: None, retry_after: None }
        }
    }
    #[async_trait::async_trait]
    impl IdentityProvider for Mock {
        fn is_testing(&self) -> bool {
            self.generation.load(Ordering::SeqCst) > 0
        }
        fn delivery_world(&self) -> String {
            format!("world:{}", self.generation.load(Ordering::SeqCst))
        }
        async fn identify(&self, _: &str, _: &str) -> std::result::Result<PrincipalIdentity, IdentityError> {
            unreachable!("OBO never introspects an ordinary login")
        }
        async fn orgs(&self, _: &str) -> std::result::Result<Vec<OrganizationAccess>, IdentityError> {
            unreachable!()
        }
        async fn exchange_short_lived_token(
            &self,
            _: ExchangeRequest,
        ) -> std::result::Result<ExchangedAuth, IdentityError> {
            unreachable!("no implicit SLT-based OBO")
        }
        async fn refresh(&self, _: RefreshRequest) -> std::result::Result<ExchangedAuth, IdentityError> {
            unreachable!("ordinary token family must not be rotated")
        }
        async fn authorize_delivery(
            &self,
            request: OboAuthorizationRequest,
            key: &str,
        ) -> std::result::Result<OboConsentDetail, IdentityError> {
            self.calls.lock().unwrap().push(("authorize".into(), key.into(), serde_json::to_string(&request).unwrap()));
            if self.fail_authorize.swap(false, Ordering::SeqCst) {
                return Err(Self::transient());
            }
            Ok(serde_json::from_value(json!({"id":Uuid::parse_str(key).unwrap(),"app_id":"browser","app_name":"Browser",
                "actor":{"type":"silicon","public_id":"si:owner"},"org_id":request.org_id,"status":"pending","version":1,
                "expires_at":(Utc::now()+chrono::TimeDelta::minutes(10)).to_rfc3339(),"endpoints":[],
                "authorization_url":format!("https://auth.iam.teamofsilicons.com/obo/consent?request={key}")})).unwrap())
        }
        async fn exchange_delivery_code(
            &self,
            id: Uuid,
            code: &str,
            key: &str,
        ) -> std::result::Result<OboTokenResponse, IdentityError> {
            self.calls.lock().unwrap().push(("exchange".into(), key.into(), format!("{id}:{code}")));
            let mut pairs: Vec<_> = RECORDING_ENDPOINTS.iter().map(|e| self.pair(e)).collect();
            if self.missing_root.load(Ordering::SeqCst) {
                pairs.pop();
            }
            if self.wrong_context.load(Ordering::SeqCst) {
                pairs[0].org_id = "wrong-org".into();
            }
            Ok(OboTokenResponse { items: pairs })
        }
        async fn refresh_obo(&self, token: &str, key: &str) -> std::result::Result<OboTokenResponse, IdentityError> {
            self.calls.lock().unwrap().push(("refresh".into(), key.into(), token.into()));
            if self.hold_refresh.load(Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            if self.fail_refresh.swap(false, Ordering::SeqCst) {
                return Err(Self::transient());
            }
            let endpoint = token.strip_prefix("obr_").unwrap().strip_suffix("_secret").unwrap();
            let mut pair = self.pair(endpoint);
            if self.wrong_context.load(Ordering::SeqCst) {
                pair.org_id = "wrong-org".into();
            }
            Ok(OboTokenResponse { items: vec![pair] })
        }
    }
    fn principal() -> PrincipalIdentity {
        PrincipalIdentity {
            principal_id: "si:owner".into(),
            public_id: Some("si:owner".into()),
            tags: None,
            org_role: None,
            scopes: vec![],
            kind: IdentityKind::Silicon,
            org_id: "source-org".into(),
            membership_id: "si:owner[source-org]".into(),
            authorization_epoch: 1,
            expires_at: Utc::now() + chrono::TimeDelta::hours(1),
        }
    }
    async fn fixture() -> (DeliveryAuth, Arc<Mock>) {
        let mock = Arc::new(Mock::new());
        (
            DeliveryAuth::new(
                Store::in_memory().await.unwrap(),
                SecretBox::new(&[37; 32]),
                mock.clone(),
                "briefcase".into(),
            ),
            mock,
        )
    }
    async fn enroll(service: &DeliveryAuth) -> DeliveryAuthorizationRequest {
        let consent = service.begin(&principal(), "oat_ordinary_subject_secret", Uuid::new_v4()).await.unwrap();
        let request = DeliveryAuthorizationRequest {
            authorization_id: consent.authorization_id,
            authorization_code: "manual-one-use-code".into(),
        };
        assert_eq!(service.complete(&principal(), &request).await.unwrap().state, State::Active);
        request
    }
    async fn clear_backoff(service: &DeliveryAuth) {
        sqlx::query("UPDATE delivery_obo SET lease_until=0").execute(service.store.pool()).await.unwrap();
    }

    #[tokio::test]
    async fn consent_is_separate_bound_encrypted_and_retried_with_the_original_request() {
        let (service, mock) = fixture().await;
        assert_eq!(service.status_for_principal("source-org", &principal()).await.unwrap().state, State::Disabled);
        mock.fail_authorize.store(true, Ordering::SeqCst);
        let key = Uuid::new_v4();
        assert!(service.begin(&principal(), "oat_ordinary_subject_secret", key).await.is_err());
        let payload: String = sqlx::query_scalar("SELECT encrypted_payload FROM delivery_obo")
            .fetch_one(service.store.pool())
            .await
            .unwrap();
        assert!(!payload.contains("ordinary_subject_secret"));
        clear_backoff(&service).await;
        let consent = service.begin(&principal(), "oat_replacement_must_not_change_retry", key).await.unwrap();
        let calls = mock.calls.lock().unwrap().clone();
        assert_eq!(calls[0], calls[1]);
        assert!(!service.status_for_principal("source-org", &principal()).await.unwrap().enabled);
        let mut wrong = principal();
        wrong.org_id = "another-org".into();
        wrong.membership_id = "si:owner[another-org]".into();
        let request = DeliveryAuthorizationRequest {
            authorization_id: consent.authorization_id,
            authorization_code: "approved-code".into(),
        };
        assert!(service.complete(&wrong, &request).await.is_err());
        assert_eq!(service.complete(&principal(), &request).await.unwrap().state, State::Active);
        service.complete(&principal(), &request).await.unwrap();
        assert_eq!(mock.calls.lock().unwrap().iter().filter(|c| c.0 == "exchange").count(), 1);
        let changed = DeliveryAuthorizationRequest { authorization_code: "different-code".into(), ..request };
        assert!(service.complete(&principal(), &changed).await.is_err());
        let access = service
            .access("source-org", "si:owner", RECORDING_ENDPOINTS[0], Some(("storage-org", "c:provider")), false)
            .await
            .unwrap();
        assert_eq!(access.org_id, "storage-org");
        assert!(
            service
                .access("source-org", "si:owner", RECORDING_ENDPOINTS[0], Some(("source-org", "si:owner")), false)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn uncertain_refresh_keeps_its_key_and_disconnect_fences_a_concurrent_result() {
        let (service, mock) = fixture().await;
        enroll(&service).await;
        mock.fail_refresh.store(true, Ordering::SeqCst);
        assert!(service.access("source-org", "si:owner", RECORDING_ENDPOINTS[0], None, true).await.is_err());
        clear_backoff(&service).await;
        service.recover_pending_once().await.unwrap();
        let calls: Vec<_> = mock.calls.lock().unwrap().iter().filter(|c| c.0 == "refresh").cloned().collect();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], calls[1]);
        mock.hold_refresh.store(true, Ordering::SeqCst);
        let running = service.clone();
        let task =
            tokio::spawn(
                async move { running.access("source-org", "si:owner", RECORDING_ENDPOINTS[0], None, true).await },
            );
        mock.entered.notified().await;
        assert!(matches!(
            service.access("source-org", "si:owner", RECORDING_ENDPOINTS[0], None, true).await,
            Err(DeliveryAuthError::Busy)
        ));
        assert_eq!(service.disable_for_principal("source-org", &principal()).await.unwrap().state, State::Disabled);
        mock.release.notify_one();
        assert!(task.await.unwrap().is_err());
        assert!(!service.status_for_principal("source-org", &principal()).await.unwrap().enabled);
    }

    #[tokio::test]
    async fn root_sets_and_refreshes_cannot_move_an_existing_destination() {
        for missing in [true, false] {
            let (service, mock) = fixture().await;
            mock.missing_root.store(missing, Ordering::SeqCst);
            mock.wrong_context.store(!missing, Ordering::SeqCst);
            let consent = service.begin(&principal(), "subject", Uuid::new_v4()).await.unwrap();
            let request = DeliveryAuthorizationRequest {
                authorization_id: consent.authorization_id,
                authorization_code: "approved".into(),
            };
            assert!(service.complete(&principal(), &request).await.is_err());
            assert!(!service.status_for_principal("source-org", &principal()).await.unwrap().enabled);
        }
        let (service, mock) = fixture().await;
        enroll(&service).await;
        mock.wrong_context.store(true, Ordering::SeqCst);
        assert!(service.access("source-org", "si:owner", RECORDING_ENDPOINTS[0], None, true).await.is_err());
        assert_eq!(service.status_for_principal("source-org", &principal()).await.unwrap().state, State::NeedsAuth);
    }

    #[tokio::test]
    async fn cleanup_generation_cannot_recover_or_use_an_older_testing_family() {
        let (service, mock) = fixture().await;
        mock.generation.store(1, Ordering::SeqCst);
        enroll(&service).await;
        let access = service.access("source-org", "si:owner", RECORDING_ENDPOINTS[0], None, false).await.unwrap();
        assert!(access.testing_app_secret.is_some());
        mock.generation.store(2, Ordering::SeqCst);
        assert!(service.access("source-org", "si:owner", RECORDING_ENDPOINTS[0], None, false).await.is_err());
        assert_eq!(service.recover_pending_once().await.unwrap(), 0);
        assert_eq!(service.status_for_principal("source-org", &principal()).await.unwrap().state, State::Disabled);
    }
}
