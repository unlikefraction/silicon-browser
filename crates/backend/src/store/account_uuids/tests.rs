use super::*;

const NEW: &str = "bc0f6684-df51-4d9f-b945-bbdf88e599ad";
const OTHER_NEW: &str = "6cfb43ce-3a3f-4f56-85eb-0990e8d93da4";
fn csv() -> Vec<u8> {
    format!("old_uuid,new_uuid,kind\nAb1,{NEW},silicon\nCd2,{OTHER_NEW},carbon\n").into_bytes()
}
fn actor(id: &str) -> Identity {
    Identity {
        id: id.into(),
        name: "si:worker".into(),
        kind: IdentityKind::Silicon,
        tags: vec![],
        verified_aliases: vec![],
    }
}
async fn fixture() -> (tempfile::TempDir, String, Store, SecretBox, String) {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("accounts.db").display());
    let store = Store::connect(&url).await.unwrap();
    let key = SecretBox::new(&[7; 32]);
    let request = SessionCreate::incognito("retained session", "UUID cutover", SessionTtl::Minutes15);
    let session = store.reserve_session("browser", &actor("Ab1"), &request, Utc::now()).await.unwrap();
    let provider = ProviderSession {
        id: "provider-session".into(),
        cdp_url: "wss://provider.example/cdp?private=1".into(),
        live_url: "https://provider.example/live?private=1".into(),
        recording_url: Some("https://provider.example/replay?private=1".into()),
    };
    store.activate_session("browser", &session.id, &provider, &key, Utc::now()).await.unwrap();
    sqlx::query("INSERT INTO profiles(id,org_id,name,provider_profile_id,fingerprint,location,access_json,owner_id,status,created_at) VALUES('profile','browser','Retained','provider-profile','fingerprint','US','[\"@Ab1\",\"@Cd2\"]','Ab1','active',?)").bind(timestamp(Utc::now())).execute(&store.pool).await.unwrap();
    let command = "open https://example.test/Ab1"; // Free text is never rewritten.
    let cipher = key.seal_for(&command_secret_context("browser", &session.id, 1, "Ab1"), command).unwrap();
    sqlx::query("INSERT INTO commands(session_id,sequence,actor_id,command_enc,started_at,ended_at,exit_code) VALUES(?,1,'Ab1',?,?,?,0)").bind(&session.id).bind(cipher).bind(timestamp(Utc::now())).bind(timestamp(Utc::now())).execute(&store.pool).await.unwrap();
    sqlx::query("INSERT INTO command_reports(session_id,command_id,actor_id,principal_id,payload_sha256,sequence,received_at) VALUES(?,'report','Ab1','Ab1','frozen-request-hash',1,?)").bind(&session.id).bind(timestamp(Utc::now())).execute(&store.pool).await.unwrap();
    sqlx::query("INSERT INTO accounts_login_attempts VALUES('login','state','preserved-pkce-ciphertext','https://browser.example/callback',9999999999,NULL)").execute(&store.pool).await.unwrap();
    // The consumer must also work before the new runtime applies migration 0015.
    sqlx::raw_sql("DROP TABLE account_uuid_rekeys; DROP TABLE account_uuid_cutover; DELETE FROM _sqlx_migrations WHERE version=15;").execute(&store.pool).await.unwrap();
    (dir, url, store, key, session.id)
}

