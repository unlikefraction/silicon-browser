//! Verified IAM notifications invalidate short-lived authorization snapshots.
//! Raw event data is never persisted; only delivery receipts are retained.

use crate::{auth_cache::AuthorizationCache, store::Store};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use serde_json::json;
use silicon_iam_client::{
    EnvironmentKey,
    webhook::{WebhookSecret, WebhookSecretKeyring, WebhookVerifier},
};
use std::sync::Arc;

const MAX_BODY: usize = 1024 * 1024;

#[derive(Clone)]
struct WebhookState {
    verifier: Arc<WebhookVerifier>,
    environment: Option<EnvironmentKey>,
    store: Store,
    cache: Arc<AuthorizationCache>,
}

pub fn router(
    store: Store,
    cache: Arc<AuthorizationCache>,
    secret: &str,
    version: i64,
    test_key: Option<&str>,
) -> Result<Router, String> {
    let secret =
        WebhookSecret::new(secret).map_err(|_| "IAM_WEBHOOK_SECRET must be 32–512 non-whitespace ASCII characters")?;
    let keyring = WebhookSecretKeyring::new(version, secret).map_err(|_| "IAM_WEBHOOK_KEY_VERSION must be positive")?;
    let state = WebhookState {
        verifier: Arc::new(WebhookVerifier::new(keyring).with_max_body_bytes(MAX_BODY)),
        store,
        cache,
        environment: test_key
            .map(EnvironmentKey::new)
            .transpose()
            .map_err(|_| "invalid IAM webhook test environment")?,
    };
    Ok(Router::new()
        .route("/webhooks/iam", post(receive))
        .route("/webhooks/iam/", post(receive))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state(state))
}

async fn receive(
    State(state): State<WebhookState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let verified = state.verifier.verify(&headers, &body).map_err(|_| StatusCode::UNAUTHORIZED)?;
    match &state.environment {
        Some(key) => verified.verify_testing_environment(key).map_err(|_| StatusCode::UNAUTHORIZED)?,
        None if verified.is_testing() => return Err(StatusCode::UNAUTHORIZED),
        None => (),
    }
    // Invalidate before durable acknowledgement. Retried events also invalidate, so an uncertain
    // database commit never leaves a new cache generation relying on the old event's receipt.
    state.cache.invalidate();
    let event = verified.event();
    let inserted = sqlx::query("INSERT INTO iam_webhook_receipts (event_id, event_type, aggregate_id, aggregate_version, received_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(event_id) DO NOTHING")
        .bind(event.event_id.to_string()).bind(&event.event_type)
        .bind(event.aggregate.get("id").and_then(|value| value.as_str()).unwrap_or_default())
        .bind(event.aggregate.get("version").and_then(|value| value.as_i64()).unwrap_or_default())
        .bind(chrono::Utc::now().to_rfc3339()).execute(state.store.pool()).await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?.rows_affected();
    // Keep an operational receipt window rather than accumulating a second IAM directory.
    sqlx::query("DELETE FROM iam_webhook_receipts WHERE received_at < ?")
        .bind((chrono::Utc::now() - chrono::Duration::days(45)).to_rfc3339())
        .execute(state.store.pool())
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(json!({"received": true, "duplicate": inserted == 0})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use hmac::{Hmac, Mac};
    use http::Request;
    use tower::ServiceExt;

    fn signed(body: &str, event: &str, timestamp: i64) -> Request<Body> {
        let mut mac = Hmac::<sha2_10::Sha256>::new_from_slice(&[b'x'; 32]).unwrap();
        mac.update(format!("{timestamp}.{body}").as_bytes());
        let signature = hex::encode(mac.finalize().into_bytes());
        Request::post("/webhooks/iam")
            .header("x-silicon-iam-event-id", event)
            .header("x-silicon-iam-timestamp", timestamp.to_string())
            .header("x-silicon-iam-key-version", "1")
            .header("x-silicon-iam-signature", format!("v1={signature}"))
            .body(Body::from(body.to_owned()))
            .unwrap()
    }

    /// Test group: exact signed bytes, idempotent receipts, no test-plane authority in production.
    #[tokio::test]
    async fn production_webhook_authenticates_and_deduplicates() {
        let store = Store::connect("sqlite::memory:").await.unwrap();
        let app = router(store.clone(), Arc::new(AuthorizationCache::default()), &"x".repeat(32), 1, None).unwrap();
        let event = uuid::Uuid::new_v4().to_string();
        let body = json!({"spec_version":"1.0","event_id":event,"event_type":"organization.membership.removed.v1",
            "occurred_at":chrono::Utc::now().to_rfc3339(),"organization_id":null,
            "aggregate":{"type":"membership","id":uuid::Uuid::new_v4(),"version":1},"data":{}})
        .to_string();
        let timestamp = chrono::Utc::now().timestamp();
        for _ in 0..2 {
            assert_eq!(app.clone().oneshot(signed(&body, &event, timestamp)).await.unwrap().status(), StatusCode::OK);
        }
        let count: (i64,) =
            sqlx::query_as("SELECT count(*) FROM iam_webhook_receipts").fetch_one(store.pool()).await.unwrap();
        assert_eq!(count.0, 1);
        let mut forged = signed(&body, &event, timestamp);
        *forged.body_mut() = Body::from(format!("{body} "));
        assert_eq!(app.clone().oneshot(forged).await.unwrap().status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            app.clone().oneshot(signed(&body, &event, timestamp - 360)).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        let wrapped = json!({"test":{"testing_key":"A".repeat(32),"metadata":{
            "spec_version":"1.0","event_id":event,"event_type":"organization.membership.removed.v1",
            "occurred_at":chrono::Utc::now().to_rfc3339(),"organization_id":null,
            "aggregate":{"type":"membership","id":uuid::Uuid::new_v4(),"version":1}},"data":{}}})
        .to_string();
        assert_eq!(app.oneshot(signed(&wrapped, &event, timestamp)).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    }
}
