fn digest() -> String {
    "a".repeat(64)
}

#[tokio::test]
async fn obo_migration_retires_old_authority_without_replacing_receipts_or_job_ownership() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
    for migration in [
        include_str!("../../migrations/0001_initial.sql"),
        include_str!("../../migrations/0002_delivery_credentials.sql"),
        include_str!("../../migrations/0003_recording_delivery.sql"),
        include_str!("../../migrations/0004_delivery_principal_ownership.sql"),
        include_str!("../../migrations/0005_client_command_reports.sql"),
        include_str!("../../migrations/0006_iam_webhook_receipts.sql"),
        include_str!("../../migrations/0007_testing_environments.sql"),
        include_str!("../../migrations/0008_canonical_delivery_replay.sql"),
        include_str!("../../migrations/0009_public_identifiers.sql"),
    ] {
        sqlx::raw_sql(migration).execute(&pool).await.unwrap();
    }
    for (id, state, operation) in [
        ("ready", "active", None),
        ("pending", "pending", Some("exchange")),
        ("refreshing", "refreshing", Some("refresh")),
    ] {
        sqlx::query("INSERT INTO delivery_credentials(id,org_id,actor_id,principal_id,membership_id,actor_kind,enabled,state,operation,enrollment_digest,encrypted_payload,lease_owner,lease_until,created_at,updated_at) VALUES(?,'org',?,?,?,'silicon',1,?,?,?,'encrypted-old-family','worker',9999999999,1,1)")
            .bind(id).bind(id).bind(id).bind(id).bind(state).bind(operation).bind(id).execute(&pool).await.unwrap();
    }
    sqlx::raw_sql("INSERT INTO sessions(id,org_id,started_by,started_by_kind,name,description,started_at,expires_at,delivery_principal_id,delivery_membership_id) VALUES('session','org','si:owner','silicon','Existing recording','Migration fixture','2026-01-01','2026-01-02','principal','membership'); INSERT INTO recording_artifacts(session_id,kind,state,next_attempt_at,entry_id,artifact_path,receipt_url_enc) VALUES('session','video','complete','2026-01-02','verified-entry','existing/path','encrypted-receipt');")
        .execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../../migrations/0010_recording_obo_consent.sql")).execute(&pool).await.unwrap();
    let retired: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM delivery_credentials WHERE enabled=0 AND state='revoking' AND operation='revoke' AND mutation_key=id AND lease_owner IS NULL AND lease_until=0 AND encrypted_payload='encrypted-old-family'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(retired, 3);
    let grants: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recording_obo_grants").fetch_one(&pool).await.unwrap();
    assert_eq!(grants, 0);
    let receipt: (String, String, String, String, String) = sqlx::query_as("SELECT s.delivery_principal_id,s.delivery_membership_id,a.state,a.entry_id,a.receipt_url_enc FROM sessions s JOIN recording_artifacts a ON a.session_id=s.id")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(
        receipt,
        (
            "principal".into(),
            "membership".into(),
            "complete".into(),
            "verified-entry".into(),
            "encrypted-receipt".into()
        )
    );
}

#[tokio::test]
async fn recovery_migration_holds_unknown_legacy_writes_and_preserves_selected_destinations() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
    let migrations = sqlx::migrate!("./migrations");
    for migration in migrations.iter().filter(|m| m.version < 12) {
        sqlx::raw_sql(migration.sql.clone()).execute(&pool).await.unwrap();
    }
    for (id, state, bound) in [
        ("pending", "pending", true),
        ("working", "working", true),
        ("uploading", "uploading", true),
        ("failed", "failed", true),
        ("complete", "complete", true),
        ("unstarted", "pending", false),
        ("hidden", "working", true),
        ("selected", "working", true),
    ] {
        sqlx::query("INSERT INTO sessions(id,org_id,started_by,started_by_kind,name,description,status,started_at,expires_at) VALUES(?,'org','si:owner','silicon','Test','Migration fixture','ended','2026-10-01','2026-10-02')")
            .bind(id).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO recordings(session_id,owner_id,status,artifact_path) VALUES(?,'si:owner',?,'')")
            .bind(id)
            .bind(if id == "hidden" { "trashed" } else { "pending" })
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO recording_artifacts(session_id,kind,state,next_attempt_at,body_sha256,last_error,storage_org_id,storage_actor_id) VALUES(?,'video',?,'2026-10-02',?,'briefcase_upload_unconfirmed',?,?)")
            .bind(id).bind(state).bind(bound.then(digest))
            .bind((id=="selected").then_some("selected-org"))
            .bind((id=="selected").then_some("si:selected")).execute(&pool).await.unwrap();
    }
    sqlx::raw_sql(include_str!("../../migrations/0012_recording_upload_recovery.sql")).execute(&pool).await.unwrap();
    let held: Vec<String> = sqlx::query_scalar("SELECT session_id FROM recording_artifacts WHERE state='failed' AND last_error='legacy_upload_reconciliation_required' ORDER BY session_id")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(held, ["failed", "hidden", "pending", "uploading", "working"]);
    let hidden: String =
        sqlx::query_scalar("SELECT status FROM recordings WHERE session_id='hidden'").fetch_one(&pool).await.unwrap();
    assert_eq!(hidden, "trashed");
    let untouched: Vec<(String,String)> = sqlx::query_as("SELECT session_id,state FROM recording_artifacts WHERE session_id IN ('complete','selected','unstarted') ORDER BY session_id")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(
        untouched,
        [
            ("complete".into(), "complete".into()),
            ("selected".into(), "working".into()),
            ("unstarted".into(), "pending".into())
        ]
    );
    let selected: (String, String) =
        sqlx::query_as("SELECT delivery_destination_org,delivery_destination_actor FROM sessions WHERE id='selected'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(selected, ("selected-org".into(), "si:selected".into()));
    let guessed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sessions WHERE id <> 'selected' AND delivery_destination_org IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(guessed, 0);
}
