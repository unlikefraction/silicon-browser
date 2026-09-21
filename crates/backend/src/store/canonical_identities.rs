//! Upgrade verified IAM ownership and encrypted delivery families before workers start.
use super::*;

impl Store {
    /// Uses only the directory projections previously verified against IAM.
    /// Each identity's ownership and ciphertext conversion commits together;
    /// interruption is safe to resume and never creates a new IAM family.
    pub async fn canonicalize_iam_identities(&self, secrets: &SecretBox) -> StoreResult<()> {
        let projections: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT org_id,public_id,kind FROM identity_projection WHERE principal_id<>public_id ORDER BY org_id,public_id",
        ).fetch_all(self.pool()).await?;
        for (org, public, kind) in projections {
            if Uuid::parse_str(&public).is_ok() {
                return Err(StoreError::Invalid(
                    "legacy IAM identity has no verified canonical public identifier".into(),
                ));
            }
            let kind = match kind.as_str() {
                "carbon" => IdentityKind::Carbon,
                "silicon" => IdentityKind::Silicon,
                _ => return Err(StoreError::Invalid("unknown IAM identity kind".into())),
            };
            self.remember_identity_projection(&org, &public, &public, kind, secrets, Utc::now()).await?;
        }
        // An orphan private UUID must be resolved from an operator-verified IAM
        // export. Guessing a user from a name or silently replacing a family
        // would change authority and lose an in-flight recording delivery.
        let remaining:Vec<String>=sqlx::query_scalar("SELECT principal_id FROM delivery_credentials UNION SELECT delivery_principal_id FROM sessions WHERE delivery_principal_id IS NOT NULL UNION SELECT principal_id FROM command_reports UNION SELECT owner_id FROM profiles UNION SELECT started_by FROM sessions UNION SELECT owner_id FROM recordings").fetch_all(self.pool()).await?;
        if remaining.iter().any(|id| Uuid::parse_str(id).is_ok()) {
            return Err(StoreError::Invalid("canonical IAM cutover needs a verified identity projection for remaining legacy UUID ownership; existing credentials were retained".into()));
        }
        Ok(())
    }
}

