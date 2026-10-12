// Direct browser capabilities and removal of remote command reporting.
// Every provider is fake: this suite starts no browser and makes no network call.
use super::*;

async fn start(fixture: &Fixture, carbon: bool, profile: bool) -> Session {
    let token = if carbon { "oat_viewer" } else { "oat_owner" };
    let profile_id = if profile {
        let (status, _, body) = request(
            &fixture.app,
            "POST",
            "/api/v1/profiles",
            Some((token, "browser")),
            Some(json!({"name":"Shared","location":"in","access":["@c:viewer-1"]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        Some(data(&body)["id"].as_str().unwrap().to_owned())
    } else {
        None
    };
    let payload = if let Some(id) = profile_id {
        json!({"profile_id":id,"name":"Local controller","description":"Fixture","ttl":"15m"})
    } else {
        json!({"incognito":true,"name":"Local controller","description":"Fixture","ttl":"15m"})
    };
    let (status, _, body) =
        request(&fixture.app, "POST", "/api/v1/sessions", Some((token, "browser")), Some(payload)).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    serde_json::from_value(data(&body)).unwrap()
}

fn report() -> Value {
    let now = Utc::now();
    json!({"command_id":Uuid::now_v7(), "command":"fill @e2 'local-secret'", "flags":["--json"],
        "started_at":now,"finished_at":now,"exit_code":0,"truncated":false})
}

async fn send(fixture: &Fixture, session: &Session, body: Value) -> (StatusCode, Value) {
    let (status, _, body) = request(
        &fixture.app,
        "POST",
        &format!("/api/v1/sessions/{}/commands", session.id),
        Some(("oat_owner", "browser")),
        Some(body),
    )
    .await;
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn direct_connection_is_secret_no_store_metadata_with_no_provider_execution() {
    let fixture = fixture().await;
    let session = start(&fixture, false, false).await;
    let path = format!("/api/v1/sessions/{}/connection", session.id);
    let (status, headers, body) = request(&fixture.app, "GET", &path, Some(("oat_owner", "browser")), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["cache-control"], "no-store");
    let connection: SessionConnection = serde_json::from_value(data(&body)).unwrap();
    assert_eq!(connection.session_id, session.id);
    assert_eq!(connection.expires_at, session.expires_at);
    assert_eq!(connection.principal_id, fixture.identity.identify("oat_owner").await.unwrap().principal_id.to_string());
    assert!(connection.cdp_url.starts_with("wss://provider.invalid/cdp/"));
    assert!(!format!("{connection:?}").contains("secret=yes"));
    assert_eq!(fixture.browser.state.lock().unwrap().get_calls, 0);
    assert!(fixture.browser.stopped().is_empty());
    assert_eq!(
        request(&fixture.app, "GET", &path, Some(("oat_owner", "obsolete-header")), None).await.0,
        StatusCode::OK
    );
    for auth in [None, Some(("oat_viewer", "browser"))] {
        let (status, _, body) = request(&fixture.app, "GET", &path, auth, None).await;
        assert!(!status.is_success());
        assert!(!String::from_utf8_lossy(&body).contains("provider.invalid"));
    }
}

#[tokio::test]
async fn connection_preserves_https_cdp_discovery_endpoint_accepted_at_creation() {
    let fixture = fixture().await;
    let now = Utc::now();
    let session = fixture
        .store
        .reserve_session(
            "browser",
            &owner(),
            &SessionCreate::incognito(
                "HTTPS discovery",
                "Real provider scheme regression",
                silicon_browser_shared::SessionTtl::Minutes15,
            ),
            now,
        )
        .await
        .unwrap();
    let mut browser = fake_provider_browser("https-discovery", false);
    let url = "https://cdp.provider.invalid/?token=fixture-secret";
    browser.cdp_url = Some(url.into());
    let runtime = provider_session(&browser).expect("creation accepts secure discovery endpoints");
    fixture.store.activate_session("browser", &session.id, &runtime, &fixture.state.secrets, now).await.unwrap();
    let (status, headers, body) = request(
        &fixture.app,
        "GET",
        &format!("/api/v1/sessions/{}/connection", session.id),
        Some(("oat_owner", "browser")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(headers["cache-control"], "no-store");
    assert_eq!(data(&body)["cdp_url"], url);
    assert_eq!(fixture.browser.state.lock().unwrap().get_calls, 0);
}

#[tokio::test]
async fn connection_renewal_denies_removed_profile_participants_and_ending_or_expired_sessions() {
    let fixture = fixture().await;
    let session = start(&fixture, false, true).await;
    let path = format!("/api/v1/sessions/{}/connection", session.id);
    fixture.store.associate_participant("browser", &session.id, "c:viewer-1",
        crate::store::ParticipantRole::Runner, Utc::now()).await.unwrap();
    let (status, _, _) = request(&fixture.app, "GET", &path, Some(("oat_viewer", "browser")), None).await;
    assert_eq!(status, StatusCode::OK);
    fixture
        .store
        .update_profile(
            "browser",
            &owner(),
            session.profile_id.as_ref().unwrap(),
            &ProfileUpdate { name: None, access: Some(silicon_browser_shared::AccessList::default()) },
        )
        .await
        .unwrap();
    let (status, _, _) = request(&fixture.app, "GET", &path, Some(("oat_viewer", "browser")), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "historical participants cannot renew removed access");
    assert!(
        fixture
            .store
            .session(
                "browser",
                &Identity {
                    id: "c:viewer-1".into(),
                    name: "Viewer".into(),
                    kind: IdentityKind::Carbon,
                    tags: vec![],
                    verified_aliases: vec![]
                },
                &session.id
            )
            .await
            .is_ok()
    );
    fixture
        .store
        .begin_end_session(
            "browser",
            &owner(),
            &session.id,
            &SessionEnd { note: "done".into() },
            &fixture.state.secrets,
        )
        .await
        .unwrap();
    let (status, _, _) = request(&fixture.app, "GET", &path, Some(("oat_owner", "browser")), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "ending maps to active publicly but cannot issue capabilities");
    let expired = seed_expired_incognito(&fixture, "connection").await;
    let (status, _, _) = request(
        &fixture.app,
        "GET",
        &format!("/api/v1/sessions/{}/connection", expired.id),
        Some(("oat_owner", "browser")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn retired_command_routes_do_not_collect_execute_or_return_history() {
    let fixture = fixture().await;
    let session = start(&fixture, false, false).await;
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("must-not-exist");
    let mut payload = report();
    payload["command"] = json!(format!("sh -c 'touch {}'", marker.display()));
    payload["stdout"] = json!("private browser output");
    for body in [payload, json!({}), Value::Null] {
        let (status, response) = send(&fixture, &session, body).await;
        assert_eq!(status, StatusCode::GONE);
        assert_eq!(response["error"]["code"], "command_history_removed");
    }
    for table in ["commands", "command_reports"] {
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}")))
            .fetch_one(fixture.store.pool()).await.unwrap();
        assert_eq!(count, 0);
    }
    for auth in [None, Some(("oat_owner", "browser"))] {
        let (status, _, body) = request(&fixture.app, "GET",
            &format!("/api/v1/sessions/{}/logs", session.id), auth, None).await;
        assert_eq!(status, StatusCode::GONE);
        assert!(!String::from_utf8_lossy(&body).contains("local-secret"));
    }
    assert!(!marker.exists());
    assert_eq!(fixture.browser.state.lock().unwrap().get_calls, 0);
    assert!(fixture.browser.stopped().is_empty());
}

#[tokio::test]
async fn historical_command_archives_are_not_exposed_with_recordings() {
    let fixture = fixture().await;
    let session = start(&fixture, false, false).await;
    sqlx::query("INSERT INTO recording_artifacts(session_id,kind,state,next_attempt_at,artifact_path,receipt_url_enc,last_error) VALUES(?,'commands','complete',?,'private/commands.jsonl','unused-old-ciphertext','old-command-error')")
        .bind(&session.id).bind(Utc::now().to_rfc3339()).execute(fixture.store.pool()).await.unwrap();
    let (status, _, body) = request(&fixture.app, "GET",
        &format!("/api/v1/recordings/{}", session.id), Some(("oat_owner", "browser")), None).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let recording = data(&body);
    assert!(recording.get("command_log_link").is_none());
    assert!(recording.get("command_log_path").is_none());
    assert!(recording.get("delivery_error").is_none());
    let state: String = sqlx::query_scalar("SELECT state FROM recording_artifacts WHERE session_id = ? AND kind = 'commands'")
        .bind(&session.id).fetch_one(fixture.store.pool()).await.unwrap();
    assert_eq!(state, "complete", "historical records remain untouched");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn five_hundred_concurrent_clients_read_connections_without_browser_work() {
    let directory = tempfile::tempdir().unwrap();
    let mut fixture = fixture().await;
    let store =
        Store::connect(&format!("sqlite://{}", directory.path().join("metadata-load.db").display())).await.unwrap();
    fixture.store = store.clone();
    fixture.state.store = store;
    fixture.app = router(fixture.state.clone());
    let mode: String = sqlx::query_scalar("PRAGMA journal_mode").fetch_one(fixture.store.pool()).await.unwrap();
    assert_eq!(mode, "wal");
    let session = start(&fixture, false, true).await;
    let barrier = Arc::new(tokio::sync::Barrier::new(500));
    let mut workers = tokio::task::JoinSet::new();
    for i in 0..500 {
        let token = format!("oat_load_{i}");
        let identity = principal("si:owner-1", IdentityKind::Silicon, true);
        fixture.identity.allow_identity(&token, identity);
        let (app, session, barrier) = (fixture.app.clone(), session.clone(), barrier.clone());
        workers.spawn(async move {
            barrier.wait().await;
            let started = tokio::time::Instant::now();
            let (status, _, body) = request(
                &app,
                "GET",
                &format!("/api/v1/sessions/{}/connection", session.id),
                Some((&token, "browser")),
                None,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "connection {}", String::from_utf8_lossy(&body));
            started.elapsed()
        });
    }
    let mut timings = Vec::new();
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(result) = workers.join_next().await {
            timings.push(result.unwrap());
        }
    })
    .await
    .expect("500-client metadata burst must not stall");
    timings.sort_unstable();
    assert_eq!(fixture.browser.state.lock().unwrap().get_calls, 0);
    assert_eq!(fixture.browser.state.lock().unwrap().browsers.len(), 1);
    eprintln!(
        "500 clients / 500 authenticated connection requests: p50={:?} p95={:?} max={:?}; fake Accounts/provider, local SQLite WAL",
        timings[249], timings[474], timings[499]
    );
}
