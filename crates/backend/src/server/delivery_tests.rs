use super::*;
use crate::providers::BriefcaseClient;

async fn configured_delivery_fixture() -> Fixture {
    let mut fixture = fixture().await;
    fixture.state = fixture
        .state
        .clone()
        .with_recording_delivery(
            BriefcaseClient::new("http://127.0.0.1:1", None).unwrap(),
            "org-1>browser".into(),
            "org-1>briefcase".into(),
        )
        .unwrap();
    fixture.app = router(fixture.state.clone());
    fixture
}
fn delivery_slt(letter: char) -> String {
    format!("oac_{}", letter.to_string().repeat(43))
}
fn allow_delivery_exchange(fixture: &Fixture, slt: &str, identity: PrincipalIdentity) {
    fixture.identity.allow_exchange(
        slt,
        "org-1",
        ExchangedAuth {
            access_token: "oat_backend_delivery_secret".into(),
            refresh_token: "ort_backend_delivery_secret".into(),
            identity,
            scope: "obo.issue memberships.read roles.read".into(),
        },
    );
}
async fn enroll_delivery(fixture: &Fixture, bearer: &str, slt: &str) -> (StatusCode, Value) {
    let (status, _, body) = request(
        &fixture.app,
        "POST",
        "/api/v1/auth/delivery",
        Some((bearer, "org-1")),
        Some(json!({"short_lived_token":slt})),
    )
    .await;
    let raw = String::from_utf8_lossy(&body);
    assert!(!raw.contains("oat_backend_delivery_secret"));
    assert!(!raw.contains("ort_backend_delivery_secret"));
    assert!(!raw.contains(slt));
    (status, serde_json::from_slice(&body).unwrap())
}
fn delivery_session_request() -> Value {
    json!({"incognito":true,"name":"Delivery integration","description":"synthetic local test only","ttl":"15m"})
}

