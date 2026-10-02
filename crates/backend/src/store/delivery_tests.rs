//! Durable delivery state-machine regressions; no network or provider credentials.
use super::*;
use crate::providers::BriefcaseEntry;

const PRINCIPAL: &str = "00000000-0000-0000-0000-000000000010";
const MEMBERSHIP: &str = "00000000-0000-0000-0000-000000000020";

fn secrets() -> SecretBox {
    SecretBox::new(&[17; 32])
}
fn digest() -> String {
    "a".repeat(64)
}

async fn fixture(kind: IdentityKind, bind: bool, unfinished: bool) -> (Store, Identity, String, Option<u64>) {
    let store = Store::in_memory().await.unwrap();
    let owner = Identity { id: "owner".into(), name: "Owner".into(), kind, tags: vec![], verified_aliases: vec![] };
    let now = Utc::now();
    let session = store
        .reserve_session(
            "org",
            &owner,
            &SessionCreate::incognito("Delivery", "Fault fixture", SessionTtl::Minutes15),
            now,
        )
        .await
        .unwrap();
    if bind {
        store.bind_session_delivery_owner("org", &session.id, PRINCIPAL, MEMBERSHIP).await.unwrap();
    }
    store
        .activate_session(
            "org",
            &session.id,
            &ProviderSession {
                id: "remote".into(),
                cdp_url: "wss://provider.example/cdp".into(),
                live_url: "https://provider.example/live".into(),
                recording_url: Some("https://provider.example/video".into()),
            },
            &secrets(),
            now,
        )
        .await
        .unwrap();
    let ticket = if unfinished {
        // Legacy database fixture: the old backend could crash before an exit.
        let encrypted = secrets()
            .seal_for(&command_secret_context("org", &session.id, 1, &owner.id), "evaluate 'a  b' --flag \"quoted\"")
            .unwrap();
        sqlx::query(
            "INSERT INTO commands(session_id, sequence, actor_id, command_enc, started_at) VALUES(?, 1, ?, ?, ?)",
        )
        .bind(&session.id)
        .bind(&owner.id)
        .bind(encrypted)
        .bind(timestamp(now))
        .execute(&store.pool)
        .await
        .unwrap();
        Some(1)
    } else {
        None
    };
    store
        .begin_end_session("org", &owner, &session.id, &SessionEnd { note: "finished".into() }, &secrets())
        .await
        .unwrap();
    store.mark_recording_pending("org", &session.id, 1_000, 0).await.unwrap();
    store.finalize_end_session("org", &session.id, now).await.unwrap();
    (store, owner, session.id, ticket)
}

fn receipt(claim: &RecordingDeliveryClaim) -> BriefcaseEntry {
    let name = match claim.kind {
        RecordingArtifactKind::Video => format!("{}.mp4", claim.session_id),
        RecordingArtifactKind::Commands => format!("{}-commands.jsonl", claim.session_id),
    };
    BriefcaseEntry {
        id: Uuid::now_v7(),
        org_id: claim.org_id.clone(),
        entry_type: "file".into(),
        path: format!("private/owner/browser/{name}"),
        name,
        content_type: None,
        size: 20,
        permanent_url: "https://briefcase.example/entry".into(),
        origin_app_id: Some("browser".into()),
    }
}

async fn begin(store: &Store, claim: &RecordingDeliveryClaim, now: DateTime<Utc>) {
    assert!(store.bind_recording_delivery(claim, &digest(), 20, now).await.unwrap());
    assert!(store.begin_recording_upload(claim, now).await.unwrap());
}

#[tokio::test]
async fn delivery_claims_are_exclusive_and_reclaimed_leases_reject_stale_receipts() {
    let (store, _, _, _) = fixture(IdentityKind::Carbon, true, false).await;
    let now = Utc::now();
    let until = now + ChronoDuration::seconds(60);
    let (first, second) =
        tokio::join!(store.claim_recording_deliveries(now, until, 2), store.claim_recording_deliveries(now, until, 2));
    let claims = first.unwrap().into_iter().chain(second.unwrap()).collect::<Vec<_>>();
    assert_eq!(claims.len(), 1);
    let old = &claims[0];
    begin(&store, old, now).await;
    let later = until + ChronoDuration::seconds(1);
    let newer =
        store.claim_recording_deliveries(later, later + ChronoDuration::seconds(60), 2).await.unwrap().pop().unwrap();
    assert_ne!(old.lease_id, newer.lease_id);
    assert_eq!(newer.attempt, 2);
    assert!(!store.recording_delivery_is_current(old, later).await.unwrap());
    assert!(!store.complete_recording_delivery(old, &receipt(old), &secrets(), later).await.unwrap());
    assert!(!store.defer_recording_delivery(old, later, "stale", true).await.unwrap());
    begin(&store, &newer, later).await;
    assert!(store.complete_recording_delivery(&newer, &receipt(&newer), &secrets(), later).await.unwrap());
}

