use super::*;
use crate::delivery_auth::obo::ENDPOINTS;
use crate::providers::BriefcaseClient;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

fn pair(endpoint: &str, testing: bool) -> Value {
    json!({"grant_id":Uuid::new_v4(),"token_id":Uuid::new_v4(),"access_token":format!("oba_{endpoint}"),"refresh_token":format!("obr_{endpoint}"),"token_type":"Bearer","expires_in":1800,"expires_at":"2099-01-01T00:00:00Z","audience":"briefcase","endpoint_id":endpoint,"org_id":"chosen-storage","actor":{"type":"silicon","public_id":"si:chosen"},"scope":"","testing_context":testing.then(||json!({"app_id":"briefcase","app_secret":format!("ask_{}","B".repeat(43)),"iam_test_key":"I".repeat(32)}))})
}
async fn configured_delivery_fixture() -> (Fixture, MockServer) {
    let mut fixture = fixture().await;
    let iam = MockServer::start().await;
    fixture.identity.allow_recording_client(
        silicon_iam_client::Client::builder(&iam.uri())
            .unwrap()
            .credential(silicon_iam_client::Credential::application("browser", "test-secret"))
            .build()
            .unwrap(),
        None,
    );
    fixture.state = fixture
        .state
        .clone()
        .with_recording_delivery(
            BriefcaseClient::new("http://127.0.0.1:1", None).unwrap(),
            "browser".into(),
            "briefcase".into(),
        )
        .unwrap();
    fixture.app = router(fixture.state.clone());
    Mock::given(path("/api/v1/obo-access/tokens"))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let endpoint = body["refresh_token"].as_str().unwrap_or("").strip_prefix("obr_").unwrap_or("");
            if ENDPOINTS.contains(&endpoint) {
                ResponseTemplate::new(200).set_body_json(json!({"items":[pair(endpoint,false)]}))
            } else {
                ResponseTemplate::new(400).set_body_json(json!({"error":{"code":"invalid_grant","message":"invalid"}}))
            }
        })
        .mount(&iam)
        .await;
    (fixture, iam)
}
async fn seed_grant(fixture: &Fixture, expected: &PrincipalIdentity, testing: bool) {
    let context = format!(
        "recording-obo/{}/{}/{}/{}/tokens",
        fixture.identity.recording_environment().map_or_else(|| "production".into(), |id| id.to_string()),
        expected.org_id,
        expected.principal_id,
        expected.membership_id
    );
    let cipher = fixture
        .state
        .secrets
        .seal_for(&context, &json!(ENDPOINTS.map(|endpoint| pair(endpoint, testing))).to_string())
        .unwrap();
    sqlx::query(
        "INSERT INTO recording_obo_grants(org_id,principal_id,membership_id,actor_id,tokens_cipher) VALUES(?,?,?,?,?)",
    )
    .bind(&expected.org_id)
    .bind(&expected.principal_id)
    .bind(&expected.membership_id)
    .bind(expected.public_id.as_deref().unwrap())
    .bind(cipher)
    .execute(fixture.store.pool())
    .await
    .unwrap();
}
fn delivery_session_request() -> Value {
    json!({"incognito":true,"name":"Delivery integration","description":"synthetic local test only","ttl":"15m"})
}
async fn consent_call(fixture: &Fixture, path: &str, body: Value) -> (StatusCode, Value) {
    use tower::ServiceExt as _;
    let response = fixture
        .app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(path)
                .header("authorization", "Bearer oat_owner")
                .header("x-org-id", "org-1")
                .header("idempotency-key", "browser-feature-consent-stable")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 65536).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
