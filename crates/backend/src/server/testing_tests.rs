use super::*;
use base64::Engine as _;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Default)]
struct TestIam {
    cleaned: AtomicBool,
    revoked: AtomicBool,
    verifications: AtomicUsize,
}

fn secret(letter: char) -> String {
    format!("ask_{}", letter.to_string().repeat(43))
}

async fn iam_stub(State(state): State<Arc<TestIam>>, request: axum::extract::Request) -> Response {
    let selector =
        request.headers().get("x-testing-application").expect("every IAM call must select testing").to_str().unwrap();
    assert!(!request.headers().contains_key("x-testing-environment-key"));
    let decoded = base64::engine::general_purpose::STANDARD.decode(selector.strip_prefix("Basic ").unwrap()).unwrap();
    let decoded = String::from_utf8(decoded).unwrap();
    let supplied = decoded.strip_prefix("browser:").expect("configured Browser application");
    let environment = if supplied == secret('B') { Uuid::from_u128(20) } else { Uuid::from_u128(10) };
    let path = request.uri().path();
    if path == "/api/version" {
        return Json(json!({"service":"silicon-iam","selected_api_version":"v1","supported_api_versions":["v1"],"build":"test","commit":"test"})).into_response();
    }
    assert_eq!(request.headers().get(AUTHORIZATION).unwrap().to_str().unwrap(), selector);
    if state.revoked.load(Ordering::SeqCst) || !['A', 'B', 'R'].iter().any(|letter| supplied == secret(*letter)) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":{"code":"unauthenticated","message":"test secret rejected"}})),
        )
            .into_response();
    }
    if path == "/api/v1/application/testing-context" {
        state.verifications.fetch_add(1, Ordering::SeqCst);
        let mut metadata = json!({"environment_id":environment,"org_id":"org-1","name":"Browser test","version":1,
            "key_generation":if supplied == secret('R') { 2 } else { 1 },"created_at":"2026-09-12T00:00:00Z",
            "creator_type":"application","creator_id":"browser"});
        if state.cleaned.load(Ordering::SeqCst) {
            metadata["cleaned_at"] = json!("2026-09-14T00:00:00Z");
        }
        return Json(json!({"environment_id":environment,
            "application":{"app_id":"browser","base_url":"https://browser.example","app_scope":{"iam":[],"external":[]},"webhook_scope":[],"testing_idle_days":30},
            "environment":metadata})).into_response();
    }
    assert_eq!(path, "/api/v1/oauth/introspect");
    Json(json!({"active":true,"public_id":"si:owner-1","actor_type":"silicon","client_id":"browser",
        "org_id":"org-1","membership_id":"si:owner-1[org-1]","session_id":Uuid::from_u128(3),
        "scope":"self.identity.read self.tags.read","audience":"browser","issued_at":Utc::now().timestamp()-1,
        "expires_at":Utc::now().timestamp()+1800,"authorization_epoch":7,
        "authorization":{"actor_type":"silicon","public_id":"si:owner-1",
            "organization_id":Uuid::from_u128(4),"org_id":"org-1","membership_id":"si:owner-1[org-1]",
            "membership_version":1,"authorization_epoch":7,"audience":"browser",
            "testing_environment_id":environment,"scopes":["self.identity.read","self.tags.read"],"tags":[]}}))
    .into_response()
}