#[tokio::test]
async fn delivery_routes_require_authentication_and_same_slt_replays_one_enrollment() {
    let fixture = configured_delivery_fixture().await;
    for (method, path, payload) in [
        ("GET", "/api/v1/auth/delivery", None),
        ("POST", "/api/v1/auth/delivery", Some(json!({"short_lived_token":delivery_slt('A')}))),
        ("POST", "/api/v1/auth/delivery/end", None),
    ] {
        let (status, _, _) = request(&fixture.app, method, path, None, payload).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let expected = fixture.identity.identify("oat_owner", "org-1").await.unwrap();
    let slt = delivery_slt('A');
    allow_delivery_exchange(&fixture, &slt, expected);
    for _ in 0..2 {
        let (status, value) = enroll_delivery(&fixture, "oat_owner", &slt).await;
        assert_eq!(status, StatusCode::OK, "{value}");
        assert_eq!(value["data"]["state"], "active");
        assert_eq!(value["data"]["enabled"], true);
    }
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM delivery_credentials").fetch_one(fixture.store.pool()).await.unwrap();
    assert_eq!(count, 1);
    let (status, _, body) =
        request(&fixture.app, "GET", "/api/v1/auth/delivery", Some(("oat_owner", "org-1")), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(data(&body)["actor_id"], "owner-1");
    assert_eq!(data(&body)["enabled"], true);
    let (status, _, body) =
        request(&fixture.app, "POST", "/api/v1/auth/delivery/end", Some(("oat_owner", "org-1")), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(data(&body)["enabled"], false);
    assert_eq!(data(&body)["state"], "revoking");
}

#[tokio::test]
async fn delivery_enrollment_rejects_another_principal_and_queues_only_that_new_family_for_revocation() {
    let fixture = configured_delivery_fixture().await;
    let wrong = fixture.identity.identify("oat_viewer", "org-1").await.unwrap();
    let slt = delivery_slt('B');
    allow_delivery_exchange(&fixture, &slt, wrong);
    let (status, value) = enroll_delivery(&fixture, "oat_owner", &slt).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{value}");
    let row: (i64, String, String) = sqlx::query_as("SELECT enabled,state,operation FROM delivery_credentials")
        .fetch_one(fixture.store.pool())
        .await
        .unwrap();
    assert_eq!(row, (0, "revoking".into(), "revoke".into()));
    assert!(fixture.browser.state.lock().unwrap().browsers.is_empty());
}

#[tokio::test]
async fn configured_session_start_requires_delivery_before_provider_creation_then_persists_identity_binding() {
    let fixture = configured_delivery_fixture().await;
    let (status, _, body) = request(
        &fixture.app,
        "POST",
        "/api/v1/sessions",
        Some(("oat_owner", "org-1")),
        Some(delivery_session_request()),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{}", String::from_utf8_lossy(&body));
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"], "recording_authorization_required");
    assert!(fixture.browser.state.lock().unwrap().browsers.is_empty());
    let session_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sessions").fetch_one(fixture.store.pool()).await.unwrap();
    assert_eq!(session_count, 0);
    let expected = fixture.identity.identify("oat_owner", "org-1").await.unwrap();
    let slt = delivery_slt('C');
    allow_delivery_exchange(&fixture, &slt, expected.clone());
    assert_eq!(enroll_delivery(&fixture, "oat_owner", &slt).await.0, StatusCode::OK);
    let (status, _, body) = request(
        &fixture.app,
        "POST",
        "/api/v1/sessions",
        Some(("oat_owner", "org-1")),
        Some(delivery_session_request()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(fixture.browser.state.lock().unwrap().browsers.len(), 1);
    let binding: (Option<String>, Option<String>) =
        sqlx::query_as("SELECT delivery_principal_id,delivery_membership_id FROM sessions WHERE id=?")
            .bind(data(&body)["id"].as_str().unwrap())
            .fetch_one(fixture.store.pool())
            .await
            .unwrap();
    assert_eq!(binding, (Some(expected.principal_id.to_string()), Some(expected.membership_id.to_string())));
    let mut replaced_membership = expected;
    replaced_membership.membership_id = Uuid::new_v4();
    fixture.identity.allow_identity("oat_owner", replaced_membership);
    let (status, _, body) = request(
        &fixture.app,
        "POST",
        "/api/v1/sessions",
        Some(("oat_owner", "org-1")),
        Some(delivery_session_request()),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{}", String::from_utf8_lossy(&body));
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"], "recording_authorization_required");
    assert_eq!(fixture.browser.state.lock().unwrap().browsers.len(), 1);
}

#[tokio::test]
async fn reused_public_name_cannot_adopt_or_disable_historical_delivery_authority_through_http() {
    let fixture = configured_delivery_fixture().await;
    let old = fixture.identity.identify("oat_owner", "org-1").await.unwrap();
    let slt = delivery_slt('D');
    allow_delivery_exchange(&fixture, &slt, old.clone());
    assert_eq!(enroll_delivery(&fixture, "oat_owner", &slt).await.0, StatusCode::OK);
    // Simulate IAM assigning the former public name to another immutable principal,
    // while an old delivery family remains in Browser's durable outbox/store.
    sqlx::query("DELETE FROM identity_projection WHERE org_id=? AND principal_id=?")
        .bind("org-1")
        .bind(old.principal_id.to_string())
        .execute(fixture.store.pool())
        .await
        .unwrap();
    let mut replacement = old.clone();
    replacement.principal_id = Uuid::new_v4();
    replacement.membership_id = Uuid::new_v4();
    fixture.identity.allow_identity("oat_replacement", replacement);
    let auth = Some(("oat_replacement", "org-1"));
    let (status, _, body) = request(&fixture.app, "GET", "/api/v1/auth/delivery", auth, None).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(data(&body)["enabled"], false);
    let (status, _, body) =
        request(&fixture.app, "POST", "/api/v1/sessions", auth, Some(delivery_session_request())).await;
    assert_eq!(status, StatusCode::CONFLICT, "{}", String::from_utf8_lossy(&body));
    assert!(fixture.browser.state.lock().unwrap().browsers.is_empty());
    let (status, _, _) = request(&fixture.app, "POST", "/api/v1/auth/delivery/end", auth, None).await;
    assert_eq!(status, StatusCode::OK);
    let enabled: bool = sqlx::query_scalar("SELECT enabled FROM delivery_credentials WHERE principal_id=?")
        .bind(old.principal_id.to_string())
        .fetch_one(fixture.store.pool())
        .await
        .unwrap();
    assert!(enabled);
}

async fn failed_retry_fixture(reason: &'static str) -> (Fixture, String) {
    let fixture = configured_delivery_fixture().await;
    let expected = fixture.identity.identify("oat_owner", "org-1").await.unwrap();
    let slt = delivery_slt('R');
    allow_delivery_exchange(&fixture, &slt, expected);
    assert_eq!(enroll_delivery(&fixture, "oat_owner", &slt).await.0, StatusCode::OK);
    let auth = Some(("oat_owner", "org-1"));
    let (status, _, body) =
        request(&fixture.app, "POST", "/api/v1/sessions", auth, Some(delivery_session_request())).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let id = data(&body)["id"].as_str().unwrap().to_owned();
    fixture
        .store
        .associate_participant("org-1", &id, "viewer-1", crate::store::ParticipantRole::Viewer, Utc::now())
        .await
        .unwrap();
    let (status, _, body) = request(
        &fixture.app,
        "POST",
        &format!("/api/v1/sessions/{id}/end"),
        auth,
        Some(json!({"note":"local retry test"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let now = Utc::now();
    let claims = fixture.store.claim_recording_deliveries(now, now + TimeDelta::seconds(60), 8).await.unwrap();
    assert!(!claims.is_empty());
    for claim in claims {
        assert_eq!(claim.session_id, id);
        fixture.store.fail_recording_delivery(&claim, reason, now).await.unwrap();
    }
    (fixture, id)
}

#[tokio::test]
async fn recording_retry_http_requires_owner_and_active_grant_then_preserves_claim_attempts() {
    let (fixture, id) = failed_retry_fixture("delivery_attempts_exhausted").await;
    let path = format!("/api/v1/recordings/{id}/retry");
    assert_eq!(request(&fixture.app, "POST", &path, None, None).await.0, StatusCode::UNAUTHORIZED);
    // Prove the viewer can see the exact recording but cannot write into its owner's Briefcase.
    assert_eq!(
        request(&fixture.app, "GET", &format!("/api/v1/recordings/{id}"), Some(("oat_viewer", "org-1")), None).await.0,
        StatusCode::OK
    );
    let (status, _, body) = request(&fixture.app, "POST", &path, Some(("oat_viewer", "org-1")), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"], "recording_owner_required");
    let auth = Some(("oat_owner", "org-1"));
    let (status, _, body) = request(&fixture.app, "POST", &path, auth, None).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(data(&body)["status"], "pending");
    let now = Utc::now();
    let claims = fixture.store.claim_recording_deliveries(now, now + TimeDelta::seconds(60), 8).await.unwrap();
    assert!(!claims.is_empty());
    assert!(claims.iter().all(|claim| claim.attempt == 1));
    let before: Vec<(String, i64, Option<String>)> =
        sqlx::query_as("SELECT state,attempts,lease_id FROM recording_artifacts WHERE session_id=? ORDER BY kind")
            .bind(&id)
            .fetch_all(fixture.store.pool())
            .await
            .unwrap();
    assert_eq!(request(&fixture.app, "POST", &path, auth, None).await.0, StatusCode::OK);
    let after: Vec<(String, i64, Option<String>)> =
        sqlx::query_as("SELECT state,attempts,lease_id FROM recording_artifacts WHERE session_id=? ORDER BY kind")
            .bind(&id)
            .fetch_all(fixture.store.pool())
            .await
            .unwrap();
    assert_eq!(after, before, "a duplicate request must not reset claimed work or its attempt count");
    assert_eq!(request(&fixture.app, "POST", "/api/v1/auth/delivery/end", auth, None).await.0, StatusCode::OK);
    let (status, _, body) = request(&fixture.app, "POST", &path, auth, None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"], "recording_authorization_required");
}

#[tokio::test]
async fn recording_retry_http_rejects_historical_principal_or_membership_even_with_active_current_grant() {
    for query in [
        "UPDATE sessions SET delivery_principal_id=? WHERE id=?",
        "UPDATE sessions SET delivery_membership_id=? WHERE id=?",
    ] {
        let (fixture, id) = failed_retry_fixture("recording_size_limit").await;
        // The current owner has an active grant, but this recording belongs to a previous
        // immutable IAM binding. A reused public identity is insufficient to adopt it.
        sqlx::query(query).bind(Uuid::new_v4().to_string()).bind(&id).execute(fixture.store.pool()).await.unwrap();
        let auth = Some(("oat_owner", "org-1"));
        let (status, _, body) = request(&fixture.app, "GET", "/api/v1/auth/delivery", auth, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(data(&body)["state"], "active");
        let (status, _, body) =
            request(&fixture.app, "POST", &format!("/api/v1/recordings/{id}/retry"), auth, None).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"], "recording_not_retryable");
        let pending: i64 =
            sqlx::query_scalar("SELECT count(*) FROM recording_artifacts WHERE session_id=? AND state <> 'failed'")
                .bind(&id)
                .fetch_one(fixture.store.pool())
                .await
                .unwrap();
        assert_eq!(pending, 0);
    }
}

#[tokio::test]
async fn recording_retry_http_keeps_permanent_source_failure_terminal() {
    let (fixture, id) = failed_retry_fixture("native_recording_unavailable").await;
    let (status, _, body) =
        request(&fixture.app, "POST", &format!("/api/v1/recordings/{id}/retry"), Some(("oat_owner", "org-1")), None)
            .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"], "recording_not_retryable");
}

#[tokio::test]
async fn only_pre_handler_auth_rejections_permit_credential_refresh_and_retry() {
    let fixture = configured_delivery_fixture().await;
    let (status, headers, _) = request(&fixture.app, "POST", "/api/v1/sessions", Some(("oat_unknown", "org-1")), Some(delivery_session_request())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(headers.get("x-sb-auth-rejected").unwrap(), "1");
    assert!(fixture.browser.state.lock().unwrap().browsers.is_empty());
    let (status, headers, _) = request(&fixture.app, "POST", "/api/v1/auth/delivery", Some(("oat_owner", "org-1")), Some(json!({"short_lived_token":delivery_slt('Z')}))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(headers.get("x-sb-auth-rejected").is_none(), "an exchange may have side effects; never retry this handler rejection automatically");
}