pub(super) async fn rewrite_authority(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    secrets: &SecretBox,
    org: &str,
    legacy: &str,
    public: &str,
) -> StoreResult<()> {
    let grants=sqlx::query("SELECT id,actor_id,principal_id,membership_id,encrypted_payload FROM delivery_credentials WHERE org_id=? AND principal_id=?")
        .bind(org).bind(legacy).fetch_all(&mut **tx).await?;
    let canonical_member = format!("{public}[{org}]");
    for row in grants {
        let id: String = row.try_get("id")?;
        let actor: String = row.try_get("actor_id")?;
        let principal: String = row.try_get("principal_id")?;
        let member: String = row.try_get("membership_id")?;
        if Uuid::parse_str(&member).is_err() && member != canonical_member {
            return Err(StoreError::Invalid("stored delivery membership does not match verified IAM ownership".into()));
        }
        let encrypted: String = row.try_get("encrypted_payload")?;
        let plaintext = secrets
            .open_for(&format!("delivery/{org}/{actor}/{principal}/{member}/{id}/credentials"), &encrypted)
            .map_err(StoreError::Crypto)?;
        let encrypted = secrets
            .seal_for(&format!("delivery/{org}/{public}/{public}/{canonical_member}/{id}/credentials"), &plaintext)
            .map_err(StoreError::Crypto)?;
        sqlx::query(
            "UPDATE delivery_credentials SET actor_id=?,principal_id=?,membership_id=?,encrypted_payload=? WHERE id=?",
        )
        .bind(public)
        .bind(public)
        .bind(&canonical_member)
        .bind(encrypted)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    }
    sqlx::query("UPDATE sessions SET delivery_principal_id=?,delivery_membership_id=? WHERE org_id=? AND delivery_principal_id=?")
        .bind(public).bind(&canonical_member).bind(org).bind(legacy).execute(&mut **tx).await?;
    sqlx::query("UPDATE command_reports SET principal_id=?,actor_id=? WHERE principal_id=? AND session_id IN(SELECT id FROM sessions WHERE org_id=?)")
        .bind(public).bind(public).bind(legacy).bind(org).execute(&mut **tx).await?;
    sqlx::query("DELETE FROM identity_projection WHERE org_id=? AND principal_id=? AND principal_id<>public_id")
        .bind(org)
        .bind(legacy)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use silicon_browser_shared::CommandReport;
    #[tokio::test]
    async fn canonical_cutover_keeps_frozen_command_artifact_bytes() {
        let store = Store::in_memory().await.unwrap();
        let secrets = SecretBox::new(&[17; 32]);
        let now = Utc::now();
        let legacy = Uuid::from_u128(10).to_string();
        let actor = Identity {
            id: legacy.clone(),
            name: "Chef".into(),
            kind: IdentityKind::Silicon,
            tags: vec![],
            verified_aliases: vec![],
        };
        let session = store
            .reserve_session(
                "bricks",
                &actor,
                &SessionCreate::incognito("Migration", "Recorded work", SessionTtl::Minutes15),
                now,
            )
            .await
            .unwrap();
        store
            .activate_session(
                "bricks",
                &session.id,
                &ProviderSession {
                    id: "provider-session".into(),
                    cdp_url: "wss://provider.example/cdp".into(),
                    live_url: "https://provider.example/live".into(),
                    recording_url: Some("https://provider.example/video".into()),
                },
                &secrets,
                now,
            )
            .await
            .unwrap();
        let report = CommandReport {
            command_id: Uuid::new_v4(),
            command: "snapshot -i".into(),
            flags: vec![],
            started_at: now,
            finished_at: now,
            exit_code: 0,
            truncated: false,
        };
        store.report_command("bricks", &actor, &legacy, &session.id, &report, &secrets, now).await.unwrap();
        store
            .begin_end_session("bricks", &actor, &session.id, &SessionEnd { note: "finished".into() }, &secrets)
            .await
            .unwrap();
        store.mark_recording_pending("bricks", &session.id, 1000, 0).await.unwrap();
        store.finalize_end_session("bricks", &session.id, now).await.unwrap();
        let claim = store
            .claim_recording_deliveries(now, now + chrono::Duration::minutes(1), 2)
            .await
            .unwrap()
            .into_iter()
            .find(|claim| claim.kind == RecordingArtifactKind::Commands)
            .unwrap();
        let before = store.recording_delivery_logs(&claim, 0, 128, &secrets).await.unwrap();
        let jsonl = |logs: &[SessionLog]| {
            logs.iter()
                .flat_map(|log| {
                    let mut line = serde_json::to_vec(log).unwrap();
                    line.push(b'\n');
                    line
                })
                .collect::<Vec<u8>>()
        };
        let frozen_bytes = jsonl(&before);
        let digest = hex::encode(Sha256::digest(&frozen_bytes));
        assert!(store.bind_recording_delivery(&claim, &digest, frozen_bytes.len() as u64, now).await.unwrap());
        sqlx::query("INSERT INTO identity_projection(org_id,principal_id,public_id,kind,updated_at) VALUES('bricks',?,'chef:bricks','silicon',?)").bind(&legacy).bind(timestamp(now)).execute(store.pool()).await.unwrap();
        store.canonicalize_iam_identities(&secrets).await.unwrap();
        let retry = now + chrono::Duration::minutes(2);
        let claim = store
            .claim_recording_deliveries(retry, retry + chrono::Duration::minutes(1), 2)
            .await
            .unwrap()
            .into_iter()
            .find(|claim| claim.kind == RecordingArtifactKind::Commands)
            .unwrap();
        assert_eq!(claim.actor_id, "chef:bricks");
        let after = store.recording_delivery_logs(&claim, 0, 128, &secrets).await.unwrap();
        assert_eq!(jsonl(&after), frozen_bytes);
        assert!(store.bind_recording_delivery(&claim, &digest, frozen_bytes.len() as u64, retry).await.unwrap());
        let canonical = Identity { id: "chef:bricks".into(), ..actor };
        let public_logs = store.session_logs("bricks", &canonical, &session.id, None, &secrets).await.unwrap();
        assert_eq!(public_logs[0].actor_id, "chef:bricks");
    }
    #[tokio::test]
    async fn canonical_cutover_preserves_delivery_family_command_replay_and_ownership() {
        let store = Store::in_memory().await.unwrap();
        let secrets = SecretBox::new(&[17; 32]);
        let now = Utc::now();
        let legacy = Uuid::from_u128(10).to_string();
        let old_member = Uuid::from_u128(20).to_string();
        let public = "chef:bricks";
        let actor = Identity {
            id: legacy.clone(),
            name: "Chef".into(),
            kind: IdentityKind::Silicon,
            tags: vec![],
            verified_aliases: vec![],
        };
        let session = store
            .reserve_session(
                "bricks",
                &actor,
                &SessionCreate::incognito("Migration", "Recorded work", SessionTtl::Minutes15),
                now,
            )
            .await
            .unwrap();
        store.bind_session_delivery_owner("bricks", &session.id, &legacy, &old_member).await.unwrap();
        store
            .activate_session(
                "bricks",
                &session.id,
                &ProviderSession {
                    id: "provider-session".into(),
                    cdp_url: "wss://provider.example/cdp".into(),
                    live_url: "https://provider.example/live".into(),
                    recording_url: None,
                },
                &secrets,
                now,
            )
            .await
            .unwrap();
        let report = CommandReport {
            command_id: Uuid::new_v4(),
            command: "snapshot -i".into(),
            flags: vec![],
            started_at: now,
            finished_at: now,
            exit_code: 0,
            truncated: false,
        };
        let receipt =
            store.report_command("bricks", &actor, &legacy, &session.id, &report, &secrets, now).await.unwrap();
        sqlx::query("INSERT INTO identity_projection(org_id,principal_id,public_id,kind,updated_at) VALUES('bricks',?,?,'silicon',?)").bind(&legacy).bind(public).bind(timestamp(now)).execute(store.pool()).await.unwrap();
        let plaintext = r#"{"access":"old-access","refresh":"unchanged-refresh","slt":null}"#;
        let encrypted = secrets
            .seal_for(&format!("delivery/bricks/{legacy}/{legacy}/{old_member}/grant/credentials"), plaintext)
            .unwrap();
        sqlx::query("INSERT INTO delivery_credentials(id,org_id,actor_id,principal_id,membership_id,actor_kind,enabled,state,operation,mutation_key,enrollment_digest,encrypted_payload,created_at,updated_at,lease_owner,lease_until) VALUES('grant','bricks',?,?,?,'\"silicon\"',1,'refreshing','refresh','unchanged-key','unchanged-digest',?,1,2,'unchanged-lease',9999999999)")
            .bind(&legacy).bind(&legacy).bind(&old_member).bind(encrypted).execute(store.pool()).await.unwrap();
        store.canonicalize_iam_identities(&secrets).await.unwrap();
        store.canonicalize_iam_identities(&secrets).await.unwrap();
        let canonical = Identity {
            id: public.into(),
            name: "Chef".into(),
            kind: IdentityKind::Silicon,
            tags: vec![],
            verified_aliases: vec![],
        };
        let replay =
            store.report_command("bricks", &canonical, public, &session.id, &report, &secrets, now).await.unwrap();
        assert_eq!(receipt, replay);
        let logs = store.session_logs("bricks", &canonical, &session.id, None, &secrets).await.unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].actor_id, public);
        assert_eq!(logs[0].command, "snapshot -i");
        let session = store.session("bricks", &canonical, &session.id).await.unwrap();
        assert_eq!(session.initiator_id, public);
        assert_eq!(session.participant_ids, [public]);
        let row =
            sqlx::query("SELECT * FROM delivery_credentials WHERE id='grant'").fetch_one(store.pool()).await.unwrap();
        assert_eq!(row.get::<String, _>("principal_id"), public);
        assert_eq!(row.get::<String, _>("actor_id"), public);
        assert_eq!(row.get::<String, _>("membership_id"), "chef:bricks[bricks]");
        assert_eq!(row.get::<String, _>("state"), "refreshing");
        assert_eq!(row.get::<String, _>("operation"), "refresh");
        assert_eq!(row.get::<String, _>("mutation_key"), "unchanged-key");
        assert_eq!(row.get::<String, _>("enrollment_digest"), "unchanged-digest");
        assert_eq!(row.get::<String, _>("lease_owner"), "unchanged-lease");
        assert_eq!(row.get::<i64, _>("lease_until"), 9999999999);
        assert_eq!(
            secrets
                .open_for(
                    "delivery/bricks/chef:bricks/chef:bricks/chef:bricks[bricks]/grant/credentials",
                    row.get("encrypted_payload")
                )
                .unwrap(),
            plaintext
        );
        let binding: (String, String) =
            sqlx::query_as("SELECT delivery_principal_id,delivery_membership_id FROM sessions WHERE id=?")
                .bind(&session.id)
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(binding, (public.into(), "chef:bricks[bricks]".into()));
        assert!(store.projected_identity("bricks", &legacy, IdentityKind::Silicon).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn canonical_cutover_rolls_back_identity_when_credential_authentication_fails() {
        let store = Store::in_memory().await.unwrap();
        let legacy = Uuid::from_u128(10).to_string();
        let secrets = SecretBox::new(&[17; 32]);
        sqlx::query("INSERT INTO identity_projection(org_id,principal_id,public_id,kind,updated_at) VALUES('bricks',?,'chef:bricks','silicon','now')").bind(&legacy).execute(store.pool()).await.unwrap();
        sqlx::query("INSERT INTO delivery_credentials(id,org_id,actor_id,principal_id,membership_id,actor_kind,enabled,state,enrollment_digest,encrypted_payload,created_at,updated_at) VALUES('grant','bricks','chef:bricks',?,?,'\"silicon\"',1,'active','retained-digest','tampered',1,2)").bind(&legacy).bind(Uuid::from_u128(20).to_string()).execute(store.pool()).await.unwrap();
        assert!(store.canonicalize_iam_identities(&secrets).await.is_err());
        let identity: String =
            sqlx::query_scalar("SELECT principal_id FROM identity_projection").fetch_one(store.pool()).await.unwrap();
        assert_eq!(identity, legacy);
        let row: (String, String) = sqlx::query_as("SELECT principal_id,encrypted_payload FROM delivery_credentials")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(row, (legacy, "tampered".into()));
    }
}
