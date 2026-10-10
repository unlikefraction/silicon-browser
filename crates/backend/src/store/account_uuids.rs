//! Offline rekey of the existing Accounts database. This does not adopt IAM data.
//! Only declared authority fields change. Resource IDs, provider capabilities,
//! signed/history bytes and token replay digests retain their original meaning.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use silicon_browser_shared::canonical_account_uuid;

const LEDGER: &str = include_str!("../../migrations/0015_account_uuid_rekeys.sql");
const REFERENCES: &[(&str, &str)] = &[
    ("profiles", "owner_id"),
    ("sessions", "started_by"),
    ("session_participants", "actor_id"),
    ("commands", "actor_id"),
    ("recordings", "owner_id"),
    ("discovery_log", "actor_id"),
    ("command_reports", "actor_id"),
    ("command_reports", "principal_id"),
];

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct Mapping {
    old_uuid: String,
    new_uuid: String,
    kind: String,
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub dry_run: bool,
    pub map_sha256: String,
    pub mapped_accounts: usize,
    pub already_applied: bool,
    pub verified_ciphertexts: u64,
    pub changed: BTreeMap<String, u64>,
}

fn invalid_map(message: &str) -> StoreError {
    StoreError::Invalid(message.into())
}

fn parse(bytes: &[u8]) -> StoreResult<BTreeMap<String, Mapping>> {
    let mut reader = csv::Reader::from_reader(bytes);
    if !reader.headers().map_err(|_| invalid_map("invalid mapping CSV"))?.iter().eq(["old_uuid", "new_uuid", "kind"]) {
        return Err(invalid_map("CSV header must be exactly old_uuid,new_uuid,kind"));
    }
    let mut mappings = BTreeMap::new();
    let mut targets = BTreeSet::new();
    for row in reader.deserialize::<Mapping>() {
        let row = row.map_err(|_| invalid_map("invalid mapping CSV row"))?;
        if !(3..=128).contains(&row.old_uuid.len())
            || !row.old_uuid.bytes().all(|b| b.is_ascii_alphanumeric())
            || !canonical_account_uuid(&row.new_uuid)
            || Uuid::parse_str(&row.new_uuid).map(|id| id.get_version_num()).unwrap_or_default() != 4
            || !matches!(row.kind.as_str(), "carbon" | "silicon")
        {
            return Err(invalid_map(
                "mapping requires a legacy base62 source, canonical lowercase UUIDv4 target, and carbon or silicon kind",
            ));
        }
        if !targets.insert(row.new_uuid.clone()) || mappings.insert(row.old_uuid.clone(), row).is_some() {
            return Err(invalid_map("duplicate source or target; identity merges are forbidden"));
        }
    }
    Ok(mappings)
}

fn check_reference(id: &str, mappings: &BTreeMap<String, Mapping>, seen: &mut BTreeSet<String>) -> StoreResult<()> {
    if !canonical_account_uuid(id) && !mappings.contains_key(id) {
        return Err(invalid_map(
            "unmapped noncanonical account reference; obtain the complete authoritative Accounts export",
        ));
    }
    seen.insert(id.to_owned());
    Ok(())
}

/// A read-only dry run never creates a database, applies schema migrations or
/// changes ciphertext. Apply takes a SQLite write lock and commits all changes
/// together. Stop the service/other writers and take a SQLite backup first.
pub async fn migrate(database_url: &str, csv: &[u8], secrets: &SecretBox, apply: bool) -> StoreResult<Report> {
    let mappings = parse(csv)?;
    let hash = hex::encode(Sha256::digest(csv));
    let options = SqliteConnectOptions::from_str(database_url)?;
    if database_url.contains(":memory:") || !options.get_filename().is_file() {
        return Err(invalid_map("migration requires an existing on-disk Accounts database"));
    }
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            options.create_if_missing(false).read_only(!apply).foreign_keys(true).busy_timeout(Duration::from_secs(10)),
        )
        .await?;
    let result = migrate_pool(&pool, mappings, hash, secrets, apply).await;
    pool.close().await;
    result
}

