// Local-controller contract, archive fencing, and synthetic metadata capacity.
// Every provider is fake: this suite starts no browser and makes no network call.
use super::*;

async fn start(fixture: &Fixture, carbon: bool, profile: bool) -> Session {
    let token = if carbon { "oat_viewer" } else { "oat_owner" };
    let profile_id = if profile {
        let (status, _, body) = request(
            &fixture.app,
            "POST",
            "/api/v1/profiles",
            Some((token, "org-1")),
            Some(json!({"name":"Shared","location":"in","access":["@viewer-1", "workers"]})),
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
        request(&fixture.app, "POST", "/api/v1/sessions", Some((token, "org-1")), Some(payload)).await;
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
        Some(("oat_owner", "org-1")),
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
    let (status, headers, body) = request(&fixture.app, "GET", &path, Some(("oat_owner", "org-1")), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["cache-control"], "no-store");
    let connection: SessionConnection = serde_json::from_value(data(&body)).unwrap();
    assert_eq!(connection.session_id, session.id);
    assert_eq!(connection.expires_at, session.expires_at);
    assert_eq!(
        connection.principal_id,
        fixture.identity.identify("oat_owner", "org-1").await.unwrap().principal_id.to_string()
    );
    assert!(connection.cdp_url.starts_with("wss://provider.invalid/cdp/"));
    assert!(!format!("{connection:?}").contains("secret=yes"));
    assert_eq!(fixture.browser.state.lock().unwrap().get_calls, 0);
    assert!(fixture.browser.stopped().is_empty());
    for auth in [None, Some(("oat_viewer", "org-1")), Some(("oat_owner", "wrong-org"))] {
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
            "org-1",
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
    fixture.store.activate_session("org-1", &session.id, &runtime, &fixture.state.secrets, now).await.unwrap();
    let (status, headers, body) = request(
        &fixture.app,
        "GET",
        &format!("/api/v1/sessions/{}/connection", session.id),
        Some(("oat_owner", "org-1")),
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
    let commands = format!("/api/v1/sessions/{}/commands", session.id);
    let (status, _, _) = request(&fixture.app, "POST", &commands, Some(("oat_viewer", "org-1")), Some(report())).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = request(&fixture.app, "GET", &path, Some(("oat_viewer", "org-1")), None).await;
    assert_eq!(status, StatusCode::OK);
    fixture
        .store
        .update_profile(
            "org-1",
            &owner(),
            session.profile_id.as_ref().unwrap(),
            &ProfileUpdate { name: None, access: Some(silicon_browser_shared::AccessList::default()) },
        )
        .await
        .unwrap();
    let (status, _, _) = request(&fixture.app, "GET", &path, Some(("oat_viewer", "org-1")), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "historical participants cannot renew removed access");
    assert!(
        fixture
            .store
            .session(
                "org-1",
                &Identity {
                    id: "viewer-1".into(),
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
        .begin_end_session("org-1", &owner(), &session.id, &SessionEnd { note: "done".into() }, &fixture.state.secrets)
        .await
        .unwrap();
    let (status, _, _) = request(&fixture.app, "GET", &path, Some(("oat_owner", "org-1")), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "ending maps to active publicly but cannot issue capabilities");
    let expired = seed_expired_incognito(&fixture, "connection").await;
    let (status, _, _) = request(
        &fixture.app,
        "GET",
        &format!("/api/v1/sessions/{}/connection", expired.id),
        Some(("oat_owner", "org-1")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn report_endpoint_never_executes_legacy_run_payloads_or_reported_shell_text() {
    let fixture = fixture().await;
    let session = start(&fixture, false, false).await;
    let (status, _) = send(&fixture, &session, json!({"command":"open https://example.com", "flags":[]})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Browser output must never cross this boundary, even if an older or
    // incorrect client attempts to include it in an otherwise valid report.
    for field in ["stdout", "stderr"] {
        let mut with_output = report();
        with_output[field] = json!("private browser output");
        assert_eq!(send(&fixture, &session, with_output).await.0, StatusCode::BAD_REQUEST);
    }
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("must-not-exist");
    let mut payload = report();
    payload["command"] = json!(format!("sh -c 'touch {}'", marker.display()));
    let (status, _) = send(&fixture, &session, payload).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!marker.exists());
    assert_eq!(fixture.browser.state.lock().unwrap().get_calls, 0);
    assert!(fixture.browser.stopped().is_empty());
}

#[tokio::test]
async fn reports_retry_idempotently_and_reject_changed_body_or_different_principal() {
    let fixture = fixture().await;
    let session = start(&fixture, false, true).await;
    let payload = report();
    let (first_status, first) = send(&fixture, &session, payload.clone()).await;
    let (again_status, again) = send(&fixture, &session, payload.clone()).await;
    assert_eq!((first_status, again_status), (StatusCode::OK, StatusCode::OK));
    assert_eq!(first["data"], again["data"]);
    let mut changed = payload.clone();
    changed["exit_code"] = json!(2);
    let (status, response) = send(&fixture, &session, changed).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(response["error"]["code"], "command_report_conflict");
    let (status, _, _) = request(
        &fixture.app,
        "POST",
        &format!("/api/v1/sessions/{}/commands", session.id),
        Some(("oat_viewer", "org-1")),
        Some(payload),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let logs = fixture.store.session_logs("org-1", &owner(), &session.id, None, &fixture.state.secrets).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert!(logs[0].command.contains("local-secret"));
    let encrypted: String = sqlx::query_scalar("SELECT command_enc FROM commands WHERE session_id = ?")
        .bind(&session.id)
        .fetch_one(fixture.store.pool())
        .await
        .unwrap();
    assert!(!encrypted.contains("local-secret"));
}

#[tokio::test]
async fn first_archive_claim_freezes_new_reports_before_hashing_but_acknowledges_replays() {
    let fixture = fixture().await;
    let session = start(&fixture, false, false).await;
    let original = report();
    assert_eq!(send(&fixture, &session, original.clone()).await.0, StatusCode::OK);
    fixture
        .store
        .begin_end_session("org-1", &owner(), &session.id, &SessionEnd { note: "done".into() }, &fixture.state.secrets)
        .await
        .unwrap();
    fixture.store.finalize_end_session("org-1", &session.id, Utc::now()).await.unwrap();
    // A report arriving after the stop is still admitted before archival starts.
    assert_eq!(send(&fixture, &session, report()).await.0, StatusCode::OK);
    let now = Utc::now();
    let claims = fixture.store.claim_recording_deliveries(now, now + TimeDelta::seconds(60), 2).await.unwrap();
    let logs = claims.iter().find(|claim| claim.kind == crate::store::RecordingArtifactKind::Commands).unwrap();
    let digest: Option<String> =
        sqlx::query_scalar("SELECT body_sha256 FROM recording_artifacts WHERE session_id = ? AND kind = 'commands'")
            .bind(&session.id)
            .fetch_one(fixture.store.pool())
            .await
            .unwrap();
    assert!(digest.is_none(), "snapshot closes before reading or binding a digest");
    let before = fixture.store.recording_delivery_logs(logs, 0, 128, &fixture.state.secrets).await.unwrap();
    let (status, body) = send(&fixture, &session, report()).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "report_window_closed");
    assert_eq!(send(&fixture, &session, original).await.0, StatusCode::OK);
    let after = fixture.store.recording_delivery_logs(logs, 0, 128, &fixture.state.secrets).await.unwrap();
    assert_eq!(serde_json::to_vec(&before).unwrap(), serde_json::to_vec(&after).unwrap());
    assert_eq!(after.len(), 2);
}

#[tokio::test]
async fn report_timestamps_cannot_invent_work_outside_the_session_window() {
    let fixture = fixture().await;
    let session = start(&fixture, false, false).await;
    for time in [session.started_at - TimeDelta::minutes(6), session.expires_at + TimeDelta::minutes(6)] {
        let mut payload = report();
        payload["started_at"] = json!(time);
        payload["finished_at"] = json!(time);
        assert_eq!(send(&fixture, &session, payload).await.0, StatusCode::BAD_REQUEST);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn five_hundred_concurrent_clients_read_connections_and_report_without_browser_work() {
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
        let mut identity = principal(&format!("worker-{i}"), IdentityKind::Silicon, true);
        identity.tags = Some(vec!["workers".into()]);
        fixture.identity.allow_identity(&token, identity);
        let (app, session, barrier) = (fixture.app.clone(), session.clone(), barrier.clone());
        workers.spawn(async move {
            barrier.wait().await;
            let started = tokio::time::Instant::now();
            let (status, _, body) = request(
                &app,
                "GET",
                &format!("/api/v1/sessions/{}/connection", session.id),
                Some((&token, "org-1")),
                None,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "connection {}", String::from_utf8_lossy(&body));
            let (status, _, body) = request(
                &app,
                "POST",
                &format!("/api/v1/sessions/{}/commands", session.id),
                Some((&token, "org-1")),
                Some(report()),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "report {}", String::from_utf8_lossy(&body));
            (data(&body)["sequence"].as_u64().unwrap(), started.elapsed())
        });
    }
    let mut sequences = Vec::new();
    let mut timings = Vec::new();
    tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(result) = workers.join_next().await {
            let (sequence, elapsed) = result.unwrap();
            sequences.push(sequence);
            timings.push(elapsed);
        }
    })
    .await
    .expect("500-client metadata burst must not stall");
    sequences.sort_unstable();
    timings.sort_unstable();
    assert_eq!(sequences, (1..=500).collect::<Vec<_>>());
    assert_eq!(fixture.browser.state.lock().unwrap().get_calls, 0);
    assert_eq!(fixture.browser.state.lock().unwrap().browsers.len(), 1);
    eprintln!(
        "500 clients / 1000 authenticated metadata requests: p50={:?} p95={:?} max={:?}; fake IAM/provider, local SQLite WAL",
        timings[249], timings[474], timings[499]
    );
}