#[tokio::test]
async fn silicon_video_and_command_receipts_complete_independently() {
    let (store, owner, session, _) = fixture(IdentityKind::Silicon, true, false).await;
    let now = Utc::now();
    let claims = store.claim_recording_deliveries(now, now + ChronoDuration::seconds(60), 2).await.unwrap();
    assert_eq!(claims.len(), 2);
    let video = claims.iter().find(|c| c.kind == RecordingArtifactKind::Video).unwrap();
    let logs = claims.iter().find(|c| c.kind == RecordingArtifactKind::Commands).unwrap();
    begin(&store, video, now).await;
    assert!(store.complete_recording_delivery(video, &receipt(video), &secrets(), now).await.unwrap());
    assert_eq!(store.recording("org", &owner, &session, &secrets()).await.unwrap().status, RecordingStatus::Pending);
    begin(&store, logs, now).await;
    assert!(store.complete_recording_delivery(logs, &receipt(logs), &secrets(), now).await.unwrap());
    let recording = store.recording("org", &owner, &session, &secrets()).await.unwrap();
    assert_eq!(recording.status, RecordingStatus::Available);
    assert_eq!(recording.size_bytes, 20);
    assert!(recording.briefcase_link.is_some());
    assert!(store.claim_recording_deliveries(now, now + ChronoDuration::seconds(60), 2).await.unwrap().is_empty());
    let done: Option<String> = sqlx::query_scalar("SELECT done_at FROM outbox WHERE id = ?")
        .bind(format!("recording.store:{session}"))
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert!(done.is_some());
}

#[tokio::test]
async fn carbon_has_only_video_and_owner_bindings_are_persisted_without_historical_adoption() {
    for bound in [true, false] {
        let (store, owner, session, _) = fixture(IdentityKind::Carbon, bound, false).await;
        let now = Utc::now();
        let claims = store.claim_recording_deliveries(now, now + ChronoDuration::seconds(60), 2).await.unwrap();
        assert_eq!(claims.len(), 1);
        let claim = &claims[0];
        assert_eq!(claim.kind, RecordingArtifactKind::Video);
        assert_eq!(claim.principal_id.as_deref(), bound.then_some(PRINCIPAL));
        assert_eq!(claim.membership_id.as_deref(), bound.then_some(MEMBERSHIP));
        assert!(store.bind_session_delivery_owner("org", &session, "replacement", "replacement").await.is_err());
        if bound {
            begin(&store, claim, now).await;
            assert!(store.complete_recording_delivery(claim, &receipt(claim), &secrets(), now).await.unwrap());
            assert_eq!(
                store.recording("org", &owner, &session, &secrets()).await.unwrap().status,
                RecordingStatus::Available
            );
        }
    }
}