#[tokio::test]
async fn dry_run_apply_replay_preserve_resources_commands_capabilities_and_history() {
    let (_dir, url, store, key, session) = fixture().await;
    let before: (String, String, String) =
        sqlx::query_as("SELECT provider_cdp_url_enc,provider_live_url_enc,provider_recording_url_enc FROM sessions")
            .fetch_one(&store.pool)
            .await
            .unwrap();
    let original_cipher: String =
        sqlx::query_scalar("SELECT command_enc FROM commands").fetch_one(&store.pool).await.unwrap();
    let dry = migrate(&url, &csv(), &key, false).await.unwrap();
    assert!(dry.dry_run && !dry.already_applied);
    assert_eq!(dry.changed["commands.command_enc"], 1);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT started_by FROM sessions").fetch_one(&store.pool).await.unwrap(),
        "Ab1"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE name='account_uuid_cutover'")
            .fetch_one(&store.pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT command_enc FROM commands").fetch_one(&store.pool).await.unwrap(),
        original_cipher
    );
    let applied = migrate(&url, &csv(), &key, true).await.unwrap();
    assert_eq!(applied.changed, dry.changed);
    assert_eq!(applied.map_sha256, dry.map_sha256);
    store.ensure_accounts_database().await.unwrap();
    assert!(store.account_subject_retired("Ab1").await.unwrap());
    assert!(store.account_subject_retired("unknown").await.unwrap());
    assert!(!store.account_subject_retired(NEW).await.unwrap());
    assert_eq!(store.session("browser", &actor(NEW), &session).await.unwrap().id, session);
    assert!(store.session("browser", &actor("Ab1"), &session).await.is_err());
    let logs = store.session_logs("browser", &actor(NEW), &session, None, &key).await.unwrap();
    assert_eq!(logs[0].actor_id, NEW);
    assert_eq!(logs[0].command, "open https://example.test/Ab1");
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT delivery_actor_id FROM commands").fetch_one(&store.pool).await.unwrap(),
        "Ab1"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT payload_sha256 FROM command_reports")
            .fetch_one(&store.pool)
            .await
            .unwrap(),
        "frozen-request-hash"
    );
    let after: (String, String, String) =
        sqlx::query_as("SELECT provider_cdp_url_enc,provider_live_url_enc,provider_recording_url_enc FROM sessions")
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(before, after);
    let access: String = sqlx::query_scalar("SELECT access_json FROM profiles").fetch_one(&store.pool).await.unwrap();
    assert_eq!(serde_json::from_str::<Vec<String>>(&access).unwrap(), [format!("@{NEW}"), format!("@{OTHER_NEW}")]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT expires_at FROM accounts_login_attempts")
            .fetch_one(&store.pool)
            .await
            .unwrap(),
        0
    );
    let cipher: String = sqlx::query_scalar("SELECT command_enc FROM commands").fetch_one(&store.pool).await.unwrap();
    assert_ne!(cipher, original_cipher);
    let replay = migrate(&url, &csv(), &key, true).await.unwrap();
    assert!(replay.already_applied && replay.changed.is_empty());
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT command_enc FROM commands").fetch_one(&store.pool).await.unwrap(),
        cipher
    );
    assert_eq!(sqlx::query_scalar::<_, String>("PRAGMA integrity_check").fetch_one(&store.pool).await.unwrap(), "ok");
    let changed_map = String::from_utf8(csv()).unwrap().replace(OTHER_NEW, "5b314034-334c-471e-a2e6-fba2b8824080");
    assert!(migrate(&url, changed_map.as_bytes(), &key, true).await.unwrap_err().to_string().contains("immutable"));
    sqlx::query("UPDATE command_reports SET principal_id='Ab1'").execute(&store.pool).await.unwrap();
    assert!(migrate(&url, &csv(), &key, true).await.unwrap_err().to_string().contains("reintroduced"));
}

#[tokio::test]
async fn missing_mapping_wrong_key_kind_and_target_collision_fail_before_updates() {
    let (_dir, url, store, key, _session) = fixture().await;
    let incomplete = format!("old_uuid,new_uuid,kind\nAb1,{NEW},silicon\n");
    assert!(migrate(&url, incomplete.as_bytes(), &key, true).await.unwrap_err().to_string().contains("unmapped"));
    assert!(matches!(migrate(&url, &csv(), &SecretBox::new(&[8; 32]), true).await.unwrap_err(), StoreError::Crypto(_)));
    let wrong_kind = String::from_utf8(csv()).unwrap().replace(",silicon", ",carbon");
    assert!(migrate(&url, wrong_kind.as_bytes(), &key, true).await.unwrap_err().to_string().contains("kind differs"));
    sqlx::query("UPDATE profiles SET access_json=?").bind(format!("[\"@{NEW}\"]")).execute(&store.pool).await.unwrap();
    assert!(migrate(&url, &csv(), &key, true).await.unwrap_err().to_string().contains("already present"));
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT started_by FROM sessions").fetch_one(&store.pool).await.unwrap(),
        "Ab1"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE name='account_uuid_cutover'")
            .fetch_one(&store.pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn legacy_iam_store_and_delivery_authority_are_never_adopted() {
    let (_dir, url, store, key, _session) = fixture().await;
    sqlx::query("UPDATE sessions SET org_id='legacy-org',started_by='worker:tos'").execute(&store.pool).await.unwrap();
    assert!(migrate(&url, &csv(), &key, true).await.unwrap_err().to_string().contains("IAM-owned"));
    sqlx::query("UPDATE sessions SET org_id='browser',started_by='Ab1',delivery_principal_id='legacy-principal'")
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(migrate(&url, &csv(), &key, true).await.unwrap_err().to_string().contains("legacy IAM delivery"));
}

#[test]
fn mapping_rejects_ambiguous_or_noncanonical_input() {
    for raw in [
        format!("old_uuid,new_uuid,kind\nAb1,{NEW},silicon\nAb1,{OTHER_NEW},silicon\n"),
        format!("old_uuid,new_uuid,kind\nAb1,{NEW},silicon\nCd2,{NEW},carbon\n"),
        format!("old_uuid,new_uuid,kind\nAb1,{},silicon\n", NEW.to_uppercase()),
        "old_uuid,new_uuid,kind\nAb1,bc0f6684-df51-7d9f-b945-bbdf88e599ad,silicon\n".into(),
        "old_uuid,new_uuid,kind,extra\n".into(),
    ] {
        assert!(parse(raw.as_bytes()).is_err());
    }
}