async fn migrate_pool(
    pool: &SqlitePool,
    mappings: BTreeMap<String, Mapping>,
    hash: String,
    secrets: &SecretBox,
    apply: bool,
) -> StoreResult<Report> {
    let mut tx = pool.begin_with(if apply { "BEGIN IMMEDIATE" } else { "BEGIN" }).await?;
    check_accounts_ownership(&mut tx).await?;
    let schema: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM _sqlx_migrations WHERE version=14 AND success=1)")
            .fetch_one(&mut *tx)
            .await?;
    if !schema {
        return Err(invalid_map(
            "expected the existing Accounts schema (migration 0014); do not migrate an IAM database",
        ));
    }
    // These fields are legacy delivery authority, not current Accounts state.
    // Adopting them would require separate, verified IAM provenance.
    let legacy_delivery: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE delivery_principal_id IS NOT NULL OR delivery_membership_id IS NOT NULL OR delivery_destination_org IS NOT NULL OR delivery_destination_actor IS NOT NULL)")
        .fetch_one(&mut *tx).await?;
    if legacy_delivery {
        return Err(invalid_map(
            "legacy IAM delivery authority is present; preserve this store and investigate provenance",
        ));
    }
    let ledger_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='account_uuid_cutover')",
    )
    .fetch_one(&mut *tx)
    .await?;
    let applied_hash: Option<String> = if ledger_exists {
        sqlx::query_scalar("SELECT map_sha256 FROM account_uuid_cutover WHERE singleton=1")
            .fetch_optional(&mut *tx)
            .await?
    } else {
        None
    };
    let mut report = Report {
        dry_run: !apply,
        map_sha256: hash.clone(),
        mapped_accounts: mappings.len(),
        already_applied: applied_hash.is_some(),
        ..Report::default()
    };
    if let Some(applied_hash) = applied_hash {
        if applied_hash != hash {
            return Err(invalid_map("cutover already applied with a different CSV; the original mapping is immutable"));
        }
        let ledger: Vec<(String, String, String, String)> =
            sqlx::query_as("SELECT old_uuid,new_uuid,kind,map_sha256 FROM account_uuid_rekeys")
                .fetch_all(&mut *tx)
                .await?;
        if ledger.len() != mappings.len()
            || ledger.iter().any(|(old, new, kind, digest)| {
                mappings.get(old).is_none_or(|row| row.new_uuid != *new || row.kind != *kind || digest != &hash)
            })
        {
            return Err(invalid_map("cutover ledger does not match the immutable mapping"));
        }
    } else if ledger_exists {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM account_uuid_rekeys").fetch_one(&mut *tx).await?;
        if count != 0 {
            return Err(invalid_map("incomplete cutover ledger; restore the verified backup"));
        }
    }

    let mut seen = BTreeSet::new();
    for (table, column) in REFERENCES {
        let ids: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT {column} FROM {table}")))
            .fetch_all(&mut *tx)
            .await?;
        let mut changed = 0;
        for id in ids {
            check_reference(&id, &mappings, &mut seen)?;
            changed += u64::from(mappings.contains_key(&id));
        }
        if changed != 0 {
            report.changed.insert(format!("{table}.{column}"), changed);
        }
    }
    let mut access_updates = Vec::new();
    for (id, raw) in
        sqlx::query_as::<_, (String, String)>("SELECT id,access_json FROM profiles").fetch_all(&mut *tx).await?
    {
        let mut entries: Vec<String> = serde_json::from_str(&raw).map_err(corrupt_json)?;
        let mut changed = false;
        for entry in &mut entries {
            if let Some(account) = entry.strip_prefix('@') {
                check_reference(account, &mappings, &mut seen)?;
                if let Some(row) = mappings.get(account) {
                    *entry = format!("@{}", row.new_uuid);
                    changed = true;
                }
            }
        }
        if changed {
            access_updates.push((id, serde_json::to_string(&entries).map_err(corrupt_json)?));
        }
    }
    for (id, kind) in sqlx::query_as::<_, (String, String)>("SELECT started_by,started_by_kind FROM sessions")
        .fetch_all(&mut *tx)
        .await?
    {
        if mappings.get(&id).is_some_and(|row| row.kind != kind) {
            return Err(invalid_map("stored account kind differs from authoritative mapping"));
        }
    }
    if report.already_applied && seen.iter().any(|id| mappings.contains_key(id)) {
        return Err(invalid_map(
            "a retired account reference was reintroduced after cutover; restore or investigate before replay",
        ));
    }
    if !report.already_applied && mappings.values().any(|row| seen.contains(&row.new_uuid)) {
        return Err(invalid_map("mapping target is already present; merging existing identities is forbidden"));
    }
    // Provider/recording AAD contains stable resource IDs, so retain exact ciphertext
    // while proving the configured key still decrypts every retained capability.
    for row in sqlx::query("SELECT s.id,s.org_id,s.provider_cdp_url_enc,s.provider_live_url_enc,s.provider_recording_url_enc,r.briefcase_url_enc FROM sessions s LEFT JOIN recordings r ON r.session_id=s.id").fetch_all(&mut *tx).await? {
        for (column, context) in [("provider_cdp_url_enc", "provider-cdp-url"), ("provider_live_url_enc", "provider-live-url"), ("provider_recording_url_enc", "provider-recording-url"), ("briefcase_url_enc", "briefcase-url")] {
            if let Some(cipher) = row.try_get::<Option<String>,_>(column)? {
                secrets.open_for(&session_secret_context(row.try_get("org_id")?, row.try_get("id")?, context), &cipher).map_err(StoreError::Crypto)?;
                report.verified_ciphertexts += 1;
            }
        }
    }
    for row in sqlx::query("SELECT a.session_id,a.kind,a.receipt_url_enc,s.org_id FROM recording_artifacts a JOIN sessions s ON s.id=a.session_id WHERE a.receipt_url_enc IS NOT NULL").fetch_all(&mut *tx).await? {
        let org: &str = row.try_get("org_id")?;
        let session: &str = row.try_get("session_id")?;
        let kind: &str = row.try_get("kind")?;
        secrets.open_for(&format!("recording-artifact:{org}:{session}:{kind}"), row.try_get("receipt_url_enc")?).map_err(StoreError::Crypto)?;
        report.verified_ciphertexts += 1;
    }
    // Authenticate every command, including unaffected rows, before any update.
    // Preserve historical serialized actor spelling used by frozen artifacts.
    let mut commands = Vec::new();
    for row in sqlx::query("SELECT c.session_id,c.sequence,c.actor_id,c.command_enc,s.org_id FROM commands c JOIN sessions s ON s.id=c.session_id").fetch_all(&mut *tx).await? {
        let session: String = row.try_get("session_id")?;
        let sequence: i64 = row.try_get("sequence")?;
        let actor: String = row.try_get("actor_id")?;
        let org: String = row.try_get("org_id")?;
        let plain = secrets.open_for(&command_secret_context(&org, &session, sequence, &actor), row.try_get("command_enc")?).map_err(StoreError::Crypto)?;
        report.verified_ciphertexts += 1;
        if let Some(mapping) = mappings.get(&actor) {
            // Dry run verifies the key/AAD without generating a replacement nonce.
            let cipher = if apply { secrets.seal_for(&command_secret_context(&org, &session, sequence, &mapping.new_uuid), &plain).map_err(StoreError::Crypto)? } else { String::new() };
            commands.push((session, sequence, cipher));
        }
    }
    if !access_updates.is_empty() {
        report.changed.insert("profiles.access_json".into(), access_updates.len() as u64);
    }
    if !commands.is_empty() {
        report.changed.insert("commands.command_enc".into(), commands.len() as u64);
    }
    if !report.already_applied {
        let pending: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM accounts_login_attempts WHERE claimed_at IS NULL AND expires_at>0",
        )
        .fetch_one(&mut *tx)
        .await?;
        if pending != 0 {
            report.changed.insert("accounts_login_attempts.expired".into(), pending as u64);
        }
    }
    if !sqlx::query("PRAGMA foreign_key_check").fetch_all(&mut *tx).await?.is_empty() {
        return Err(invalid_map("foreign key check failed; preserve and repair the source database first"));
    }
    if apply && !report.already_applied {
        for (session, sequence, cipher) in commands {
            sqlx::query("UPDATE commands SET command_enc=?,delivery_actor_id=COALESCE(delivery_actor_id,actor_id) WHERE session_id=? AND sequence=?")
                .bind(cipher).bind(session).bind(sequence).execute(&mut *tx).await?;
        }
        for row in mappings.values() {
            for (table, column) in REFERENCES {
                sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE {table} SET {column}=? WHERE {column}=?")))
                    .bind(&row.new_uuid)
                    .bind(&row.old_uuid)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        for (id, access) in access_updates {
            sqlx::query("UPDATE profiles SET access_json=? WHERE id=?").bind(access).bind(id).execute(&mut *tx).await?;
        }
        sqlx::query("UPDATE accounts_login_attempts SET expires_at=0 WHERE claimed_at IS NULL AND expires_at>0")
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql(LEDGER).execute(&mut *tx).await?;
        let now = timestamp(Utc::now());
        for row in mappings.values() {
            sqlx::query(
                "INSERT INTO account_uuid_rekeys(old_uuid,new_uuid,kind,map_sha256,migrated_at) VALUES(?,?,?,?,?)",
            )
            .bind(&row.old_uuid)
            .bind(&row.new_uuid)
            .bind(&row.kind)
            .bind(&hash)
            .bind(&now)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("INSERT INTO account_uuid_cutover(singleton,map_sha256,migrated_at) VALUES(1,?,?)")
            .bind(hash)
            .bind(now)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(report)
}

impl Store {
    /// Once cut over, stale introspection/cache results and direct old-ID ACL
    /// selectors must fail. The ledger is never used to authenticate an alias.
    pub async fn account_subject_retired(&self, subject: &str) -> StoreResult<bool> {
        let cutover: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM account_uuid_cutover WHERE singleton=1)")
            .fetch_one(&self.pool)
            .await?;
        Ok(cutover && !canonical_account_uuid(subject))
    }
}

#[cfg(test)]
mod tests;