async fn test_request(
    app: &Router,
    method: &str,
    path: &str,
    secret: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(AUTHORIZATION, format!("Bearer oat_{}", "T".repeat(43)))
        .header("x-org-id", "org-1");
    if let Some(secret) = secret {
        builder = builder.header("x-sb-test-app-secret", secret);
    }
    if body.is_some() {
        builder = builder.header(CONTENT_TYPE, "application/json");
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body.map(|body| body.to_string()).unwrap_or_default())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn test_routes_verify_before_storage_and_isolate_environments_cleaning_and_secret_rotation() {
    let fixture = fixture().await;
    let directory = tempfile::tempdir().unwrap();
    let iam = Arc::new(TestIam::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mock = Router::new().fallback(iam_stub).with_state(iam.clone());
    let worker = tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });
    let values = HashMap::from([
        ("SB_ORIGIN", "https://browser.example".to_owned()),
        ("SB_DATABASE_URL", format!("sqlite://{}?mode=rwc", directory.path().join("production.db").display())),
        ("SB_ENCRYPTION_KEY", "07".repeat(32)),
        ("IAM_APP_ID", "browser".to_owned()),
        ("IAM_APP_SECRET", secret('P')),
        ("BROWSER_USE_API_KEY", "provider-test".to_owned()),
        ("SILICON_IAM_URL", base),
    ]);
    let config = crate::config::Config::from_vars(|key| values.get(key).cloned()).unwrap();
    let registry = TestingRegistry::new(config.clone(), fixture.state.clone()).unwrap();
    let app = production_router_with_testing(fixture.state.clone(), registry);
    let storage = directory.path().join("production.db.testing");
    let a = format!("/testing/{}/api/v1/profiles", Uuid::from_u128(10));
    let b = format!("/testing/{}/api/v1/profiles", Uuid::from_u128(20));

    let (status, body) = test_request(&app, "GET", &a, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "test_credentials_required");
    let (status, _) = test_request(&app, "GET", &a, Some("ask_bad\tsecret"), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(iam.verifications.load(Ordering::SeqCst), 0);
    let (status, _) = test_request(&app, "GET", &a, Some(&secret('X')), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = test_request(&app, "GET", &a, Some(&secret('B')), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = test_request(&app, "GET", "/api/v1/profiles", Some(&secret('A')), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "test_route_required");
    assert_eq!(std::fs::read_dir(&storage).unwrap().count(), 0);
    let registrations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM testing_environments").fetch_one(fixture.store.pool()).await.unwrap();
    assert_eq!(registrations, 0);

    let verified_before_legacy_secret = iam.verifications.load(Ordering::SeqCst);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/testing/{}/api/v1/iam", Uuid::from_u128(10)))
                .header("x-sb-test-app-secret", secret('A'))
                .header("x-sb-test-briefcase-key", "K".repeat(32))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(iam.verifications.load(Ordering::SeqCst), verified_before_legacy_secret);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/testing/{}/api/v1/iam", Uuid::from_u128(10)))
                .header("x-sb-test-app-secret", secret('A'))
                .header("x-sb-test-briefcase-key", format!("ask_{}", "K".repeat(43)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let (status, profile) = test_request(
        &app,
        "POST",
        &a,
        Some(&secret('A')),
        Some(json!({"name":"Test A profile","location":"us","access":[]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{profile}");
    let profile_id = profile["data"]["id"].as_str().unwrap();
    for (path, credential, count) in [(&a, 'A', 1), (&a, 'R', 1), (&b, 'B', 0)] {
        let (status, profiles) = test_request(&app, "GET", path, Some(&secret(credential)), None).await;
        assert_eq!(status, StatusCode::OK, "{profiles}");
        assert_eq!(profiles["data"].as_array().unwrap().len(), count);
    }
    let (status, selected) = test_request(&app, "GET", &format!("{a}/{profile_id}"), Some(&secret('A')), None).await;
    assert_eq!(status, StatusCode::OK, "{selected}");
    assert_eq!(selected["data"]["id"], profile_id);
    assert!(fixture.store.profiles("org-1", &owner()).await.unwrap().is_empty());
    let registrants: Vec<String> = sqlx::query_scalar("SELECT credentials FROM testing_environments")
        .fetch_all(fixture.store.pool())
        .await
        .unwrap();
    assert_eq!(registrants.len(), 2, "key rotation must preserve the environment database");
    assert!(registrants.iter().all(|encrypted| !encrypted.contains("ask_")));
    let (namespace, encrypted): (String, String) =
        sqlx::query_as("SELECT namespace, credentials FROM testing_environments WHERE environment_id = ?")
            .bind(Uuid::from_u128(10).to_string())
            .fetch_one(fixture.store.pool())
            .await
            .unwrap();
    let persisted: Value =
        serde_json::from_str(&fixture.state.secrets.open_for(&format!("testing:{namespace}"), &encrypted).unwrap())
            .unwrap();
    assert_eq!(
        persisted["briefcase_test_environment_key"],
        format!("ask_{}", "K".repeat(43)),
        "secret-only requests must preserve configured recording delivery"
    );

    drop(app);
    let restarted = TestingRegistry::new(config, fixture.state.clone()).unwrap();
    let app = production_router_with_testing(fixture.state.clone(), restarted);
    let (status, restored) = test_request(&app, "GET", &a, Some(&secret('R')), None).await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(restored["data"].as_array().unwrap().len(), 1, "a fresh registry must reopen persistent test data");
    assert_eq!(restored["data"][0]["id"], profile_id);

    iam.cleaned.store(true, Ordering::SeqCst);
    let (status, profiles) = test_request(&app, "GET", &a, Some(&secret('R')), None).await;
    assert_eq!(status, StatusCode::OK, "{profiles}");
    assert_eq!(profiles["data"], json!([]));
    let generations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM testing_environments").fetch_one(fixture.store.pool()).await.unwrap();
    assert_eq!(generations, 3, "IAM clean must get a fresh generation without reusing old data");
    iam.revoked.store(true, Ordering::SeqCst);
    let (status, _) = test_request(&app, "GET", &a, Some(&secret('R')), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "cached database must not bypass fresh IAM verification");
    worker.abort();
}