#[tokio::test]
async fn trash_fences_new_uploads_but_preserves_late_success_without_resurrection() {
    let (store, owner, session, _) = fixture(IdentityKind::Silicon, true, false).await;
    let now = Utc::now();
    let claims = store.claim_recording_deliveries(now, now + ChronoDuration::seconds(60), 2).await.unwrap();
    let video = claims.iter().find(|c| c.kind == RecordingArtifactKind::Video).unwrap();
    let logs = claims.iter().find(|c| c.kind == RecordingArtifactKind::Commands).unwrap();
    begin(&store, video, now).await;
    store.trash_recording("org", &owner, &session, now, &secrets()).await.unwrap();
    assert!(!store.recording_delivery_is_current(video, now).await.unwrap());
    assert!(!store.bind_recording_delivery(logs, &digest(), 20, now).await.unwrap());
    assert!(!store.begin_recording_upload(logs, now).await.unwrap());
    let receipt = receipt(video);
    assert!(store.complete_recording_delivery(video, &receipt, &secrets(), now).await.unwrap());
    let recording = store.recording("org", &owner, &session, &secrets()).await.unwrap();
    assert_eq!(recording.status, RecordingStatus::Trashed);
    assert!(recording.briefcase_link.is_none());
    let persisted: String =
        sqlx::query_scalar("SELECT entry_id FROM recording_artifacts WHERE session_id = ? AND kind = 'video'")
            .bind(&session)
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(persisted, receipt.id.to_string());
    assert!(
        store
            .claim_recording_deliveries(now + ChronoDuration::minutes(5), now + ChronoDuration::minutes(6), 2)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn retry_digest_changes_and_wrong_receipt_names_cannot_replace_completed_video() {
    let (store, owner, session, _) = fixture(IdentityKind::Silicon, true, false).await;
    let now = Utc::now();
    let claims = store.claim_recording_deliveries(now, now + ChronoDuration::seconds(60), 2).await.unwrap();
    let video = claims.iter().find(|c| c.kind == RecordingArtifactKind::Video).unwrap();
    let logs = claims.iter().find(|c| c.kind == RecordingArtifactKind::Commands).unwrap();
    begin(&store, video, now).await;
    let mut invalid = receipt(video);
    invalid.name = "another-session.mp4".into();
    assert!(store.complete_recording_delivery(video, &invalid, &secrets(), now).await.is_err());
    let confirmed = receipt(video);
    assert!(store.complete_recording_delivery(video, &confirmed, &secrets(), now).await.unwrap());
    assert!(store.bind_recording_delivery(logs, &digest(), 20, now).await.unwrap());
    assert!(store.defer_recording_delivery(logs, now, "uncertain", true).await.unwrap());
    let retried =
        store.claim_recording_deliveries(now, now + ChronoDuration::seconds(60), 2).await.unwrap().pop().unwrap();
    assert_eq!(retried.kind, RecordingArtifactKind::Commands);
    assert!(!store.bind_recording_delivery(&retried, &"b".repeat(64), 20, now).await.unwrap());
    assert!(!store.bind_recording_delivery(&retried, &digest(), 21, now).await.unwrap());
    assert!(store.fail_recording_delivery(&retried, "source_changed", now).await.unwrap());
    let persisted: String =
        sqlx::query_scalar("SELECT entry_id FROM recording_artifacts WHERE session_id = ? AND kind = 'video'")
            .bind(&session)
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(persisted, confirmed.id.to_string());
    assert_eq!(store.recording("org", &owner, &session, &secrets()).await.unwrap().status, RecordingStatus::Failed);
}

#[tokio::test]
async fn claiming_terminal_logs_freezes_unfinished_commands_before_hashing() {
    let (store, _, _, ticket) = fixture(IdentityKind::Silicon, true, true).await;
    let now = Utc::now();
    let claims = store.claim_recording_deliveries(now, now + ChronoDuration::seconds(60), 2).await.unwrap();
    let logs = claims.iter().find(|c| c.kind == RecordingArtifactKind::Commands).unwrap();
    let before = store.recording_delivery_logs(logs, 0, 128, &secrets()).await.unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].exit_code, Some(130));
    assert_eq!(before[0].command, "evaluate 'a  b' --flag \"quoted\"");
    let changed =
        sqlx::query("UPDATE commands SET ended_at = ?, exit_code = 0 WHERE sequence = ? AND ended_at IS NULL")
            .bind(timestamp(now))
            .bind(ticket.unwrap() as i64)
            .execute(&store.pool)
            .await
            .unwrap()
            .rows_affected();
    assert_eq!(changed, 0);
    let after = store.recording_delivery_logs(logs, 0, 128, &secrets()).await.unwrap();
    assert_eq!(serde_json::to_vec(&before).unwrap(), serde_json::to_vec(&after).unwrap());
}

