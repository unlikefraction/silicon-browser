//! Offline, world-scoped conversion using an authenticated IAM export.
use super::*;
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, Transaction};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublicIdentifierMapping {
    /// IAM's empty production scope or the exact testing environment UUID.
    #[serde(default)]
    pub scope_key: String,
    pub org_id: String,
    #[serde(default)]
    pub owning_org_id: Option<String>,
    pub kind: IdentityKind,
    pub old_id: String,
    pub new_id: String,
}

type ActorMap = HashMap<(String, String), (String, IdentityKind)>;

fn mapped<'a>(map: &'a ActorMap, org: &str, actor: &str, kind: Option<IdentityKind>) -> StoreResult<&'a str> {
    let (new, actual_kind) = map.get(&(org.to_owned(), actor.to_owned())).ok_or_else(|| {
        StoreError::Invalid(format!("unmapped identity {actor} in organization {org}; restore the IAM inventory"))
    })?;
    if kind.is_some_and(|expected| expected != *actual_kind) {
        return Err(StoreError::Invalid(format!("identity kind mismatch for {actor} in organization {org}")));
    }
    Ok(new)
}

fn add_mapping(map: &mut ActorMap, org: &str, old: &str, new: &str, kind: IdentityKind) -> StoreResult<()> {
    if let Some(previous) = map.insert((org.to_owned(), old.to_owned()), (new.to_owned(), kind))
        && previous != (new.to_owned(), kind)
    {
        return Err(StoreError::Invalid(format!("ambiguous identity mapping for {old} in organization {org}")));
    }
    Ok(())
}

fn stored_kind(value: &str) -> StoreResult<IdentityKind> {
    match value {
        "carbon" | "\"carbon\"" => Ok(IdentityKind::Carbon),
        "silicon" | "\"silicon\"" => Ok(IdentityKind::Silicon),
        _ => Err(corrupt("identity", "unknown actor kind")),
    }
}

const REFERENCES: &[(&str, &str, &str)] = &[
    ("profiles", "owner_id", "profiles.org_id"),
    ("sessions", "started_by", "sessions.org_id"),
    ("session_participants", "actor_id", "(SELECT org_id FROM sessions WHERE id = session_participants.session_id)"),
    ("commands", "actor_id", "(SELECT org_id FROM sessions WHERE id = commands.session_id)"),
    ("command_reports", "actor_id", "(SELECT org_id FROM sessions WHERE id = command_reports.session_id)"),
    ("recordings", "owner_id", "(SELECT org_id FROM sessions WHERE id = recordings.session_id)"),
    ("discovery_log", "actor_id", "discovery_log.org_id"),
    ("delivery_credentials", "actor_id", "delivery_credentials.org_id"),
    ("identity_projection", "public_id", "identity_projection.org_id"),
    ("identity_projection", "principal_id", "identity_projection.org_id"),
    ("command_reports", "principal_id", "(SELECT org_id FROM sessions WHERE id = command_reports.session_id)"),
];

