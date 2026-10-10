use super::*;
#[tokio::test]
async fn legacy_authority_tables_are_rejected_before_any_schema_mutation() {
    let dir = tempfile::tempdir().unwrap();
    for (index, schema) in [
        "CREATE TABLE identity_projection (principal_id TEXT); INSERT INTO identity_projection VALUES ('legacy-owner');",
        "CREATE TABLE login_attempts (id TEXT); INSERT INTO login_attempts VALUES ('old-login');",
        "CREATE TABLE recording_obo_authorizations (id TEXT); INSERT INTO recording_obo_authorizations VALUES ('old-authorization');",
        "CREATE TABLE recording_obo_grants (id TEXT); INSERT INTO recording_obo_grants VALUES ('old-grant');",
        "CREATE TABLE delivery_obo (id TEXT); INSERT INTO delivery_obo VALUES ('old-grant');",
        "CREATE TABLE testing_environments (id TEXT); INSERT INTO testing_environments VALUES ('old-world');",
        "CREATE TABLE public_identifier_schema (id TEXT); INSERT INTO public_identifier_schema VALUES ('old-schema');",
    ].into_iter().enumerate() {
        let path = dir.path().join(format!("legacy-{index}.db"));
        let database_url = format!("sqlite://{}?mode=rwc", path.display());
        let pool = SqlitePoolOptions::new().max_connections(1).connect(&database_url).await.unwrap();
        sqlx::raw_sql(schema).execute(&pool).await.unwrap();
        pool.close().await;
        let original = std::fs::read(&path).unwrap();
        assert!(Store::preflight_accounts_database(&database_url).await.unwrap_err().to_string().contains("IAM-owned"));
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}
#[tokio::test]
async fn native_recording_links_require_visibility_and_safe_provider_urls() {
    let store = Store::in_memory().await.unwrap();
    let secrets = SecretBox::new(&[7; 32]);
    let owner = Identity {
        id: "8HV".into(),
        name: "c:ada".into(),
        kind: IdentityKind::Carbon,
        tags: vec![],
        verified_aliases: vec![],
    };
    let other = Identity {
        id: "K1E".into(),
        name: "si:worker".into(),
        kind: IdentityKind::Silicon,
        tags: vec![],
        verified_aliases: vec![],
    };
    let request = SessionCreate::incognito(
        "Accounts recording",
        "native provider replay",
        silicon_browser_shared::SessionTtl::Minutes15,
    );
    let reserved = store.reserve_session("browser", &owner, &request, Utc::now()).await.unwrap();
    let provider = ProviderSession {
        id: "remote-browser".into(),
        cdp_url: "wss://provider.example/cdp".into(),
        live_url: "https://provider.example/live".into(),
        recording_url: Some("https://provider.example/video?capability=private".into()),
    };
    store.activate_session("browser", &reserved.id, &provider, &secrets, Utc::now()).await.unwrap();
    store
        .begin_end_session("browser", &owner, &reserved.id, &SessionEnd { note: "done".into() }, &secrets)
        .await
        .unwrap();
    store.mark_recording_pending("browser", &reserved.id, 1000, 0).await.unwrap();
    store.finalize_end_session("browser", &reserved.id, Utc::now()).await.unwrap();
    let recording = store.recording("browser", &owner, &reserved.id, &secrets).await.unwrap();
    assert_eq!(recording.status, RecordingStatus::Available);
    assert_eq!(recording.briefcase_link.as_deref(), Some("https://provider.example/video?capability=private"));
    assert!(recording.briefcase_path.is_empty());
    assert!(store.recording("browser", &other, &reserved.id, &secrets).await.is_err());
    for unsafe_url in [
        "http://provider.example/video",
        "https://127.0.0.1/video",
        "https://user:secret@provider.example/video",
        "javascript:alert(1)",
    ] {
        store.remember_provider_recording_url("browser", &reserved.id, unsafe_url, &secrets).await.unwrap();
        let recording = store.recording("browser", &owner, &reserved.id, &secrets).await.unwrap();
        assert!(recording.briefcase_link.is_none());
        assert_eq!(recording.status, RecordingStatus::Pending);
    }
    store.ensure_accounts_database().await.unwrap();
}
