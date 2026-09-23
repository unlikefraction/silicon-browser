use super::*;
use sha2::{Digest, Sha256};
use silicon_browser_shared::CommandReport;

const PRINCIPAL: &str = "00000000-0000-0000-0000-000000000011";
const MEMBERSHIP: &str = "00000000-0000-0000-0000-000000000022";
const PAYLOAD: &str = r#"{"slt":null,"access":"oat_exact_old_bytes","refresh":"ort_exact_family"}"#;

fn mapping() -> Vec<PublicIdentifierMapping> {
    serde_json::from_value(serde_json::json!([
        {"org_id":"bricks", "kind":"silicon", "old_id":"chef:bricks", "new_id":"si:chef"},
        {"org_id":"bricks", "kind":"carbon", "old_id":"alice0", "new_id":"c:alice0"}
    ]))
    .unwrap()
}

fn secrets() -> SecretBox {
    SecretBox::new(&[37; 32])
}

fn actor(id: &str) -> Identity {
    Identity { id: id.into(), name: id.into(), kind: IdentityKind::Silicon, tags: vec![], verified_aliases: vec![] }
}

fn lines(logs: &[SessionLog]) -> Vec<u8> {
    logs.iter()
        .flat_map(|log| {
            let mut value = serde_json::to_vec(log).unwrap();
            value.push(b'\n');
            value
        })
        .collect()
}