impl Store {
    /// This runs before listeners/workers. A fresh empty store needs no cutover.
    pub async fn ensure_public_identifiers_migrated(&self, scope_key: &str) -> StoreResult<()> {
        validate_scope(scope_key)?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        if let Some(stored) = sqlx::query_scalar::<_, String>("SELECT scope_key FROM public_identifier_schema")
            .fetch_optional(&mut *tx)
            .await?
            && stored != scope_key
        {
            return Err(StoreError::Invalid("database belongs to a different IAM world".into()));
        }
        for (table, column, _) in REFERENCES {
            let ids: Vec<String> =
                sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT DISTINCT {column} FROM {table}")))
                    .fetch_all(&mut *tx)
                    .await?;
            for id in ids {
                silicon_browser_shared::actor_id(&id, "stored identity").map_err(|_| {
                    StoreError::Invalid("legacy identity references remain; stop writers and run --migrate-public-identifiers with this world's IAM mapping".into())
                })?;
            }
        }
        let access: Vec<String> = sqlx::query_scalar("SELECT access_json FROM profiles").fetch_all(&mut *tx).await?;
        for value in access {
            let entries: Vec<String> = serde_json::from_str(&value).map_err(corrupt_json)?;
            for entry in entries {
                if let Some(id) = entry.strip_prefix('@') {
                    silicon_browser_shared::actor_id(id, "stored ACL identity").map_err(|_| {
                        StoreError::Invalid("legacy ACL references remain; run --migrate-public-identifiers".into())
                    })?;
                }
            }
        }
        for row in sqlx::query("SELECT org_id, actor_id, principal_id, membership_id FROM delivery_credentials UNION ALL SELECT org_id, started_by, delivery_principal_id, delivery_membership_id FROM sessions WHERE delivery_principal_id IS NOT NULL OR delivery_membership_id IS NOT NULL")
            .fetch_all(&mut *tx).await?
        {
            let actor: &str = row.try_get("actor_id")?;
            let principal: &str = row.try_get("principal_id")?;
            let membership: &str = row.try_get("membership_id")?;
            let org: &str = row.try_get("org_id")?;
            if principal != actor || membership != format!("{actor}[{org}]") {
                return Err(StoreError::Invalid("legacy IAM authority references remain; run --migrate-public-identifiers".into()));
            }
        }
        for row in sqlx::query("SELECT started_by AS actor, started_by_kind AS kind FROM sessions UNION ALL SELECT public_id, kind FROM identity_projection UNION ALL SELECT actor_id, actor_kind FROM delivery_credentials")
            .fetch_all(&mut *tx).await?
        {
            if silicon_browser_shared::actor_id(row.try_get("actor")?, "stored identity").map_err(invalid)?
                != stored_kind(row.try_get("kind")?)?
            {
                return Err(StoreError::Invalid("stored actor kind does not match its public identity".into()));
            }
        }
        let inconsistent: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM identity_projection WHERE principal_id <> public_id UNION ALL SELECT 1 FROM command_reports WHERE principal_id <> actor_id)")
            .fetch_one(&mut *tx).await?;
        if inconsistent {
            return Err(StoreError::Invalid(
                "stored IAM principal binding is inconsistent; restore the verified inventory".into(),
            ));
        }
        sqlx::query("INSERT OR IGNORE INTO public_identifier_schema (singleton, scope_key) VALUES (1, ?)")
            .bind(scope_key)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn migrate_public_identifiers(
        &self,
        scope_key: &str,
        mapping: &[PublicIdentifierMapping],
        secrets: &SecretBox,
    ) -> StoreResult<()> {
        validate_scope(scope_key)?;
        let mut mapping = mapping.to_vec();
        mapping.sort_by(|a, b| (&a.org_id, &a.old_id).cmp(&(&b.org_id, &b.old_id)));
        let mapping_json = serde_json::to_string(&mapping).map_err(corrupt_json)?;
        let mut map = ActorMap::new();
        let mut targets = HashMap::new();
        let mut identities = HashMap::new();
        let mut sources = std::collections::HashSet::new();
        for entry in &mapping {
            if entry.scope_key != scope_key {
                return Err(StoreError::Invalid("mapping belongs to a different IAM world".into()));
            }
            safe_id(&entry.org_id, "mapping org_id")?;
            let owning_org = entry.owning_org_id.as_deref().unwrap_or(&entry.org_id);
            safe_id(owning_org, "mapping owning_org_id")?;
            safe_id(&entry.old_id, "mapping old_id")?;
            let kind = silicon_browser_shared::actor_id(&entry.new_id, "mapping new_id").map_err(invalid)?;
            if kind != entry.kind || !sources.insert((&entry.org_id, &entry.old_id)) {
                return Err(StoreError::Invalid("duplicate mapping or actor kind mismatch".into()));
            }
            if entry.old_id != entry.new_id {
                match kind {
                    IdentityKind::Carbon => {
                        silicon_browser_shared::actor_id(&format!("c:{}", entry.old_id), "mapping old_id")
                            .map_err(invalid)?;
                    }
                    IdentityKind::Silicon => {
                        let handle = entry.old_id.strip_suffix(&format!(":{owning_org}")).ok_or_else(|| {
                            StoreError::Invalid("legacy Silicon mapping has inconsistent ownership".into())
                        })?;
                        silicon_browser_shared::actor_id(&format!("si:{handle}"), "mapping old_id").map_err(invalid)?;
                    }
                }
            }
            if let Some((old, owner)) = targets.insert(&entry.new_id, (&entry.old_id, owning_org))
                && (old != &entry.old_id || (kind == IdentityKind::Silicon && owner != owning_org))
            {
                return Err(StoreError::Invalid(format!(
                    "identity collision or inconsistent ownership at {}",
                    entry.new_id
                )));
            }
            if let Some((new, owner)) = identities.insert(&entry.old_id, (&entry.new_id, owning_org))
                && (new != &entry.new_id || (kind == IdentityKind::Silicon && owner != owning_org))
            {
                return Err(StoreError::Invalid("one IAM actor has conflicting mappings across organizations".into()));
            }
            add_mapping(&mut map, &entry.org_id, &entry.old_id, &entry.new_id, kind)?;
            add_mapping(&mut map, &entry.org_id, &entry.new_id, &entry.new_id, kind)?;
        }
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        if let Some(row) =
            sqlx::query("SELECT scope_key, mapping_json FROM public_identifier_schema").fetch_optional(&mut *tx).await?
            && (row.try_get::<&str, _>("scope_key")? != scope_key
                || row.try_get::<Option<&str>, _>("mapping_json")?.is_some_and(|stored| stored != mapping_json))
        {
            return Err(StoreError::Invalid("database already migrated with a different IAM world or mapping".into()));
        }
        let pending: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM delivery_credentials WHERE operation IS NOT NULL)")
                .fetch_one(&mut *tx)
                .await?;
        if pending {
            return Err(StoreError::Invalid(
                "reconcile unfinished IAM delivery credential operations before migration".into(),
            ));
        }
        let uncertain: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recording_artifacts WHERE state = 'uploading' OR last_error = 'briefcase_upload_unconfirmed')")
            .fetch_one(&mut *tx).await?;
        if uncertain {
            return Err(StoreError::Invalid("reconcile uncertain Briefcase uploads before migration".into()));
        }
        // Earlier OAT-only data used IAM UUID references. Only verified
        // projections may resolve these into IAM 4's text identity keys.
        let projections = sqlx::query("SELECT * FROM identity_projection").fetch_all(&mut *tx).await?;
        let mut principals = HashMap::new();
        let mut principal_actors = HashMap::new();
        for row in &projections {
            let org: &str = row.try_get("org_id")?;
            let principal: &str = row.try_get("principal_id")?;
            let kind = stored_kind(row.try_get("kind")?)?;
            let new = mapped(&map, org, row.try_get("public_id")?, Some(kind))?.to_owned();
            if !Uuid::parse_str(principal).is_ok_and(|id| !id.is_nil())
                && mapped(&map, org, principal, Some(kind))? != new
            {
                return Err(StoreError::Invalid(
                    "projection principal does not match its mapped public identity".into(),
                ));
            }
            if let Some(previous) = principals.insert(new.clone(), principal.to_owned())
                && previous != principal
            {
                return Err(StoreError::Invalid("conflicting local principal bindings".into()));
            }
            if let Some(previous) = principal_actors.insert(principal.to_owned(), new.clone())
                && previous != new
            {
                return Err(StoreError::Invalid("one IAM principal is bound to different public actors".into()));
            }
            add_mapping(&mut map, org, principal, &new, kind)?;
        }
        for row in sqlx::query("SELECT org_id, started_by, started_by_kind FROM sessions").fetch_all(&mut *tx).await? {
            mapped(
                &map,
                row.try_get("org_id")?,
                row.try_get("started_by")?,
                Some(stored_kind(row.try_get("started_by_kind")?)?),
            )?;
        }
        for row in sqlx::query(
            "SELECT c.actor_id, c.principal_id, s.org_id FROM command_reports c JOIN sessions s ON s.id = c.session_id",
        )
        .fetch_all(&mut *tx)
        .await?
        {
            let org: &str = row.try_get("org_id")?;
            if mapped(&map, org, row.try_get("actor_id")?, None)?
                != mapped(&map, org, row.try_get("principal_id")?, None)?
            {
                return Err(StoreError::Invalid("command receipt principal does not match its actor".into()));
            }
        }
        for row in sqlx::query(
            "SELECT * FROM sessions WHERE delivery_principal_id IS NOT NULL OR delivery_membership_id IS NOT NULL",
        )
        .fetch_all(&mut *tx)
        .await?
        {
            let org: &str = row.try_get("org_id")?;
            let new = mapped(&map, org, row.try_get("started_by")?, None)?;
            if mapped(&map, org, row.try_get("delivery_principal_id")?, None)? != new {
                return Err(StoreError::Invalid("session delivery principal does not match its actor".into()));
            }
            let membership =
                canonical_membership(org, row.try_get("started_by")?, new, row.try_get("delivery_membership_id")?)?;
            sqlx::query("UPDATE sessions SET delivery_principal_id = ?, delivery_membership_id = ? WHERE id = ?")
                .bind(new)
                .bind(membership)
                .bind(row.try_get::<&str, _>("id")?)
                .execute(&mut *tx)
                .await?;
        }
        for row in sqlx::query("SELECT rowid, org_id, access_json FROM profiles").fetch_all(&mut *tx).await? {
            let org: &str = row.try_get("org_id")?;
            let mut access: Vec<String> = serde_json::from_str(row.try_get("access_json")?).map_err(corrupt_json)?;
            for entry in &mut access {
                if let Some(actor) = entry.strip_prefix('@') {
                    *entry = format!("@{}", mapped(&map, org, actor, None)?);
                }
            }
            sqlx::query("UPDATE profiles SET access_json = ? WHERE rowid = ?")
                .bind(serde_json::to_string(&access).map_err(corrupt_json)?)
                .bind(row.try_get::<i64, _>("rowid")?)
                .execute(&mut *tx)
                .await?;
        }
        migrate_encryption(&mut tx, &map, secrets).await?;
        for (table, column, org) in REFERENCES {
            // Identifiers are exclusively the constant audited REFERENCES above.
            let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT rowid AS migration_rowid, {org} AS org_id, {column} AS actor FROM {table}"
            )))
            .fetch_all(&mut *tx)
            .await?;
            for row in rows {
                let old: &str = row.try_get("actor")?;
                let new = mapped(&map, row.try_get("org_id")?, old, None)?;
                if new != old {
                    sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE {table} SET {column} = ? WHERE rowid = ?")))
                        .bind(new)
                        .bind(row.try_get::<i64, _>("migration_rowid")?)
                        .execute(&mut *tx)
                        .await?;
                }
            }
        }
        sqlx::query("INSERT INTO public_identifier_schema (singleton, scope_key, mapping_json) VALUES (1, ?, ?) ON CONFLICT(singleton) DO UPDATE SET mapping_json = excluded.mapping_json")
            .bind(scope_key).bind(mapping_json).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}

