//! Explicit feature consent. No ordinary login or old delivery family is an OBO grant.
use super::*;
use silicon_browser_shared::IdentityKind;
use silicon_iam_client::{
    Client, IdempotencyKey, Mutation,
    models::{ActorRefType, OboAuthorizationEndpoint, OboAuthorizationRequest, OboTokenPair},
};
use sqlx::Row;
use subtle::ConstantTimeEq;

pub const ENDPOINTS: [&str; 3] = ["briefcase.uploads.reserve", "briefcase.uploads.commit", "briefcase.entries.list"];
#[derive(Serialize)]
pub struct RecordingConsent {
    pub authorization_id: String,
    pub consent_url: Option<String>,
    pub state: String,
    pub status: String,
    pub expires_at: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentComplete {
    pub code: String,
    pub state: String,
}

/// Request-local selected provider authority; no refresh credential leaves this module.
#[derive(Debug)]
pub struct RecordingTokens {
    /// Digest of this exact credential payload, used to fence delayed rejection.
    pub credential_version: String,
    pub org_id: String,
    pub actor_id: String,
    pub reserve: crate::providers::OnBehalfOfGrant,
    pub commit: crate::providers::OnBehalfOfGrant,
    pub list: crate::providers::OnBehalfOfGrant,
    pub testing_secret: Option<secrecy::SecretString>,
    pub expires_at: chrono::DateTime<Utc>,
}
fn database(_: sqlx::Error) -> DeliveryAuthError {
    DeliveryAuthError::Storage
}
fn mutation(key: &str) -> Result<Mutation> {
    IdempotencyKey::parse(key).map(Mutation::with_key).map_err(|_| {
        IdentityError::InvalidInput { field: "idempotency_key", reason: "use 16 through 255 ASCII characters" }.into()
    })
}
fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn upstream(error: silicon_iam_client::Error) -> DeliveryAuthError {
    match error {
        silicon_iam_client::Error::Api(api)
            if matches!(
                api.code.as_str(),
                "invalid_grant"
                    | "obo_access_token_invalid"
                    | "obo_token_invalid"
                    | "obo_token_expired"
                    | "obo_token_revoked"
                    | "obo_authorization_denied"
                    | "obo_consent_required"
            ) =>
        {
            DeliveryAuthError::NeedsAuthorization
        }
        _ => IdentityError::Upstream {
            kind: crate::auth::UpstreamFailure::Unavailable,
            request_id: None,
            retry_after: None,
        }
        .into(),
    }
}
impl DeliveryAuth {
    fn sdk(&self) -> Result<Client> {
        self.identity
            .recording_client()
            .ok_or_else(|| IdentityError::CapabilityUnavailable(crate::auth::IdentityCapability::RecordingProof).into())
    }
    fn obo_context(&self, suffix: &str) -> String {
        format!(
            "recording-obo/{}/{suffix}",
            self.identity.recording_environment().map_or_else(|| "production".into(), |id| id.to_string())
        )
    }
    fn grant_context(&self, org: &str, principal: &str, membership: &str) -> String {
        self.obo_context(&format!("{org}/{principal}/{membership}/tokens"))
    }
    fn check_actor<'a>(org: &str, actor: &'a PrincipalIdentity) -> Result<&'a str> {
        if actor.org_id != org || actor.expires_at <= Utc::now() {
            return Err(IdentityError::Forbidden.into());
        }
        actor.public_id.as_deref().filter(|id| !id.is_empty()).ok_or_else(|| IdentityError::Forbidden.into())
    }
    fn consent_response(&self, row: &sqlx::sqlite::SqliteRow) -> Result<RecordingConsent> {
        let id: String = row.try_get("id").map_err(database)?;
        let cipher: String = row.try_get("state_cipher").map_err(database)?;
        Ok(RecordingConsent {
            authorization_id: id.clone(),
            consent_url: row.try_get("consent_url").map_err(database)?,
            state: self
                .secrets
                .open_for(&self.obo_context(&format!("{id}/state")), &cipher)
                .map_err(|_| DeliveryAuthError::Storage)?,
            status: row.try_get("status").map_err(database)?,
            expires_at: chrono::DateTime::from_timestamp(row.try_get("expires_at").map_err(database)?, 0)
                .ok_or(DeliveryAuthError::Storage)?
                .to_rfc3339(),
        })
    }
    pub async fn start_consent(
        &self,
        org: &str,
        actor: &PrincipalIdentity,
        bearer: &str,
        key: &str,
        redirect_uri: Option<&str>,
    ) -> Result<RecordingConsent> {
        let public = Self::check_actor(org, actor)?;
        mutation(key)?;
        // Capture the exact login token before the first network attempt. A retry after
        // login rotation must retain the original authorization payload and mutation key.
        let sdk = self.sdk()?;
        let id = Uuid::now_v7().to_string();
        let state = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let request = OboAuthorizationRequest {
            subject_token: bearer.into(),
            org_id: org.into(),
            endpoints: ENDPOINTS
                .iter()
                .map(|endpoint| OboAuthorizationEndpoint {
                    audience: self.audience.clone(),
                    endpoint_id: (*endpoint).into(),
                })
                .collect(),
            redirect_uri: redirect_uri.map(str::to_owned),
            state: redirect_uri.map(|_| state.clone()),
        };
        let request_cipher = self
            .secrets
            .seal_for(
                &self.obo_context(&format!("{id}/request")),
                &serde_json::to_string(&request).map_err(|_| DeliveryAuthError::Storage)?,
            )
            .map_err(|_| DeliveryAuthError::Storage)?;
        let state_cipher = self
            .secrets
            .seal_for(&self.obo_context(&format!("{id}/state")), &state)
            .map_err(|_| DeliveryAuthError::Storage)?;
        sqlx::query("INSERT INTO recording_obo_authorizations(id,org_id,principal_id,membership_id,actor_id,retry_key,request_cipher,state_cipher,state_digest,expires_at) VALUES(?,?,?,?,?,?,?,?,?,?) ON CONFLICT(org_id,principal_id,membership_id,retry_key) DO NOTHING")
            .bind(&id).bind(org).bind(&actor.principal_id).bind(&actor.membership_id).bind(public).bind(key).bind(request_cipher).bind(state_cipher).bind(digest(&state)).bind(Utc::now().timestamp()+900).execute(self.store.pool()).await.map_err(database)?;
        let mut tx = self.store.pool().begin_with("BEGIN IMMEDIATE").await.map_err(database)?;
        let mut row=sqlx::query("SELECT * FROM recording_obo_authorizations WHERE org_id=? AND principal_id=? AND membership_id=? AND retry_key=?").bind(org).bind(&actor.principal_id).bind(&actor.membership_id).bind(key).fetch_one(&mut *tx).await.map_err(database)?;
        if row.try_get::<i64, _>("expires_at").map_err(database)? <= Utc::now().timestamp() {
            return Err(DeliveryAuthError::NeedsAuthorization);
        }
        // A pending operation may be retried, but its callback cannot be changed.
        if row.try_get::<String, _>("status").map_err(database)? == "pending" {
            let stored_id: String = row.try_get("id").map_err(database)?;
            let plain = self
                .secrets
                .open_for(
                    &self.obo_context(&format!("{stored_id}/request")),
                    &row.try_get::<String, _>("request_cipher").map_err(database)?,
                )
                .map_err(|_| DeliveryAuthError::Storage)?;
            let stored: OboAuthorizationRequest =
                serde_json::from_str(&plain).map_err(|_| DeliveryAuthError::Storage)?;
            if stored.redirect_uri.as_deref() != redirect_uri {
                return Err(DeliveryAuthError::InvalidConsent);
            }
        }
        if row.try_get::<Option<String>, _>("iam_id").map_err(database)?.is_none() {
            let id: String = row.try_get("id").map_err(database)?;
            let plain = self
                .secrets
                .open_for(
                    &self.obo_context(&format!("{id}/request")),
                    &row.try_get::<String, _>("request_cipher").map_err(database)?,
                )
                .map_err(|_| DeliveryAuthError::Storage)?;
            let request = serde_json::from_str(&plain).map_err(|_| DeliveryAuthError::Storage)?;
            let response =
                sdk.obo().authorize(&request, &mutation(&format!("browser-consent-{id}"))?).await.map_err(upstream)?;
            // The origin account is fixed by this authenticated request. Provider
            // account selection happens later, on IAM's consent page.
            if response.id.is_nil()
                || response.app_id != self.identity.app_id()
                || response.org_id != org
                || response.actor.public_id != public
                || !matches!(
                    (&response.actor.type_field, &actor.kind),
                    (ActorRefType::Carbon, IdentityKind::Carbon) | (ActorRefType::Silicon, IdentityKind::Silicon)
                )
                || response.expires_at.unix_timestamp() <= Utc::now().timestamp()
            {
                return Err(DeliveryAuthError::Storage);
            }
            let url = response.authorization_url.ok_or(DeliveryAuthError::Storage)?;
            let parsed = url::Url::parse(&url).map_err(|_| DeliveryAuthError::Storage)?;
            if !crate::url_policy::is_https_or_loopback_http(&parsed)
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.fragment().is_some()
            {
                return Err(DeliveryAuthError::Storage);
            }
            row=sqlx::query("UPDATE recording_obo_authorizations SET iam_id=?,consent_url=?,expires_at=MIN(expires_at,?) WHERE id=? RETURNING *").bind(response.id.to_string()).bind(url).bind(response.expires_at.unix_timestamp()).bind(&id).fetch_one(&mut *tx).await.map_err(database)?;
        }
        tx.commit().await.map_err(database)?;
        self.consent_response(&row)
    }
    pub async fn consent_status(&self, org: &str, actor: &PrincipalIdentity, id: &str) -> Result<RecordingConsent> {
        Self::check_actor(org, actor)?;
        let row = sqlx::query(
            "SELECT * FROM recording_obo_authorizations WHERE id=? AND org_id=? AND principal_id=? AND membership_id=?",
        )
        .bind(id)
        .bind(org)
        .bind(&actor.principal_id)
        .bind(&actor.membership_id)
        .fetch_optional(self.store.pool())
        .await
        .map_err(database)?
        .ok_or(IdentityError::Forbidden)?;
        self.consent_response(&row)
    }
    pub async fn complete_consent(
        &self,
        org: &str,
        actor: &PrincipalIdentity,
        id: &str,
        body: ConsentComplete,
    ) -> Result<RecordingConsent> {
        let public = Self::check_actor(org, actor)?;
        if !body.code.starts_with("obc_")
            || !(5..=16384).contains(&body.code.len())
            || !body.code.bytes().all(|b| b.is_ascii_graphic())
            || body.state.len() != 64
            || !body.state.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(DeliveryAuthError::InvalidConsent);
        }
        let sdk = self.sdk()?;
        let mut tx = self.store.pool().begin_with("BEGIN IMMEDIATE").await.map_err(database)?;
        let row = sqlx::query(
            "SELECT * FROM recording_obo_authorizations WHERE id=? AND org_id=? AND principal_id=? AND membership_id=?",
        )
        .bind(id)
        .bind(org)
        .bind(&actor.principal_id)
        .bind(&actor.membership_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database)?
        .ok_or(IdentityError::Forbidden)?;
        let expected: String = row.try_get("state_digest").map_err(database)?;
        if !bool::from(expected.as_bytes().ct_eq(digest(&body.state).as_bytes())) {
            return Err(DeliveryAuthError::InvalidConsent);
        }
        let code_digest = digest(&body.code);
        if row.try_get::<String, _>("status").map_err(database)? == "completed" {
            if row.try_get::<Option<String>, _>("code_digest").map_err(database)?.as_deref() != Some(&code_digest) {
                return Err(IdentityError::Forbidden.into());
            }
            return self.consent_response(&row);
        }
        if row.try_get::<i64, _>("expires_at").map_err(database)? <= Utc::now().timestamp() {
            return Err(DeliveryAuthError::NeedsAuthorization);
        }
        let iam_id = row
            .try_get::<Option<String>, _>("iam_id")
            .map_err(database)?
            .and_then(|value| Uuid::parse_str(&value).ok())
            .ok_or(DeliveryAuthError::NeedsAuthorization)?;
        let response = sdk
            .obo()
            .exchange_code(iam_id, &body.code, &mutation(&format!("browser-code-{id}-{code_digest}"))?)
            .await
            .map_err(|error| match &error {
                silicon_iam_client::Error::Api(api)
                    if api.code != "invalid_client"
                        && matches!(api.status, 400 | 401 | 403 | 404 | 409 | 410 | 422) =>
                {
                    DeliveryAuthError::InvalidConsent
                }
                _ => upstream(error),
            })?;
        self.validate_tokens(&response.items)?;
        let plaintext = serde_json::to_string(&response.items).map_err(|_| DeliveryAuthError::Storage)?;
        let credential_version = digest(&plaintext);
        let cipher = self
            .secrets
            .seal_for(&self.grant_context(org, &actor.principal_id, &actor.membership_id), &plaintext)
            .map_err(|_| DeliveryAuthError::Storage)?;
        sqlx::query("INSERT INTO recording_obo_grants(org_id,principal_id,membership_id,actor_id,tokens_cipher,credential_version) VALUES(?,?,?,?,?,?) ON CONFLICT(org_id,principal_id,membership_id) DO UPDATE SET actor_id=excluded.actor_id,tokens_cipher=excluded.tokens_cipher,credential_version=excluded.credential_version,enabled=1,needs_auth=0").bind(org).bind(&actor.principal_id).bind(&actor.membership_id).bind(public).bind(cipher).bind(credential_version).execute(&mut *tx).await.map_err(database)?;
        let row=sqlx::query("UPDATE recording_obo_authorizations SET status='completed',code_digest=?,request_cipher='' WHERE id=? RETURNING *").bind(code_digest).bind(id).fetch_one(&mut *tx).await.map_err(database)?;
        tx.commit().await.map_err(database)?;
        self.consent_response(&row)
    }
    fn validate_tokens(&self, pairs: &[OboTokenPair]) -> Result<()> {
        if pairs.len() != ENDPOINTS.len() {
            return Err(DeliveryAuthError::Storage);
        }
        let first = pairs.first().ok_or(DeliveryAuthError::Storage)?;
        for endpoint in ENDPOINTS {
            let pair = pairs.iter().find(|pair| pair.endpoint_id == endpoint).ok_or(DeliveryAuthError::Storage)?;
            if pair.audience != self.audience || pair.org_id.is_empty() || !pair.access_token.starts_with("oba_") || !pair.refresh_token.starts_with("obr_") || !(5..=16384).contains(&pair.access_token.len()) || !(5..=16384).contains(&pair.refresh_token.len()) || !pair.access_token.bytes().all(|b|b.is_ascii_graphic()) || !pair.refresh_token.bytes().all(|b|b.is_ascii_graphic()) || pair.actor.as_ref().is_none_or(|actor| !matches!((&actor.type_field,actor.public_id.split_once(':')), (silicon_iam_client::models::ActorRefType::Carbon,Some(("c",suffix))) | (silicon_iam_client::models::ActorRefType::Silicon,Some(("si",suffix))) if !suffix.is_empty() && suffix.bytes().all(|b|b.is_ascii_alphanumeric() || matches!(b,b'-'|b'_')))) || pair.org_id!=first.org_id || pair.actor.as_ref().map(|actor|&actor.public_id)!=first.actor.as_ref().map(|actor|&actor.public_id) || pair.testing_context.is_some()!=self.identity.is_testing() || pair.testing_context.as_ref().is_some_and(|context|context.app_id!=self.audience || !context.app_secret.starts_with("ask_")) || pair.testing_context.as_ref().map(|c|(&c.app_id,&c.app_secret))!=first.testing_context.as_ref().map(|c|(&c.app_id,&c.app_secret)) {return Err(DeliveryAuthError::Storage);}
        }
        Ok(())
    }
    pub async fn recording_tokens(
        &self,
        org: &str,
        actor: &str,
        principal: &str,
        membership: &str,
        force_refresh: bool,
    ) -> Result<RecordingTokens> {
        let sdk = self.sdk()?;
        let mut tx = self.store.pool().begin_with("BEGIN IMMEDIATE").await.map_err(database)?;
        let row=sqlx::query("SELECT * FROM recording_obo_grants WHERE org_id=? AND principal_id=? AND membership_id=? AND actor_id=? AND enabled=1 AND needs_auth=0").bind(org).bind(principal).bind(membership).bind(actor).fetch_optional(&mut *tx).await.map_err(database)?.ok_or(DeliveryAuthError::NeedsAuthorization)?;
        let context = self.grant_context(org, principal, membership);
        let plain = self
            .secrets
            .open_for(&context, &row.try_get::<String, _>("tokens_cipher").map_err(database)?)
            .map_err(|_| DeliveryAuthError::Storage)?;
        let mut pairs: Vec<OboTokenPair> = serde_json::from_str(&plain).map_err(|_| DeliveryAuthError::Storage)?;
        self.validate_tokens(&pairs)?;
        for pair in &mut pairs {
            if force_refresh || pair.expires_at.unix_timestamp() <= Utc::now().timestamp() + 90 {
                // The token's high-entropy digest identifies this rotation across process
                // crashes and lost responses. A SQLite write lock serializes consumers.
                let key = format!("browser-refresh-{}", digest(&pair.refresh_token));
                match sdk.obo().refresh(&pair.refresh_token, &mutation(&key)?).await {
                    Ok(mut result) if result.items.len() == 1 => {
                        let replacement = result.items.remove(0);
                        if replacement.endpoint_id != pair.endpoint_id
                            || replacement.audience != pair.audience
                            || replacement.org_id != pair.org_id
                            || replacement.actor.as_ref().map(|a| &a.public_id)
                                != pair.actor.as_ref().map(|a| &a.public_id)
                        {
                            return Err(DeliveryAuthError::Storage);
                        }
                        *pair = replacement;
                    }
                    Ok(_) => return Err(DeliveryAuthError::Storage),
                    Err(error) => {
                        let error = upstream(error);
                        if matches!(error, DeliveryAuthError::NeedsAuthorization) {
                            sqlx::query("UPDATE recording_obo_grants SET needs_auth=1 WHERE org_id=? AND principal_id=? AND membership_id=?").bind(org).bind(principal).bind(membership).execute(&mut *tx).await.map_err(database)?;
                            tx.commit().await.map_err(database)?;
                        }
                        return Err(error);
                    }
                }
            }
        }
        self.validate_tokens(&pairs)?;
        let plaintext = serde_json::to_string(&pairs).map_err(|_| DeliveryAuthError::Storage)?;
        let credential_version = digest(&plaintext);
        let cipher = self.secrets.seal_for(&context, &plaintext).map_err(|_| DeliveryAuthError::Storage)?;
        sqlx::query(
            "UPDATE recording_obo_grants SET tokens_cipher=?,credential_version=? WHERE org_id=? AND principal_id=? AND membership_id=?",
        )
        .bind(cipher)
        .bind(&credential_version)
        .bind(org)
        .bind(principal)
        .bind(membership)
        .execute(&mut *tx)
        .await
        .map_err(database)?;
        tx.commit().await.map_err(database)?;
        let first = &pairs[0];
        let token = |endpoint| {
            crate::providers::OnBehalfOfGrant::new(
                &pairs.iter().find(|p| p.endpoint_id == endpoint).ok_or(DeliveryAuthError::Storage)?.access_token,
            )
            .map_err(|_| DeliveryAuthError::Storage)
        };
        let expiry = pairs.iter().map(|p| p.expires_at.unix_timestamp()).min().ok_or(DeliveryAuthError::Storage)?;
        if expiry <= Utc::now().timestamp() + 5 {
            return Err(DeliveryAuthError::NeedsAuthorization);
        }
        Ok(RecordingTokens {
            credential_version,
            org_id: first.org_id.clone(),
            actor_id: first.actor.as_ref().ok_or(DeliveryAuthError::Storage)?.public_id.clone(),
            reserve: token(ENDPOINTS[0])?,
            commit: token(ENDPOINTS[1])?,
            list: token(ENDPOINTS[2])?,
            testing_secret: first.testing_context.as_ref().map(|c| secrecy::SecretString::from(c.app_secret.clone())),
            expires_at: chrono::DateTime::from_timestamp(expiry, 0).ok_or(DeliveryAuthError::Storage)?,
        })
    }
    pub async fn storage_binding(
        &self,
        org: &str,
        actor: &str,
        principal: &str,
        membership: &str,
        live: bool,
    ) -> Result<(String, String)> {
        if live {
            self.recording_tokens(org, actor, principal, membership, true).await?;
        } else {
            let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recording_obo_grants WHERE org_id=? AND principal_id=? AND membership_id=? AND actor_id=? AND enabled=1 AND needs_auth=0)").bind(org).bind(principal).bind(membership).bind(actor).fetch_one(self.store.pool()).await.map_err(database)?;
            if !active {
                return Err(DeliveryAuthError::NeedsAuthorization);
            }
        }
        Ok((principal.into(), membership.into()))
    }
    pub async fn storage_status(&self, org: &str, actor: &PrincipalIdentity) -> Result<DeliveryAuthorization> {
        let public = Self::check_actor(org, actor)?;
        let row = sqlx::query(
            "SELECT enabled,needs_auth FROM recording_obo_grants WHERE org_id=? AND principal_id=? AND membership_id=?",
        )
        .bind(org)
        .bind(&actor.principal_id)
        .bind(&actor.membership_id)
        .fetch_optional(self.store.pool())
        .await
        .map_err(database)?;
        let enabled = row.as_ref().is_some_and(|row| row.get::<bool, _>("enabled"));
        let state = if !enabled {
            State::Disabled
        } else if row.as_ref().is_some_and(|row| row.get::<bool, _>("needs_auth")) {
            State::NeedsAuth
        } else {
            State::Active
        };
        Ok(DeliveryAuthorization { configured: true, enabled, state, actor_id: public.into() })
    }
    /// A delayed rejection of an older credential must not disable a newer approval.
    pub async fn invalidate_storage(
        &self,
        org: &str,
        principal: &str,
        membership: &str,
        credential_version: &str,
    ) -> Result<()> {
        sqlx::query("UPDATE recording_obo_grants SET needs_auth=1 WHERE org_id=? AND principal_id=? AND membership_id=? AND credential_version=?")
            .bind(org).bind(principal).bind(membership).bind(credential_version).execute(self.store.pool()).await.map_err(database)?;
        Ok(())
    }
    pub async fn disable_storage(&self, org: &str, actor: &PrincipalIdentity) -> Result<DeliveryAuthorization> {
        Self::check_actor(org, actor)?;
        // IAM grant management remains available to the user. Local disable erases
        // all credentials and immediately blocks new/delayed recording work.
        sqlx::query("UPDATE recording_obo_grants SET enabled=0,tokens_cipher='' WHERE org_id=? AND principal_id=? AND membership_id=?").bind(org).bind(&actor.principal_id).bind(&actor.membership_id).execute(self.store.pool()).await.map_err(database)?;
        self.storage_status(org, actor).await
    }
}
