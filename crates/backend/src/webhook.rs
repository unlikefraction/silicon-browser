//! Accounts HMAC notifications invalidate bounded authorization snapshots.
use crate::{auth_cache::AuthorizationCache, store::Store};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
#[derive(Clone)]
struct WebhookState {
    secret: Arc<str>,
    app_id: Arc<str>,
    store: Store,
    cache: Arc<AuthorizationCache>,
}
#[derive(Deserialize)]
struct Event {
    event_id: uuid::Uuid,
    #[serde(rename = "type")]
    kind: String,
    app_id: Option<String>,
    silicon: Option<String>,
    occurred_at: chrono::DateTime<chrono::Utc>,
}
pub fn router(store: Store, cache: Arc<AuthorizationCache>, secret: &str, app_id: &str) -> Result<Router, String> {
    if !secret.starts_with("whsec_")
        || secret.len() < 8
        || secret.len() > 512
        || !secret.bytes().all(|b| b.is_ascii_graphic())
    {
        return Err("ACCOUNTS_WEBHOOK_SECRET must be an Accounts whsec_ secret".into());
    }
    Ok(Router::new()
        .route("/webhooks/accounts", post(receive))
        .route("/webhooks/accounts/", post(receive))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .with_state(WebhookState { secret: secret.into(), app_id: app_id.into(), store, cache }))
}
fn header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, StatusCode> {
    if headers.get_all(name).iter().count() != 1 {
        return Err(StatusCode::UNAUTHORIZED);
    }
    headers.get(name).and_then(|v| v.to_str().ok()).ok_or(StatusCode::UNAUTHORIZED)
}
async fn receive(
    State(state): State<WebhookState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let timestamp = header(&headers, "x-accounts-timestamp")?;
    let ts = timestamp.parse::<i64>().map_err(|_| StatusCode::UNAUTHORIZED)?;
    if chrono::Utc::now().timestamp().abs_diff(ts) > 300 {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let mut mac =
        Hmac::<sha2_10::Sha256>::new_from_slice(state.secret.as_bytes()).map_err(|_| StatusCode::UNAUTHORIZED)?;
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(&body);
    let valid = header(&headers, "x-accounts-signature")?
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter_map(|part| part.strip_prefix("v1="))
        .filter_map(|signature| hex::decode(signature).ok())
        .any(|signature| mac.clone().verify_slice(&signature).is_ok());
    if !valid {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let event: Event = serde_json::from_slice(&body).map_err(|_| StatusCode::UNAUTHORIZED)?;
    if event.event_id.to_string() != header(&headers, "x-accounts-event-id")?
        || event.kind != header(&headers, "x-accounts-event-type")?
        || event.app_id.as_deref() != Some(&state.app_id)
        || event.silicon.is_some()
        || event.kind.is_empty()
        || event.kind.len() > 128
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let _ = event.occurred_at;
    state.cache.invalidate();
    let inserted =
        sqlx::query("INSERT OR IGNORE INTO accounts_webhook_receipts(event_id,event_type,received_at) VALUES (?,?,?)")
            .bind(event.event_id.to_string())
            .bind(&event.kind)
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(state.store.pool())
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .rows_affected();
    sqlx::query("DELETE FROM accounts_webhook_receipts WHERE received_at < ?")
        .bind((chrono::Utc::now() - chrono::Duration::days(45)).to_rfc3339())
        .execute(state.store.pool())
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(json!({"received":true,"duplicate":inserted==0})))
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http::Request;
    use tower::ServiceExt;
    const SECRET: &str = "whsec_test_secret";
    fn signed(body: &str, event: &str, timestamp: i64) -> Request<Body> {
        let mut mac = Hmac::<sha2_10::Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
        mac.update(format!("{timestamp}.{body}").as_bytes());
        Request::post("/webhooks/accounts")
            .header("x-accounts-event-id", event)
            .header("x-accounts-event-type", "membership.access_removed")
            .header("x-accounts-timestamp", timestamp.to_string())
            .header("x-accounts-signature", format!("v1={}", hex::encode(mac.finalize().into_bytes())))
            .body(Body::from(body.to_owned()))
            .unwrap()
    }
    #[tokio::test]
    async fn validates_exact_body_freshness_receiver_and_duplicate_receipts() {
        let store = Store::connect("sqlite::memory:").await.unwrap();
        let app = router(store.clone(), Arc::new(AuthorizationCache::default()), SECRET, "browser").unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();
        let body=json!({"event_id":id,"type":"membership.access_removed","app_id":"browser","silicon":null,"occurred_at":chrono::Utc::now().to_rfc3339(),"data":{"uuid":"8HV"}}).to_string();
        for _ in 0..2 {
            assert_eq!(app.clone().oneshot(signed(&body, &id, now)).await.unwrap().status(), StatusCode::OK);
        }
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM accounts_webhook_receipts").fetch_one(store.pool()).await.unwrap();
        assert_eq!(count, 1);
        let mut forged = signed(&body, &id, now);
        *forged.body_mut() = Body::from(format!("{body} "));
        assert_eq!(app.clone().oneshot(forged).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            app.clone().oneshot(signed(&body, &id, now - 301)).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        let other = body.replace("browser", "other");
        assert_eq!(app.oneshot(signed(&other, &id, now)).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    }
}