async fn fixture() -> (Store, String, String, CommandReport, RecordingDeliveryClaim) {
    let store = Store::in_memory().await.unwrap();
    let now = Utc::now();
    let owner = actor(PRINCIPAL);
    let profile = store
        .create_profile(
            "bricks",
            &owner,
            &ProfileCreate {
                name: "chef:bricks authored this".into(),
                location: "in".into(),
                access: AccessList::default(),
            },
            "provider-chef:bricks",
            "fingerprint",
            now,
        )
        .await
        .unwrap();
    sqlx::query("UPDATE profiles SET access_json = ?")
        .bind(format!(r#"["@{PRINCIPAL}","@alice0","chef:bricks"]"#))
        .execute(&store.pool)
        .await
        .unwrap();
    let session = store
        .reserve_session(
            "bricks",
            &owner,
            &SessionCreate::with_profile(
                &profile.id,
                "Old chef:bricks",
                "Don't replace chef:bricks here",
                SessionTtl::Minutes15,
            ),
            now,
        )
        .await
        .unwrap();
    store.bind_session_delivery_owner("bricks", &session.id, PRINCIPAL, MEMBERSHIP).await.unwrap();
    store
        .activate_session(
            "bricks",
            &session.id,
            &ProviderSession {
                id: "provider-session-chef:bricks".into(),
                cdp_url: "wss://example.com/chef:bricks".into(),
                live_url: "https://example.com/chef:bricks".into(),
                recording_url: None,
            },
            &secrets(),
            now,
        )
        .await
        .unwrap();
    let report = CommandReport {
        command_id: Uuid::new_v4(),
        command: "echo chef:bricks".into(),
        flags: vec![],
        started_at: now,
        finished_at: now,
        exit_code: 0,
        truncated: false,
    };
    store.report_command("bricks", &owner, PRINCIPAL, &session.id, &report, &secrets(), now).await.unwrap();
    store.end_session("bricks", &session.id, &SessionEnd { note: "finished".into() }, now).await.unwrap();
    let claim =
        store.claim_recording_deliveries(now, now + ChronoDuration::seconds(60), 2).await.unwrap().pop().unwrap();
    let bytes = lines(&store.recording_delivery_logs(&claim, 0, 128, &secrets()).await.unwrap());
    store.bind_recording_delivery(&claim, &hex::encode(Sha256::digest(&bytes)), bytes.len() as u64, now).await.unwrap();
    sqlx::query("INSERT INTO identity_projection(org_id,principal_id,public_id,kind,updated_at) VALUES('bricks',?,'chef:bricks','silicon',?)")
        .bind(PRINCIPAL).bind(timestamp(now)).execute(&store.pool).await.unwrap();
    let encrypted = secrets()
        .seal_for(&format!("delivery/bricks/chef:bricks/{PRINCIPAL}/{MEMBERSHIP}/credential-uuid/credentials"), PAYLOAD)
        .unwrap();
    sqlx::query("INSERT INTO delivery_credentials(id,org_id,actor_id,principal_id,membership_id,actor_kind,enabled,state,mutation_key,enrollment_digest,encrypted_payload,created_at,updated_at) VALUES('credential-uuid','bricks','chef:bricks',?,?,'\"silicon\"',1,'active','unchanged-key','unchanged-hash',?,0,0)")
        .bind(PRINCIPAL).bind(MEMBERSHIP).bind(encrypted).execute(&store.pool).await.unwrap();
    sqlx::query("INSERT INTO discovery_log(id,org_id,actor_id,kind,purpose,item_count,created_at) VALUES('discovery-uuid','bricks','alice0','search','literal alice0',1,?)")
        .bind(timestamp(now)).execute(&store.pool).await.unwrap();
    let receipt = secrets()
        .seal_for(
            &session_secret_context("bricks", &session.id, "briefcase-url"),
            "https://briefcase.example/private/chef:bricks/receipt",
        )
        .unwrap();
    sqlx::query("UPDATE recordings SET artifact_path='private/chef:bricks/frozen.mp4', briefcase_url_enc=?")
        .bind(receipt)
        .execute(&store.pool)
        .await
        .unwrap();
    (store, profile.id, session.id, report, claim)
}

#[tokio::test]
async fn public_identifier_migration_preserves_resources_credentials_replay_and_frozen_bytes() {
    let (store, profile, session, report, old_claim) = fixture().await;
    let before = lines(&store.recording_delivery_logs(&old_claim, 0, 128, &secrets()).await.unwrap());
    let provider: String =
        sqlx::query_scalar("SELECT provider_cdp_url_enc FROM sessions").fetch_one(&store.pool).await.unwrap();
    let receipt: String =
        sqlx::query_scalar("SELECT briefcase_url_enc FROM recordings").fetch_one(&store.pool).await.unwrap();
    let command_hash: String =
        sqlx::query_scalar("SELECT payload_sha256 FROM command_reports").fetch_one(&store.pool).await.unwrap();
    assert!(store.ensure_public_identifiers_migrated("").await.is_err());
    store.migrate_public_identifiers("", &mapping(), &secrets()).await.unwrap();
    store.ensure_public_identifiers_migrated("").await.unwrap();
    let owner = actor("si:chef");
    let updated = store.profile("bricks", &owner, &profile).await.unwrap();
    assert_eq!(updated.id, profile);
    assert_eq!(updated.owner_id, "si:chef");
    assert_eq!(updated.name, "chef:bricks authored this");
    assert_eq!(updated.access.as_slice(), ["@si:chef", "@c:alice0", "chef:bricks"]);
    let updated = store.session("bricks", &owner, &session).await.unwrap();
    assert_eq!(updated.id, session);
    assert_eq!(updated.initiator_id, "si:chef");
    assert_eq!(updated.participant_ids, ["si:chef"]);
    assert_eq!(updated.description, "Don't replace chef:bricks here");
    let claim = RecordingDeliveryClaim {
        actor_id: "si:chef".into(),
        principal_id: Some("si:chef".into()),
        membership_id: Some("si:chef[bricks]".into()),
        ..old_claim
    };
    let after = lines(&store.recording_delivery_logs(&claim, 0, 128, &secrets()).await.unwrap());
    assert_eq!(before, after, "frozen command archive bytes must survive migration");
    let logs = store.session_logs("bricks", &owner, &session, None, &secrets()).await.unwrap();
    assert_eq!(logs[0].actor_id, "si:chef");
    assert_eq!(logs[0].command, "echo chef:bricks");
    let hash: String =
        sqlx::query_scalar("SELECT body_sha256 FROM recording_artifacts").fetch_one(&store.pool).await.unwrap();
    assert_eq!(hash, hex::encode(Sha256::digest(&before)));
    let row = sqlx::query("SELECT * FROM delivery_credentials").fetch_one(&store.pool).await.unwrap();
    let payload: &str = row.try_get("encrypted_payload").unwrap();
    assert_eq!(
        secrets()
            .open_for("delivery/bricks/si:chef/si:chef/si:chef[bricks]/credential-uuid/credentials", payload)
            .unwrap(),
        PAYLOAD
    );
    assert_eq!(row.try_get::<&str, _>("mutation_key").unwrap(), "unchanged-key");
    assert_eq!(row.try_get::<&str, _>("enrollment_digest").unwrap(), "unchanged-hash");
    assert_eq!(row.try_get::<&str, _>("id").unwrap(), "credential-uuid");
    let authority: (String, String) =
        sqlx::query_as("SELECT delivery_principal_id, delivery_membership_id FROM sessions")
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(authority, ("si:chef".into(), "si:chef[bricks]".into()));
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT provider_cdp_url_enc FROM sessions")
            .fetch_one(&store.pool)
            .await
            .unwrap(),
        provider
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT briefcase_url_enc FROM recordings")
            .fetch_one(&store.pool)
            .await
            .unwrap(),
        receipt
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT artifact_path FROM recordings").fetch_one(&store.pool).await.unwrap(),
        "private/chef:bricks/frozen.mp4"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT payload_sha256 FROM command_reports")
            .fetch_one(&store.pool)
            .await
            .unwrap(),
        command_hash
    );
    let replay =
        store.report_command("bricks", &owner, "si:chef", &session, &report, &secrets(), Utc::now()).await.unwrap();
    assert_eq!(replay.sequence, Some(1));
    store.migrate_public_identifiers("", &mapping(), &secrets()).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT encrypted_payload FROM delivery_credentials")
            .fetch_one(&store.pool)
            .await
            .unwrap(),
        payload
    );
}