#[tokio::test]
async fn failed_command_delivery_does_not_starve_delayed_native_video() {
    let (store, _, session, _) = fixture(IdentityKind::Silicon, true, false).await;
    // Recreate the terminal state where Browser Use has not yet published its URL.
    sqlx::query("UPDATE sessions SET provider_recording_url_enc = NULL WHERE id = ?")
        .bind(&session)
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM outbox WHERE id = ?")
        .bind(format!("recording.store:{session}"))
        .execute(&store.pool)
        .await
        .unwrap();
    let now = Utc::now();
    store.queue_recording_source_resolution("org", &session, 1_000, now).await.unwrap();
    let claims = store.claim_recording_deliveries(now, now + ChronoDuration::seconds(60), 2).await.unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].kind, RecordingArtifactKind::Commands);
    assert!(store.fail_recording_delivery(&claims[0], "recording_size_limit", now).await.unwrap());
    let later = now + ChronoDuration::seconds(30);
    let source = store.claim_recording_source_resolutions(later, later + ChronoDuration::seconds(60), 2).await.unwrap();
    assert_eq!(source.len(), 1, "a failed log must not abandon the native video");
    assert!(
        store
            .complete_recording_source_resolution(&source[0], "https://provider.example/ready.mp4", &secrets(), later)
            .await
            .unwrap()
    );
    let delivery = store.claim_recording_deliveries(later, later + ChronoDuration::seconds(60), 2).await.unwrap();
    assert_eq!(delivery.len(), 1);
    assert_eq!(delivery[0].kind, RecordingArtifactKind::Video);
    begin(&store, &delivery[0], later).await;
    assert!(store.complete_recording_delivery(&delivery[0], &receipt(&delivery[0]), &secrets(), later).await.unwrap());
}

#[tokio::test]
async fn recording_paths_are_empty_until_a_real_receipt_even_for_legacy_rows() {
    let (store, owner, session, _) = fixture(IdentityKind::Carbon, true, false).await;
    assert!(store.recording("org", &owner, &session, &secrets()).await.unwrap().briefcase_path.is_empty());
    sqlx::query("UPDATE recordings SET artifact_path = ? WHERE session_id = ?")
        .bind(format!("private/owner/sb/{session}/recording.mp4"))
        .bind(&session)
        .execute(&store.pool)
        .await
        .unwrap();
    assert!(store.recording("org", &owner, &session, &secrets()).await.unwrap().briefcase_path.is_empty());
    let now = Utc::now();
    let claim =
        store.claim_recording_deliveries(now, now + ChronoDuration::seconds(60), 2).await.unwrap().pop().unwrap();
    begin(&store, &claim, now).await;
    let receipt = receipt(&claim);
    assert!(store.complete_recording_delivery(&claim, &receipt, &secrets(), now).await.unwrap());
    assert_eq!(store.recording("org", &owner, &session, &secrets()).await.unwrap().briefcase_path, receipt.path);
}

#[tokio::test]
async fn owner_retry_preserves_completed_receipts_and_bound_bytes_and_is_idempotent() {
    let (store, owner, session, _) = fixture(IdentityKind::Silicon, true, false).await;
    let now = Utc::now();
    let claims = store.claim_recording_deliveries(now, now + ChronoDuration::seconds(60), 2).await.unwrap();
    let video = claims.iter().find(|c| c.kind == RecordingArtifactKind::Video).unwrap();
    let logs = claims.iter().find(|c| c.kind == RecordingArtifactKind::Commands).unwrap();
    begin(&store, video, now).await;
    store.complete_recording_delivery(video, &receipt(video), &secrets(), now).await.unwrap();
    begin(&store, logs, now).await;
    store.fail_recording_delivery(logs, "briefcase_upload_unconfirmed", now).await.unwrap();
    for (org, actor, principal, member) in [
        ("other", "owner", PRINCIPAL, MEMBERSHIP),
        ("org", "other", PRINCIPAL, MEMBERSHIP),
        ("org", "owner", "replacement", MEMBERSHIP),
        ("org", "owner", PRINCIPAL, "replacement"),
    ] {
        assert!(!store.retry_failed_recording_delivery(org, &session, actor, principal, member, now).await.unwrap());
    }
    assert!(store.retry_failed_recording_delivery("org", &session, "owner", PRINCIPAL, MEMBERSHIP, now).await.unwrap());
    assert!(store.retry_failed_recording_delivery("org", &session, "owner", PRINCIPAL, MEMBERSHIP, now).await.unwrap());
    let recording = store.recording("org", &owner, &session, &secrets()).await.unwrap();
    assert_eq!(recording.status, RecordingStatus::Pending);
    assert!(recording.briefcase_link.is_some());
    let claims = store.claim_recording_deliveries(now, now + ChronoDuration::seconds(60), 2).await.unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].kind, RecordingArtifactKind::Commands);
    assert_eq!(claims[0].attempt, 1);
    assert!(!store.bind_recording_delivery(&claims[0], &"b".repeat(64), 20, now).await.unwrap());
    begin(&store, &claims[0], now).await;
    store.complete_recording_delivery(&claims[0], &receipt(&claims[0]), &secrets(), now).await.unwrap();
    assert_eq!(store.recording("org", &owner, &session, &secrets()).await.unwrap().status, RecordingStatus::Available);
}

