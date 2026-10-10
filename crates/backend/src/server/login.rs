//! Hosted Carbon login uses server-owned PKCE and one-shot redemption.
use super::*;
use crate::auth::CodeExchangeRequest;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use silicon_browser_shared::IdentityKind;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BeginRequest {
    identity_kind: IdentityKind,
}
#[derive(Serialize)]
struct Attempt {
    attempt_id: Uuid,
    state: String,
    authorize_url: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExchangeAttemptRequest {
    state: String,
    code: String,
}
fn storage_error(_: impl std::fmt::Debug) -> ApiFailure {
    ApiFailure::new(StatusCode::INTERNAL_SERVER_ERROR, "login_unavailable", "login state is unavailable")
}
fn no_store<T: Serialize>(value: T) -> Response {
    let mut response = success(value).into_response();
    response.headers_mut().insert("cache-control", HeaderValue::from_static("no-store"));
    response
}
pub(super) async fn claim_token(state: &AppState, operation: &str, token: &str) -> Result<(), ApiFailure> {
    let digest = hex::encode(Sha256::digest(token.as_bytes()));
    sqlx::query("DELETE FROM accounts_token_operations WHERE submitted_at < ?")
        .bind((Utc::now() - TimeDelta::days(901)).timestamp())
        .execute(state.store.pool())
        .await
        .map_err(storage_error)?;
    let result = sqlx::query("INSERT OR IGNORE INTO accounts_token_operations(token_digest,operation,submitted_at) SELECT ?,?,? WHERE (SELECT COUNT(*) FROM accounts_token_operations) < 100000")
        .bind(digest).bind(operation).bind(Utc::now().timestamp()).execute(state.store.pool()).await.map_err(storage_error)?;
    if result.rows_affected() == 0 {
        return Err(ApiFailure::conflict(
            "accounts_token_already_submitted",
            "This credential was already submitted. Sign in again after an interrupted exchange; replaying it can revoke your sign-in.",
        ));
    }
    Ok(())
}
pub(super) async fn begin(
    State(state): State<AppState>,
    payload: Result<Json<BeginRequest>, JsonRejection>,
) -> Result<Response, ApiFailure> {
    let input = json_payload(payload)?;
    if input.identity_kind != IdentityKind::Carbon {
        return Err(ApiFailure::field(
            "identity_kind",
            "Silicons sign in with a short-lived token from silicon-accounts",
        ));
    }
    let now = Utc::now().timestamp();
    sqlx::query("DELETE FROM accounts_login_attempts WHERE expires_at <= ?")
        .bind(now)
        .execute(state.store.pool())
        .await
        .map_err(storage_error)?;
    let id = Uuid::new_v4();
    let nonce = format!("{id}.{}", hex::encode(rand::random::<[u8; 32]>()));
    let verifier = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    // This exact fixed URI can be registered once with Accounts. State carries the attempt ID.
    let redirect = format!("{}/auth/callback", state.public_origin);
    let authorize_url = state.identity.authorization_url(&redirect, &nonce, &challenge)?;
    let sealed = state.secrets.seal_for(&format!("accounts-login:{id}"), &verifier).map_err(storage_error)?;
    let inserted = sqlx::query("INSERT INTO accounts_login_attempts(id,state_hash,code_verifier_enc,redirect_uri,expires_at) SELECT ?,?,?,?,? WHERE (SELECT COUNT(*) FROM accounts_login_attempts) < 4096")
        .bind(id.to_string()).bind(hex::encode(Sha256::digest(nonce.as_bytes()))).bind(sealed).bind(redirect).bind(now+600).execute(state.store.pool()).await.map_err(storage_error)?;
    if inserted.rows_affected() == 0 {
        return Err(ApiFailure::new(
            StatusCode::TOO_MANY_REQUESTS,
            "login_busy",
            "too many pending login attempts; retry shortly",
        ));
    }
    Ok(no_store(Attempt { attempt_id: id, state: nonce, authorize_url }))
}
pub(super) async fn exchange(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    payload: Result<Json<ExchangeAttemptRequest>, JsonRejection>,
) -> Result<Response, ApiFailure> {
    let input = json_payload(payload)?;
    if input.state.len() != 101
        || !input.state.starts_with(&format!("{id}."))
        || !input.code.starts_with("sac_")
        || input.code.len() > 16384
        || !input.code.bytes().all(|b| b.is_ascii_graphic())
    {
        return Err(ApiFailure::unauthenticated());
    }
    // Claim before network I/O, even if the response is lost. Accounts revokes on code reuse.
    let row: Option<(String,String)> = sqlx::query_as("UPDATE accounts_login_attempts SET claimed_at=? WHERE id=? AND state_hash=? AND expires_at>? AND claimed_at IS NULL RETURNING code_verifier_enc,redirect_uri")
        .bind(Utc::now().timestamp()).bind(id.to_string()).bind(hex::encode(Sha256::digest(input.state.as_bytes()))).bind(Utc::now().timestamp()).fetch_optional(state.store.pool()).await.map_err(storage_error)?;
    let (sealed, redirect_uri) = row.ok_or_else(ApiFailure::unauthenticated)?;
    claim_token(&state, "authorization_code", &input.code).await?;
    let code_verifier = state.secrets.open_for(&format!("accounts-login:{id}"), &sealed).map_err(storage_error)?;
    let exchanged = state
        .identity
        .exchange_authorization_code(CodeExchangeRequest { code: input.code, redirect_uri, code_verifier })
        .await?;
    if exchanged.identity.kind != IdentityKind::Carbon {
        return Err(ApiFailure::forbidden("hosted sign-in returned a different account kind"));
    }
    Ok(no_store(auth_session(&state, exchanged).await?))
}