async fn migrate_encryption(tx: &mut Transaction<'_, Sqlite>, map: &ActorMap, secrets: &SecretBox) -> StoreResult<()> {
    for row in sqlx::query("SELECT c.*, s.org_id, EXISTS(SELECT 1 FROM recording_artifacts a WHERE a.session_id = c.session_id AND a.kind = 'commands') AS frozen FROM commands c JOIN sessions s ON s.id = c.session_id")
        .fetch_all(&mut **tx).await?
    {
        let org: &str = row.try_get("org_id")?;
        let session: &str = row.try_get("session_id")?;
        let sequence: i64 = row.try_get("sequence")?;
        let old: &str = row.try_get("actor_id")?;
        let new = mapped(map, org, old, None)?;
        let value = secrets.open_for(&command_secret_context(org, session, sequence, old), row.try_get("command_enc")?)
            .map_err(StoreError::Crypto)?;
        if old != new {
            let encrypted = secrets.seal_for(&command_secret_context(org, session, sequence, new), &value).map_err(StoreError::Crypto)?;
            sqlx::query("UPDATE commands SET command_enc = ?, delivery_actor_id = CASE WHEN ? THEN COALESCE(delivery_actor_id, actor_id) ELSE delivery_actor_id END WHERE session_id = ? AND sequence = ?")
                .bind(encrypted).bind(row.try_get::<bool, _>("frozen")?).bind(session).bind(sequence).execute(&mut **tx).await?;
        }
    }
    for row in sqlx::query("SELECT * FROM delivery_credentials").fetch_all(&mut **tx).await? {
        let org: &str = row.try_get("org_id")?;
        let old: &str = row.try_get("actor_id")?;
        let new = mapped(map, org, old, Some(stored_kind(row.try_get("actor_kind")?)?))?;
        let principal: &str = row.try_get("principal_id")?;
        if mapped(map, org, principal, None)? != new {
            return Err(StoreError::Invalid("credential principal does not match its verified projection".into()));
        }
        let membership: &str = row.try_get("membership_id")?;
        let new_membership = canonical_membership(org, old, new, membership)?;
        let id: &str = row.try_get("id")?;
        let old_context = format!("delivery/{org}/{old}/{principal}/{membership}/{id}/credentials");
        let value = secrets.open_for(&old_context, row.try_get("encrypted_payload")?).map_err(StoreError::Crypto)?;
        if old != new || principal != new || membership != new_membership {
            let context = format!("delivery/{org}/{new}/{new}/{new_membership}/{id}/credentials");
            let encrypted = secrets.seal_for(&context, &value).map_err(StoreError::Crypto)?;
            sqlx::query("UPDATE delivery_credentials SET encrypted_payload = ?, principal_id = ?, membership_id = ? WHERE id = ?")
                .bind(encrypted).bind(new).bind(new_membership).bind(id).execute(&mut **tx).await?;
        }
    }
    Ok(())
}

fn canonical_membership(org: &str, old: &str, new: &str, membership: &str) -> StoreResult<String> {
    let canonical = format!("{new}[{org}]");
    if membership != canonical
        && membership != format!("{old}[{org}]")
        && !Uuid::parse_str(membership).is_ok_and(|id| !id.is_nil())
    {
        return Err(StoreError::Invalid("membership does not match the verified local actor and organization".into()));
    }
    Ok(canonical)
}

fn validate_scope(scope: &str) -> StoreResult<()> {
    if !scope.is_empty() && !Uuid::parse_str(scope).is_ok_and(|id| !id.is_nil() && id.to_string() == scope) {
        return Err(StoreError::Invalid(
            "scope_key must be empty for production or a canonical IAM testing UUID".into(),
        ));
    }
    Ok(())
}
