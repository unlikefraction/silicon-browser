//! Durable, expiring browser login attempts. A retry may exchange only the same
//! SLT; IAM's stable mutation key recovers a lost response without a second login.
use super::*;
use silicon_browser_shared::IdentityKind;
use sqlx::Row;
use subtle::ConstantTimeEq;

/// Browser-only contract; CLI/manual clients retain the direct SLT exchange.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LoginAttemptRequest {
    identity_kind: IdentityKind,
}

/// Preserve the ID and state through the exact callback. Access and refresh
/// credentials never travel through popup messages or callback URLs.
#[derive(Serialize)]
pub(super) struct LoginAttempt {
    attempt_id: String,
    state: String,
    identity_kind: IdentityKind,
    expires_at: DateTime<Utc>,
}

/// An identical retry recovers an uncertain IAM exchange; a different SLT
/// cannot reuse an attempt that already submitted a token to IAM.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LoginAttemptComplete {
    short_lived_token: String,
    state: String,
}

fn database(error: sqlx::Error) -> ApiFailure {
    StoreError::from(error).into()
}
fn invalid_attempt() -> ApiFailure {
    ApiFailure::bad_request("invalid_login_attempt", "This sign-in attempt is invalid or expired. Start sign-in again.")
}
fn kind_name(kind: IdentityKind) -> &'static str {
    match kind {
        IdentityKind::Carbon => "carbon",
        IdentityKind::Silicon => "silicon",
    }
}
fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

pub(super) async fn start(
    State(state): State<AppState>,
    payload: Result<Json<LoginAttemptRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let request = json_payload(payload)?;
    let now = Utc::now();
    let attempt = LoginAttempt {
        attempt_id: Uuid::new_v4().to_string(),
        state: format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
        identity_kind: request.identity_kind,
        expires_at: now + TimeDelta::minutes(10),
    };
    let mut tx = state.store.pool().begin_with("BEGIN IMMEDIATE").await.map_err(database)?;
    sqlx::query("DELETE FROM login_attempts WHERE expires_at<=?")
        .bind(now.timestamp())
        .execute(&mut *tx)
        .await
        .map_err(database)?;
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM login_attempts").fetch_one(&mut *tx).await.map_err(database)?;
    if count >= 10_000 {
        return Err(ApiFailure::new(
            StatusCode::TOO_MANY_REQUESTS,
            "login_busy",
            "Please try signing in again shortly.",
        ));
    }
    sqlx::query("INSERT INTO login_attempts(id,identity_kind,state_digest,expires_at) VALUES(?,?,?,?)")
        .bind(&attempt.attempt_id)
        .bind(kind_name(attempt.identity_kind))
        .bind(digest(&attempt.state))
        .bind(attempt.expires_at.timestamp())
        .execute(&mut *tx)
        .await
        .map_err(database)?;
    tx.commit().await.map_err(database)?;
    Ok(([(http::header::CACHE_CONTROL, "no-store")], success(attempt)))
}

pub(super) async fn complete(
    State(state): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<LoginAttemptComplete>, JsonRejection>,
) -> Result<impl IntoResponse, ApiFailure> {
    let request = json_payload(payload)?;
    if request.state.len() != 64 || !request.state.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid_attempt());
    }
    AuthExchangeRequest { short_lived_token: request.short_lived_token.clone(), org_id: None }
        .validate()
        .map_err(ApiFailure::validation)?;
    let token_digest = digest(&request.short_lived_token);
    // Persist the exact SLT binding before calling IAM, including if its answer
    // is lost. Concurrent requests and process restarts retain the same key.
    let mut tx = state.store.pool().begin_with("BEGIN IMMEDIATE").await.map_err(database)?;
    let row = sqlx::query("SELECT * FROM login_attempts WHERE id=?")
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database)?
        .ok_or_else(invalid_attempt)?;
    let expected_state: String = row.try_get("state_digest").map_err(database)?;
    if !bool::from(expected_state.as_bytes().ct_eq(digest(&request.state).as_bytes()))
        || row.try_get::<i64, _>("expires_at").map_err(database)? <= Utc::now().timestamp()
        || row.try_get::<bool, _>("rejected").map_err(database)?
        || row
            .try_get::<Option<String>, _>("token_digest")
            .map_err(database)?
            .is_some_and(|bound| bound != token_digest)
    {
        return Err(invalid_attempt());
    }
    let expected_kind: String = row.try_get("identity_kind").map_err(database)?;
    sqlx::query("UPDATE login_attempts SET token_digest=? WHERE id=?")
        .bind(&token_digest)
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(database)?;
    tx.commit().await.map_err(database)?;
    let exchanged = state
        .identity
        .exchange_short_lived_token(ExchangeRequest {
            short_lived_token: request.short_lived_token,
            required_org_id: None,
            idempotency_key: format!("browser-login-{id}"),
        })
        .await
        .map_err(ApiFailure::from)?;
    if kind_name(exchanged.identity.kind) != expected_kind {
        sqlx::query("UPDATE login_attempts SET rejected=1 WHERE id=?")
            .bind(&id)
            .execute(state.store.pool())
            .await
            .map_err(database)?;
        // A mismatched account never reaches the browser. Revoke the unused
        // family when IAM permits it; a revoke failure still cannot allow login.
        let _ = state
            .identity
            .revoke_application_token(&exchanged.refresh_token, &format!("browser-login-reject-{id}"))
            .await;
        return Err(ApiFailure::bad_request(
            "login_identity_mismatch",
            "The returned account did not match the selected identity kind. Start sign-in again.",
        ));
    }
    Ok(([(http::header::CACHE_CONTROL, "no-store")], success(auth_session(&state, exchanged).await?)))
}