#[tokio::test]
async fn public_identifier_migration_failure_rolls_back_every_change() {
    for failure in [
        "unmapped",
        "collision",
        "binding-collision",
        "kind",
        "principal",
        "membership",
        "bad-key",
        "pending",
        "uncertain",
        "projection",
    ] {
        let (store, _, _, _, _) = fixture().await;
        let mut map = mapping();
        let mut key = secrets();
        match failure {
            "unmapped" => {
                map.pop();
            }
            "collision" => {
                let mut other = map[0].clone();
                other.org_id = "other".into();
                other.old_id = "chef:other".into();
                map.push(other);
            }
            "binding-collision" => {
                sqlx::query("INSERT INTO identity_projection(org_id,principal_id,public_id,kind,updated_at) VALUES('bricks','00000000-0000-0000-0000-000000000033','si:chef','silicon','unchanged')").execute(&store.pool).await.unwrap();
            }
            "kind" => {
                map[0].kind = IdentityKind::Carbon;
            }
            "principal" => {
                sqlx::query("UPDATE command_reports SET principal_id='missing'").execute(&store.pool).await.unwrap();
            }
            "membership" => {
                sqlx::query("UPDATE delivery_credentials SET membership_id='chef:bricks[other]'")
                    .execute(&store.pool)
                    .await
                    .unwrap();
            }
            "bad-key" => {
                key = SecretBox::new(&[99; 32]);
            }
            "pending" => {
                sqlx::query("UPDATE delivery_credentials SET operation='refresh'").execute(&store.pool).await.unwrap();
            }
            "uncertain" => {
                sqlx::query("UPDATE recording_artifacts SET state='uploading'").execute(&store.pool).await.unwrap();
            }
            "projection" => {
                sqlx::query("UPDATE identity_projection SET principal_id='c:another'")
                    .execute(&store.pool)
                    .await
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let command: String =
            sqlx::query_scalar("SELECT command_enc FROM commands").fetch_one(&store.pool).await.unwrap();
        let credentials: String = sqlx::query_scalar("SELECT encrypted_payload FROM delivery_credentials")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert!(store.migrate_public_identifiers("", &map, &key).await.is_err(), "{failure}");
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT owner_id FROM profiles").fetch_one(&store.pool).await.unwrap(),
            PRINCIPAL,
            "{failure}"
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT command_enc FROM commands").fetch_one(&store.pool).await.unwrap(),
            command,
            "{failure}"
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT encrypted_payload FROM delivery_credentials")
                .fetch_one(&store.pool)
                .await
                .unwrap(),
            credentials,
            "{failure}"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM public_identifier_schema")
                .fetch_one(&store.pool)
                .await
                .unwrap(),
            0,
            "{failure}"
        );
    }
}

#[tokio::test]
async fn public_identifier_migration_fences_worlds_and_selected_membership_organizations() {
    let (store, _, _, _, _) = fixture().await;
    let scope = Uuid::new_v4().to_string();
    let mut map = mapping();
    for entry in &mut map {
        entry.scope_key = scope.clone();
    }
    assert!(store.migrate_public_identifiers("", &map, &secrets()).await.is_err());
    assert!(store.migrate_public_identifiers(&Uuid::new_v4().to_string(), &map, &secrets()).await.is_err());
    sqlx::query("INSERT INTO identity_projection(org_id,principal_id,public_id,kind,updated_at) VALUES('invited',?,'chef:bricks','silicon','unchanged')")
        .bind(PRINCIPAL).execute(&store.pool).await.unwrap();
    assert!(store.migrate_public_identifiers(&scope, &map, &secrets()).await.is_err());
    let mut invited = map[0].clone();
    invited.org_id = "invited".into();
    invited.owning_org_id = Some("bricks".into());
    map.push(invited);
    map.last_mut().unwrap().new_id = "si:imposter".into();
    assert!(store.migrate_public_identifiers(&scope, &map, &secrets()).await.is_err());
    map.last_mut().unwrap().new_id = "si:chef".into();
    store.migrate_public_identifiers(&scope, &map, &secrets()).await.unwrap();
    assert!(store.ensure_public_identifiers_migrated("").await.is_err());
    assert!(store.ensure_public_identifiers_migrated(&Uuid::new_v4().to_string()).await.is_err());
    store.ensure_public_identifiers_migrated(&scope).await.unwrap();
    sqlx::query("UPDATE sessions SET started_by_kind='carbon'").execute(&store.pool).await.unwrap();
    assert!(store.ensure_public_identifiers_migrated(&scope).await.is_err());
}

#[tokio::test]
async fn public_identifier_migration_fresh_store_binds_its_world_at_startup() {
    let store = Store::in_memory().await.unwrap();
    let world = Uuid::new_v4().to_string();
    store.ensure_public_identifiers_migrated(&world).await.unwrap();
    assert!(store.ensure_public_identifiers_migrated("").await.is_err());
    assert!(store.migrate_public_identifiers("", &[], &secrets()).await.is_err());
    store.migrate_public_identifiers(&world, &[], &secrets()).await.unwrap();
    store.ensure_public_identifiers_migrated(&world).await.unwrap();
}

#[tokio::test]
async fn public_identifier_migration_upgrades_deployed_schema_without_changing_frozen_actor_spelling() {
    let (store, _, _, _, old_claim) = fixture().await;
    store.migrate_public_identifiers("", &mapping(), &secrets()).await.unwrap();
    let claim = RecordingDeliveryClaim { actor_id: "si:chef".into(), ..old_claim };
    let before = lines(&store.recording_delivery_logs(&claim, 0, 128, &secrets()).await.unwrap());
    // Reproduce the already canonical deployed v8 store, which has the original
    // delivery_actor_id column but predates the explicit world marker.
    sqlx::raw_sql("DROP TABLE public_identifier_schema; DELETE FROM _sqlx_migrations WHERE version = 9;")
        .execute(&store.pool)
        .await
        .unwrap();
    let checksum: String = sqlx::query_scalar("SELECT hex(checksum) FROM _sqlx_migrations WHERE version = 8")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(
        checksum,
        "E4F3A3171263651A58706FAF5FEAA080DBA903D5704AB21EC13F7EEAD98D1CE0DAF6C951A476EEC04B58508070171ABE"
    );
    store.migrate().await.unwrap();
    store.ensure_public_identifiers_migrated("").await.unwrap();
    let after = lines(&store.recording_delivery_logs(&claim, 0, 128, &secrets()).await.unwrap());
    assert_eq!(before, after);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT delivery_actor_id FROM commands").fetch_one(&store.pool).await.unwrap(),
        PRINCIPAL
    );
    let marker: (String, Option<String>) =
        sqlx::query_as("SELECT scope_key, mapping_json FROM public_identifier_schema")
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(marker, (String::new(), None));
}