#[tokio::test]
async fn owner_retry_cannot_revive_permanent_sources_or_hidden_recordings() {
    for reason in
        ["native_recording_unavailable", "recording_source_invalid", "recording_source_changed", "recording_size_limit"]
    {
        let (store, owner, session, _) = fixture(IdentityKind::Carbon, true, false).await;
        let now = Utc::now();
        let claim =
            store.claim_recording_deliveries(now, now + ChronoDuration::seconds(60), 2).await.unwrap().pop().unwrap();
        store.fail_recording_delivery(&claim, reason, now).await.unwrap();
        if reason == "recording_size_limit" {
            store.trash_recording("org", &owner, &session, now, &secrets()).await.unwrap();
        }
        assert!(
            !store.retry_failed_recording_delivery("org", &session, "owner", PRINCIPAL, MEMBERSHIP, now).await.unwrap()
        );
    }
}

#[tokio::test]
async fn public_identity_projection_never_rewrites_a_verified_partial_receipt_path() {
    let (store, owner, session, _) = fixture(IdentityKind::Silicon, true, false).await;
    let now = Utc::now();
    store
        .remember_identity_projection("org", PRINCIPAL, &owner.id, IdentityKind::Silicon, &secrets(), now)
        .await
        .unwrap();
    let claims = store.claim_recording_deliveries(now, now + ChronoDuration::seconds(60), 2).await.unwrap();
    let video = claims.iter().find(|c| c.kind == RecordingArtifactKind::Video).unwrap();
    begin(&store, video, now).await;
    let receipt = receipt(video);
    assert!(store.complete_recording_delivery(video, &receipt, &secrets(), now).await.unwrap());
    assert_eq!(store.recording("org", &owner, &session, &secrets()).await.unwrap().status, RecordingStatus::Pending);
    let renamed = store
        .remember_identity_projection("org", PRINCIPAL, "renamed-owner", IdentityKind::Silicon, &secrets(), now)
        .await
        .unwrap();
    let recording = store.recording("org", &renamed, &session, &secrets()).await.unwrap();
    assert_eq!(recording.owner_id, "renamed-owner");
    assert_eq!(recording.briefcase_path, receipt.path);
    assert_eq!(recording.briefcase_link.as_deref(), Some(receipt.permanent_url.as_str()));
    let persisted: String =
        sqlx::query_scalar("SELECT artifact_path FROM recording_artifacts WHERE session_id = ? AND kind = 'video'")
            .bind(&session)
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(persisted, receipt.path);
}

#[tokio::test]
async fn selected_provider_destination_is_pinned_before_upload_and_receipt_stays_origin_owned() {
    let (store, owner, session, _) = fixture(IdentityKind::Carbon, true, false).await;
    let now = Utc::now();
    let claims = store.claim_recording_deliveries(now, now + ChronoDuration::seconds(60), 2).await.unwrap();
    let claim = &claims[0];
    assert!(store.bind_recording_destination(claim, "selected-org", "si:selected").await.unwrap());
    assert!(store.bind_recording_destination(claim, "selected-org", "si:selected").await.unwrap());
    assert!(!store.bind_recording_destination(claim, "other-org", "si:selected").await.unwrap());
    assert!(!store.bind_recording_destination(claim, "selected-org", "si:other").await.unwrap());
    begin(&store, claim, now).await;
    assert!(store.complete_recording_delivery(claim, &receipt(claim), &secrets(), now).await.is_err());
    let mut selected = receipt(claim);
    selected.org_id = "selected-org".into();
    selected.path = format!("apps/browser/private/si:selected/{}", selected.name);
    assert!(store.complete_recording_delivery(claim, &selected, &secrets(), now).await.unwrap());
    let recording = store.recording("org", &owner, &session, &secrets()).await.unwrap();
    assert_eq!(recording.status, RecordingStatus::Available);
    assert!(store.recording("selected-org", &owner, &session, &secrets()).await.is_err());
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
    sqlx::raw_sql(include_str!("../../migrations/0009_recording_obo_consent.sql")).execute(&pool).await.unwrap();
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