#[tokio::test]
async fn feature_consent_is_explicit_bound_encrypted_and_bad_code_does_not_log_out() {
    let (fixture, iam) = configured_delivery_fixture().await;
    let id = Uuid::new_v4();
    Mock::given(path("/api/v1/obo-access/authorizations")).respond_with(ResponseTemplate::new(201).set_body_json(json!({"id":id,"app_id":"browser","app_name":"Browser","actor":{"type":"silicon","public_id":"si:owner-1"},"org_id":"org-1","status":"pending","version":1,"expires_at":"2099-01-01T00:00:00Z","endpoints":[],"authorization_url":format!("{}/obo/consent?request={id}",iam.uri())}))).expect(1).mount(&iam).await;
    let (status, started) = consent_call(&fixture, "/api/v1/auth/delivery/authorizations", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{started}");
    assert_eq!(consent_call(&fixture, "/api/v1/auth/delivery/authorizations", json!({})).await.1, started);
    assert!(!started.to_string().contains("oat_"));
    let route = format!(
        "/api/v1/auth/delivery/authorizations/{}/complete",
        started["data"]["authorization_id"].as_str().unwrap()
    );
    let state = &started["data"]["state"];
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = requests.clone();
    Mock::given(path("/api/v1/obo-access/tokens"))
        .respond_with(move |request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            captured.lock().unwrap().push(request.headers["idempotency-key"].to_str().unwrap().to_owned());
            if body["authorization_code"] == "obc_wrong" {
                ResponseTemplate::new(401).set_body_json(json!({"error":{"code":"invalid_grant","message":"bad code"}}))
            } else {
                ResponseTemplate::new(200).set_body_json(json!({"items":ENDPOINTS.map(|endpoint|pair(endpoint,false))}))
            }
        })
        .with_priority(1)
        .expect(3)
        .mount(&iam)
        .await;
    for _ in 0..2 {
        let (status, error) = consent_call(&fixture, &route, json!({"code":"obc_wrong","state":state})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error["error"]["code"], "invalid_recording_consent");
    }
    let (status, completed) = consent_call(&fixture, &route, json!({"code":"obc_correct","state":state})).await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(consent_call(&fixture, &route, json!({"code":"obc_correct","state":state})).await.1, completed);
    {
        let keys = requests.lock().unwrap();
        assert_eq!(keys[0], keys[1]);
        assert_ne!(keys[1], keys[2]);
    }
    let cipher: String = sqlx::query_scalar("SELECT tokens_cipher FROM recording_obo_grants")
        .fetch_one(fixture.store.pool())
        .await
        .unwrap();
    assert!(!cipher.contains("obr_"));
    assert!(!cipher.contains("oba_"));
    let route = route.trim_end_matches("/complete");
    assert_eq!(request(&fixture.app, "GET", route, Some(("oat_viewer", "org-1")), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(request(&fixture.app, "GET", "/api/v1/me", Some(("oat_owner", "org-1")), None).await.0, StatusCode::OK);
    assert_eq!(
        request(
            &fixture.app,
            "POST",
            "/api/v1/auth/delivery",
            Some(("oat_owner", "org-1")),
            Some(json!({"short_lived_token":"oac_legacy"}))
        )
        .await
        .0,
        StatusCode::GONE
    );
}
#[tokio::test]
async fn feature_consent_rejects_misbound_or_expired_iam_response_before_saving_it() {
    for (field, invalid) in [
        ("id", json!(Uuid::nil())),
        ("app_id", json!("other-app")),
        ("org_id", json!("other-org")),
        ("actor", json!({"type":"silicon","public_id":"si:other"})),
        ("actor", json!({"type":"carbon","public_id":"si:owner-1"})),
        ("expires_at", json!("2000-01-01T00:00:00Z")),
    ] {
        let (fixture, iam) = configured_delivery_fixture().await;
        let id = Uuid::new_v4();
        let mut detail = json!({"id":id,"app_id":"browser","actor":{"type":"silicon","public_id":"si:owner-1"},"org_id":"org-1","status":"pending","version":1,"expires_at":"2099-01-01T00:00:00Z","endpoints":[],"authorization_url":format!("{}/obo/consent?request={id}",iam.uri())});
        detail[field] = invalid;
        Mock::given(path("/api/v1/obo-access/authorizations"))
            .respond_with(ResponseTemplate::new(201).set_body_json(detail))
            .expect(1)
            .mount(&iam)
            .await;
        let (status, error) = consent_call(&fixture, "/api/v1/auth/delivery/authorizations", json!({})).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{field}: {error}");
        let row: (Option<String>, Option<String>, String) =
            sqlx::query_as("SELECT iam_id,consent_url,request_cipher FROM recording_obo_authorizations")
                .fetch_one(fixture.store.pool())
                .await
                .unwrap();
        assert_eq!(row.0, None, "{field}");
        assert_eq!(row.1, None, "{field}");
        assert!(!row.2.contains("oat_owner"));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recording_obo_grants")
            .fetch_one(fixture.store.pool())
            .await
            .unwrap();
        assert_eq!(count, 0, "{field}");
        assert_eq!(
            request(&fixture.app, "GET", "/api/v1/me", Some(("oat_owner", "org-1")), None).await.0,
            StatusCode::OK
        );
    }
}
#[tokio::test]
async fn paid_session_revalidates_dedicated_storage_and_stops_on_revocation() {
    let (fixture, iam) = configured_delivery_fixture().await;
    let expected = fixture.identity.identify("oat_owner", "org-1").await.unwrap();
    seed_grant(&fixture, &expected, false).await;
    let auth = Some(("oat_owner", "org-1"));
    assert_eq!(
        request(&fixture.app, "POST", "/api/v1/sessions", auth, Some(delivery_session_request())).await.0,
        StatusCode::OK
    );
    assert_eq!(fixture.browser.state.lock().unwrap().browsers.len(), 1);
    iam.reset().await;
    Mock::given(path("/api/v1/obo-access/tokens"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"error":{"code":"obo_access_token_invalid","message":"revoked"}})),
        )
        .expect(1)
        .mount(&iam)
        .await;
    let (status, _, body) =
        request(&fixture.app, "POST", "/api/v1/sessions", auth, Some(delivery_session_request())).await;
    assert_eq!(status, StatusCode::CONFLICT, "{}", String::from_utf8_lossy(&body));
    assert_eq!(fixture.browser.state.lock().unwrap().browsers.len(), 1);
    let (_, _, body) = request(&fixture.app, "GET", "/api/v1/auth/delivery", auth, None).await;
    assert_eq!(data(&body)["state"], "needs_auth");
}
#[tokio::test]
async fn missing_feature_grant_is_not_a_login_failure() {
    let (fixture, _iam) = configured_delivery_fixture().await;
    let (status, headers, body) = request(
        &fixture.app,
        "POST",
        "/api/v1/sessions",
        Some(("oat_owner", "org-1")),
        Some(delivery_session_request()),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{}", String::from_utf8_lossy(&body));
    assert!(headers.get("x-sb-auth-rejected").is_none());
    assert!(fixture.browser.state.lock().unwrap().browsers.is_empty());
    let (status, headers, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/sessions",
        Some(("oat_unknown", "org-1")),
        Some(delivery_session_request()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(headers["x-sb-auth-rejected"], "1");
}
async fn failed_retry_fixture(reason: &'static str) -> (Fixture, String, wiremock::MockServer) {
    let (fixture, iam) = configured_delivery_fixture().await;
    let expected = fixture.identity.identify("oat_owner", "org-1").await.unwrap();
    seed_grant(&fixture, &expected, false).await;
    let auth = Some(("oat_owner", "org-1"));
    let (status, _, body) =
        request(&fixture.app, "POST", "/api/v1/sessions", auth, Some(delivery_session_request())).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let id = data(&body)["id"].as_str().unwrap().to_owned();
    fixture
        .store
        .associate_participant("org-1", &id, "c:viewer-1", crate::store::ParticipantRole::Viewer, Utc::now())
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
    (fixture, id, iam)
}

#[tokio::test]
async fn recording_retry_http_requires_owner_and_active_grant_then_preserves_claim_attempts() {
    let (fixture, id, _iam) = failed_retry_fixture("delivery_attempts_exhausted").await;
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
        let (fixture, id, _iam) = failed_retry_fixture("recording_size_limit").await;
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
    let (fixture, id, _iam) = failed_retry_fixture("native_recording_unavailable").await;
    let (status, _, body) =
        request(&fixture.app, "POST", &format!("/api/v1/recordings/{id}/retry"), Some(("oat_owner", "org-1")), None)
            .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"], "recording_not_retryable");
}

#[tokio::test]
async fn concurrent_refresh_survives_uncertain_response_and_ciphertext_cannot_cross_test_worlds() {
    let (fixture, iam) = configured_delivery_fixture().await;
    let expected = fixture.identity.identify("oat_owner", "org-1").await.unwrap();
    let env = Uuid::new_v4();
    let sdk = silicon_iam_client::Client::builder(&iam.uri())
        .unwrap()
        .credential(silicon_iam_client::Credential::application("browser", "test-secret"))
        .build()
        .unwrap()
        .with_environment(silicon_iam_client::EnvironmentKey::new("T".repeat(32)).unwrap());
    fixture.identity.allow_recording_client(sdk.clone(), Some(env));
    let context = format!("recording-obo/{env}/org-1/{}/{}/tokens", expected.principal_id, expected.membership_id);
    let expired = ENDPOINTS.map(|endpoint| {
        let mut value = pair(endpoint, true);
        value["expires_at"] = json!("2000-01-01T00:00:00Z");
        value
    });
    let cipher = fixture.state.secrets.seal_for(&context, &json!(expired).to_string()).unwrap();
    sqlx::query(
        "INSERT INTO recording_obo_grants(org_id,principal_id,membership_id,actor_id,tokens_cipher) VALUES(?,?,?,?,?)",
    )
    .bind("org-1")
    .bind(&expected.principal_id)
    .bind(&expected.membership_id)
    .bind("si:owner-1")
    .bind(cipher)
    .execute(fixture.store.pool())
    .await
    .unwrap();
    let broker = crate::delivery_auth::DeliveryAuth::new(
        fixture.store.clone(),
        fixture.state.secrets.clone(),
        Arc::new(fixture.identity.clone()),
        "briefcase".into(),
    );
    iam.reset().await;
    let keys = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = keys.clone();
    Mock::given(path("/api/v1/obo-access/tokens"))
        .respond_with(move |request: &wiremock::Request| {
            captured.lock().unwrap().push(request.headers["idempotency-key"].to_str().unwrap().to_owned());
            ResponseTemplate::new(503).set_body_json(json!({"error":{"code":"unavailable","message":"lost response"}}))
        })
        .expect(1)
        .mount(&iam)
        .await;
    assert!(
        broker
            .recording_tokens("org-1", "si:owner-1", &expected.principal_id, &expected.membership_id, false)
            .await
            .is_err()
    );
    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT needs_auth FROM recording_obo_grants")
            .fetch_one(fixture.store.pool())
            .await
            .unwrap()
    );
    iam.reset().await;
    let captured = keys.clone();
    Mock::given(path("/api/v1/obo-access/tokens"))
        .respond_with(move |request: &wiremock::Request| {
            assert_eq!(request.headers["x-testing-environment-key"], "T".repeat(32));
            captured.lock().unwrap().push(request.headers["idempotency-key"].to_str().unwrap().to_owned());
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let endpoint = body["refresh_token"].as_str().unwrap().strip_prefix("obr_").unwrap();
            ResponseTemplate::new(200).set_body_json(json!({"items":[pair(endpoint,true)]}))
        })
        .with_priority(1)
        .expect(3)
        .mount(&iam)
        .await;
    let (one, two) = tokio::join!(
        broker.recording_tokens("org-1", "si:owner-1", &expected.principal_id, &expected.membership_id, false),
        broker.recording_tokens("org-1", "si:owner-1", &expected.principal_id, &expected.membership_id, false)
    );
    assert_eq!(one.unwrap().org_id, "chosen-storage");
    assert_eq!(two.unwrap().actor_id, "si:chosen");
    {
        let keys = keys.lock().unwrap();
        assert_eq!(keys.len(), 4);
        assert_eq!(keys[0], keys[1]);
    }
    fixture.identity.allow_recording_client(sdk, Some(Uuid::new_v4()));
    assert!(matches!(
        broker.recording_tokens("org-1", "si:owner-1", &expected.principal_id, &expected.membership_id, false).await,
        Err(crate::delivery_auth::DeliveryAuthError::Storage)
    ));
    assert!(matches!(
        broker.recording_tokens("org-1", "si:owner-1", "other-principal", &expected.membership_id, false).await,
        Err(crate::delivery_auth::DeliveryAuthError::NeedsAuthorization)
    ));
}

#[tokio::test]
async fn delayed_rejection_cannot_invalidate_a_replaced_approval() {
    let (fixture, _iam) = configured_delivery_fixture().await;
    let expected = fixture.identity.identify("oat_owner", "org-1").await.unwrap();
    seed_grant(&fixture, &expected, false).await;
    let broker = crate::delivery_auth::DeliveryAuth::new(
        fixture.store.clone(),
        fixture.state.secrets.clone(),
        Arc::new(fixture.identity.clone()),
        "briefcase".into(),
    );
    let old = broker
        .recording_tokens("org-1", "si:owner-1", &expected.principal_id, &expected.membership_id, false)
        .await
        .unwrap();
    // Simulate successful reapproval while the old provider request is in flight.
    let replacement = ENDPOINTS.map(|endpoint| pair(endpoint, false));
    let cipher = fixture
        .state
        .secrets
        .seal_for(
            &format!("recording-obo/production/org-1/{}/{}/tokens", expected.principal_id, expected.membership_id),
            &json!(replacement).to_string(),
        )
        .unwrap();
    sqlx::query("UPDATE recording_obo_grants SET tokens_cipher=?,credential_version='newly-approved',needs_auth=0")
        .bind(cipher)
        .execute(fixture.store.pool())
        .await
        .unwrap();
    broker
        .invalidate_storage("org-1", &expected.principal_id, &expected.membership_id, &old.credential_version)
        .await
        .unwrap();
    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT needs_auth FROM recording_obo_grants")
            .fetch_one(fixture.store.pool())
            .await
            .unwrap()
    );
    let current = broker
        .recording_tokens("org-1", "si:owner-1", &expected.principal_id, &expected.membership_id, false)
        .await
        .unwrap();
    assert_ne!(old.credential_version, current.credential_version);
    broker
        .invalidate_storage("org-1", &expected.principal_id, &expected.membership_id, &current.credential_version)
        .await
        .unwrap();
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT needs_auth FROM recording_obo_grants")
            .fetch_one(fixture.store.pool())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn popup_consent_binds_fixed_frontend_callback_and_immutable_state() {
    let (fixture, iam) = configured_delivery_fixture().await;
    let id = Uuid::new_v4();
    Mock::given(path("/api/v1/obo-access/authorizations")).respond_with(ResponseTemplate::new(201).set_body_json(json!({"id":id,"app_id":"browser","actor":{"type":"silicon","public_id":"si:owner-1"},"org_id":"org-1","status":"pending","version":1,"expires_at":"2099-01-01T00:00:00Z","endpoints":[],"authorization_url":format!("{}/obo/consent?request={id}",iam.uri())}))).expect(1).mount(&iam).await;
    let (status, started) = consent_call(&fixture,"/api/v1/auth/delivery/authorizations",json!({"popup":true})).await;
    assert_eq!(status,StatusCode::OK,"{started}");
    let calls=iam.received_requests().await.unwrap(); let body:Value=serde_json::from_slice(&calls[0].body).unwrap();
    assert_eq!(body["redirect_uri"],format!("{}/auth/obo/callback",fixture.state.public_origin));
    assert_eq!(body["state"],started["data"]["state"]);
    assert_eq!(body["state"].as_str().unwrap().len(),64);
    assert_eq!(consent_call(&fixture,"/api/v1/auth/delivery/authorizations",json!({"popup":true})).await.1,started);
    assert_eq!(consent_call(&fixture,"/api/v1/auth/delivery/authorizations",json!({})).await.0,StatusCode::BAD_REQUEST);
}
