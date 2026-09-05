//! Durable, fenced delivery of independent recording artifacts.
//! Network work stays outside SQLite transactions. A retry uses the same
//! filename and digest and can add an identical Briefcase version after a lost
//! response; completed receipts are never uploaded again.

use super::*;
use crate::providers::BriefcaseEntry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingArtifactKind {
    Video,
    Commands,
}

impl RecordingArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Commands => "commands",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RecordingDeliveryClaim {
    pub session_id: String,
    pub org_id: String,
    pub actor_id: String,
    pub principal_id: Option<String>,
    pub membership_id: Option<String>,
    pub provider_session_id: Option<String>,
    pub kind: RecordingArtifactKind,
    pub lease_id: String,
    pub attempt: u32,
}

impl Store {
    pub async fn bind_session_delivery_owner(
        &self,
        org: &str,
        session: &str,
        principal: &str,
        membership: &str,
    ) -> StoreResult<()> {
        let changed = sqlx::query("UPDATE sessions SET delivery_principal_id = ?, delivery_membership_id = ? WHERE org_id = ? AND id = ? AND status = 'starting' AND delivery_principal_id IS NULL")
            .bind(principal).bind(membership).bind(org).bind(session).execute(&self.pool).await?.rows_affected();
        if changed != 1 {
            return Err(StoreError::Invalid("session delivery owner could not be bound".into()));
        }
        Ok(())
    }
    /// Materialize idempotent artifact intents and claim a bounded batch. Only
    /// terminal sessions are eligible, so their ordered command stream is final.
    pub async fn claim_recording_deliveries(
        &self,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
        limit: u32,
    ) -> StoreResult<Vec<RecordingDeliveryClaim>> {
        if !(1..=16).contains(&limit) || lease_until <= now {
            return Err(StoreError::Invalid("invalid recording delivery claim bounds".into()));
        }
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        // Preserve interrupted legacy logs during upgrade from backend execution.
        // New local controllers only send completed reports.
        sqlx::query("UPDATE commands SET ended_at = COALESCE((SELECT ended_at FROM sessions WHERE id = commands.session_id), ?), exit_code = 130 WHERE ended_at IS NULL AND EXISTS(SELECT 1 FROM sessions s WHERE s.id = commands.session_id AND s.status IN ('ended', 'expired', 'failed'))")
            .bind(timestamp(now)).execute(&mut *tx).await?;
        sqlx::query(
            "INSERT OR IGNORE INTO recording_artifacts (session_id, kind, next_attempt_at) \
             SELECT s.id, 'video', ? FROM sessions s JOIN recordings r ON r.session_id = s.id \
             JOIN outbox o ON o.id = 'recording.store:' || s.id \
             WHERE s.status IN ('ended', 'expired', 'failed') AND r.status IN ('pending', 'failed') AND o.done_at IS NULL",
        )
        .bind(timestamp(now)).execute(&mut *tx).await?;
        // Keep Silicon logs even when the provider conclusively has no video.
        sqlx::query(
            "INSERT OR IGNORE INTO recording_artifacts (session_id, kind, next_attempt_at) \
             SELECT s.id, 'commands', ? FROM sessions s JOIN recordings r ON r.session_id = s.id \
             WHERE s.status IN ('ended', 'expired', 'failed') AND s.started_by_kind = 'silicon' \
             AND r.status IN ('pending', 'failed')",
        )
        .bind(timestamp(now))
        .execute(&mut *tx)
        .await?;
        let lease = Uuid::now_v7().to_string();
        let rows = sqlx::query(
            "UPDATE recording_artifacts SET state = 'working', attempts = attempts + 1, lease_id = ?, lease_until = ? \
             WHERE (session_id, kind) IN (SELECT a.session_id, a.kind FROM recording_artifacts a \
               JOIN recordings r ON r.session_id = a.session_id \
               WHERE a.state IN ('pending', 'working', 'uploading') AND r.status <> 'trashed' \
               AND a.next_attempt_at <= ? AND (a.lease_until IS NULL OR a.lease_until <= ?) \
               ORDER BY a.next_attempt_at, a.session_id, a.kind LIMIT ?) \
             RETURNING session_id, kind, attempts",
        )
        .bind(&lease)
        .bind(timestamp(lease_until))
        .bind(timestamp(now))
        .bind(timestamp(now))
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
        let mut claims = Vec::with_capacity(rows.len());
        for row in rows {
            let session_id: String = row.try_get("session_id")?;
            let session = sqlx::query("SELECT org_id, started_by, provider_session_id, delivery_principal_id, delivery_membership_id FROM sessions WHERE id = ?")
                .bind(&session_id).fetch_one(&mut *tx).await?;
            claims.push(RecordingDeliveryClaim {
                session_id,
                org_id: session.try_get("org_id")?,
                actor_id: session.try_get("started_by")?,
                principal_id: session.try_get("delivery_principal_id")?,
                membership_id: session.try_get("delivery_membership_id")?,
                provider_session_id: session.try_get("provider_session_id")?,
                kind: match row.try_get::<&str, _>("kind")? {
                    "video" => RecordingArtifactKind::Video,
                    "commands" => RecordingArtifactKind::Commands,
                    _ => return Err(corrupt("recording artifact", "invalid kind")),
                },
                lease_id: lease.clone(),
                attempt: u32::try_from(row.try_get::<i64, _>("attempts")?)
                    .map_err(|_| corrupt("recording artifact", "invalid attempts"))?,
            });
        }
        tx.commit().await?;
        Ok(claims)
    }

    pub async fn recording_delivery_is_current(
        &self,
        claim: &RecordingDeliveryClaim,
        now: DateTime<Utc>,
    ) -> StoreResult<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM recording_artifacts a JOIN sessions s ON s.id = a.session_id \
             JOIN recordings r ON r.session_id = s.id WHERE a.session_id = ? AND a.kind = ? \
             AND a.lease_id = ? AND a.lease_until > ? AND a.state IN ('working', 'uploading') \
             AND s.org_id = ? AND s.started_by = ? AND r.status <> 'trashed')",
        )
        .bind(&claim.session_id)
        .bind(claim.kind.as_str())
        .bind(&claim.lease_id)
        .bind(timestamp(now))
        .bind(&claim.org_id)
        .bind(&claim.actor_id)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Persist exact bytes before issuing a proof or sending the upload. A
    /// regenerated source after an uncertain write must match the prior digest.
    pub async fn bind_recording_delivery(
        &self,
        claim: &RecordingDeliveryClaim,
        digest: &str,
        size: u64,
        now: DateTime<Utc>,
    ) -> StoreResult<bool> {
        if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
            return Err(StoreError::Invalid("invalid recording digest".into()));
        }
        let size = to_i64(size, "recording artifact size")?;
        let changed = sqlx::query(
            "UPDATE recording_artifacts SET body_sha256 = ?, size_bytes = ? \
             WHERE session_id = ? AND kind = ? AND lease_id = ? AND lease_until > ? AND state = 'working' \
             AND (body_sha256 IS NULL OR (body_sha256 = ? AND size_bytes = ?)) \
             AND EXISTS(SELECT 1 FROM sessions s JOIN recordings r ON r.session_id = s.id \
                 WHERE s.id = recording_artifacts.session_id AND s.org_id = ? AND s.started_by = ? AND r.status <> 'trashed')",
        ).bind(digest).bind(size).bind(&claim.session_id).bind(claim.kind.as_str()).bind(&claim.lease_id)
            .bind(timestamp(now)).bind(digest).bind(size).bind(&claim.org_id).bind(&claim.actor_id)
            .execute(&self.pool).await?.rows_affected();
        Ok(changed == 1)
    }

    pub async fn begin_recording_upload(
        &self,
        claim: &RecordingDeliveryClaim,
        now: DateTime<Utc>,
    ) -> StoreResult<bool> {
        Ok(sqlx::query(
            "UPDATE recording_artifacts SET state = 'uploading' \
             WHERE session_id = ? AND kind = ? AND lease_id = ? AND lease_until > ? AND state = 'working' \
             AND body_sha256 IS NOT NULL AND EXISTS(SELECT 1 FROM sessions s JOIN recordings r ON r.session_id = s.id \
                 WHERE s.id = recording_artifacts.session_id AND s.org_id = ? AND s.started_by = ? AND r.status <> 'trashed')",
        ).bind(&claim.session_id).bind(claim.kind.as_str()).bind(&claim.lease_id).bind(timestamp(now))
            .bind(&claim.org_id).bind(&claim.actor_id).execute(&self.pool).await?.rows_affected() == 1)
    }

    pub async fn complete_recording_delivery(
        &self,
        claim: &RecordingDeliveryClaim,
        entry: &BriefcaseEntry,
        secrets: &SecretBox,
        now: DateTime<Utc>,
    ) -> StoreResult<bool> {
        if entry.org_id != claim.org_id || entry.entry_type != "file" {
            return Err(StoreError::Invalid("recording receipt belongs to a different scope".into()));
        }
        let expected_name = match claim.kind {
            RecordingArtifactKind::Video => format!("{}.mp4", claim.session_id),
            RecordingArtifactKind::Commands => format!("{}-commands.jsonl", claim.session_id),
        };
        if entry.name != expected_name {
            return Err(StoreError::Invalid("recording receipt filename does not match".into()));
        }
        let link = secrets.seal_for(&artifact_context(claim), &entry.permanent_url).map_err(StoreError::Crypto)?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        // Preserve a successful receipt even if the user hid the recording while
        // it was in flight; never resurrect the hidden recording below.
        let changed = sqlx::query(
            "UPDATE recording_artifacts SET state = 'complete', entry_id = ?, artifact_path = ?, receipt_url_enc = ?, \
             completed_at = ?, lease_id = NULL, lease_until = NULL, last_error = NULL \
             WHERE session_id = ? AND kind = ? AND lease_id = ? AND state = 'uploading' AND size_bytes = ? \
             AND EXISTS(SELECT 1 FROM sessions s WHERE s.id = recording_artifacts.session_id AND s.org_id = ? AND s.started_by = ?)",
        ).bind(entry.id.to_string()).bind(&entry.path).bind(link).bind(timestamp(now)).bind(&claim.session_id)
            .bind(claim.kind.as_str()).bind(&claim.lease_id).bind(to_i64(entry.size, "recording size")?)
            .bind(&claim.org_id).bind(&claim.actor_id).execute(&mut *tx).await?.rows_affected();
        if changed == 0 {
            return Ok(false);
        }
        if claim.kind == RecordingArtifactKind::Video {
            let link = secrets
                .seal_for(
                    &session_secret_context(&claim.org_id, &claim.session_id, "briefcase-url"),
                    &entry.permanent_url,
                )
                .map_err(StoreError::Crypto)?;
            sqlx::query(
                "UPDATE recordings SET briefcase_url_enc = ?, artifact_path = ?, size_bytes = ? WHERE session_id = ?",
            )
            .bind(link)
            .bind(&entry.path)
            .bind(to_i64(entry.size, "recording size")?)
            .bind(&claim.session_id)
            .execute(&mut *tx)
            .await?;
        }
        refresh_recording_status(&mut tx, &claim.session_id, now).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Authentication waits do not exhaust the transfer retry budget. All
    /// diagnostic reasons are static internal codes, never upstream bodies.
    pub async fn defer_recording_delivery(
        &self,
        claim: &RecordingDeliveryClaim,
        next: DateTime<Utc>,
        reason: &'static str,
        count_attempt: bool,
    ) -> StoreResult<bool> {
        Ok(sqlx::query(
            "UPDATE recording_artifacts SET state = 'pending', lease_id = NULL, lease_until = NULL, next_attempt_at = ?, \
             last_error = ?, attempts = attempts - ? WHERE session_id = ? AND kind = ? AND lease_id = ? \
             AND state IN ('working', 'uploading')",
        ).bind(timestamp(next)).bind(reason).bind(i64::from(!count_attempt)).bind(&claim.session_id)
            .bind(claim.kind.as_str()).bind(&claim.lease_id).execute(&self.pool).await?.rows_affected() == 1)
    }

    pub async fn fail_recording_delivery(
        &self,
        claim: &RecordingDeliveryClaim,
        reason: &'static str,
        now: DateTime<Utc>,
    ) -> StoreResult<bool> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let changed = sqlx::query(
            "UPDATE recording_artifacts SET state = 'failed', last_error = ?, lease_id = NULL, lease_until = NULL \
             WHERE session_id = ? AND kind = ? AND lease_id = ? AND state IN ('working', 'uploading')",
        )
        .bind(reason)
        .bind(&claim.session_id)
        .bind(claim.kind.as_str())
        .bind(&claim.lease_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if changed == 1 {
            refresh_recording_status(&mut tx, &claim.session_id, now).await?;
        }
        tx.commit().await?;
        Ok(changed == 1)
    }

    pub async fn wake_recording_deliveries(&self, org: &str, actor: &str, now: DateTime<Utc>) -> StoreResult<()> {
        sqlx::query(
            "UPDATE recording_artifacts SET next_attempt_at = ? WHERE state = 'pending' AND session_id IN \
             (SELECT id FROM sessions WHERE org_id = ? AND started_by = ?)",
        )
        .bind(timestamp(now))
        .bind(org)
        .bind(actor)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Explicit owner retry after an outage or a raised size limit. Preserve
    /// proof-bound bytes and completed receipts; permanent source failures are
    /// never reset. Repeating an accepted request does not reset active work.
    pub async fn retry_failed_recording_delivery(
        &self,
        org: &str,
        session: &str,
        actor: &str,
        principal: &str,
        membership: &str,
        now: DateTime<Utc>,
    ) -> StoreResult<bool> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let owned: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sessions s JOIN recordings r ON r.session_id = s.id \
             WHERE s.org_id = ? AND s.id = ? AND s.started_by = ? AND s.delivery_principal_id = ? \
             AND s.delivery_membership_id = ? AND s.status IN ('ended', 'expired', 'failed') AND r.status <> 'trashed')",
        ).bind(org).bind(session).bind(actor).bind(principal).bind(membership).fetch_one(&mut *tx).await?;
        if !owned {
            return Ok(false);
        }
        let changed = sqlx::query(
            "UPDATE recording_artifacts SET state = 'pending', attempts = 0, next_attempt_at = ?, \
             lease_id = NULL, lease_until = NULL, last_error = NULL WHERE session_id = ? AND state = 'failed' \
             AND last_error IN ('recording_size_limit', 'recording_source_unavailable', 'recording_proof_unavailable', \
               'recording_proof_expired', 'briefcase_upload_unconfirmed', 'delivery_timeout', 'delivery_attempts_exhausted')",
        ).bind(timestamp(now)).bind(session).execute(&mut *tx).await?.rows_affected();
        let pending: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM recording_artifacts WHERE session_id = ? AND state IN ('pending', 'working', 'uploading'))",
        ).bind(session).fetch_one(&mut *tx).await?;
        if changed > 0 {
            sqlx::query("UPDATE recordings SET status = CASE WHEN EXISTS(SELECT 1 FROM recording_artifacts WHERE session_id = ? AND state = 'failed') THEN 'failed' ELSE 'pending' END WHERE session_id = ?")
                .bind(session).bind(session).execute(&mut *tx).await?;
            sqlx::query("UPDATE outbox SET done_at = NULL WHERE id = ?")
                .bind(format!("recording.store:{session}"))
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(changed > 0 || pending)
    }

    /// Read one bounded page; proof-bound logs include exact command strings,
    /// sequence, actor, timestamp and exit result without buffering all history.
    pub async fn recording_delivery_logs(
        &self,
        claim: &RecordingDeliveryClaim,
        after: u64,
        limit: u32,
        secrets: &SecretBox,
    ) -> StoreResult<Vec<SessionLog>> {
        if !(1..=128).contains(&limit) || claim.kind != RecordingArtifactKind::Commands {
            return Err(StoreError::Invalid("invalid command log page".into()));
        }
        let rows = sqlx::query(
            "SELECT c.* FROM commands c JOIN sessions s ON s.id = c.session_id \
             WHERE c.session_id = ? AND s.org_id = ? AND s.started_by = ? AND s.started_by_kind = 'silicon' \
             AND s.status IN ('ended', 'expired', 'failed') AND c.sequence > ? ORDER BY c.sequence LIMIT ?",
        )
        .bind(&claim.session_id)
        .bind(&claim.org_id)
        .bind(&claim.actor_id)
        .bind(to_i64(after, "log sequence")?)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let sequence: i64 = row.try_get("sequence")?;
                let actor: String = row.try_get("actor_id")?;
                let command = secrets
                    .open_for(
                        &command_secret_context(&claim.org_id, &claim.session_id, sequence, &actor),
                        row.try_get("command_enc")?,
                    )
                    .map_err(StoreError::Crypto)?;
                Ok(SessionLog {
                    sequence: nonnegative(sequence, "log sequence")?,
                    actor_id: actor,
                    command,
                    at: parse_timestamp(row.try_get("started_at")?, "command.started_at")?,
                    exit_code: row.try_get("exit_code")?,
                })
            })
            .collect()
    }
}

pub(super) fn artifact_context(claim: &RecordingDeliveryClaim) -> String {
    format!("recording-artifact:{}:{}:{}", claim.org_id, claim.session_id, claim.kind.as_str())
}

async fn refresh_recording_status(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    session: &str,
    now: DateTime<Utc>,
) -> StoreResult<()> {
    sqlx::query(
        "UPDATE recordings SET status = CASE \
           WHEN EXISTS(SELECT 1 FROM recording_artifacts WHERE session_id = ? AND state = 'failed') THEN 'failed' \
           WHEN EXISTS(SELECT 1 FROM recording_artifacts WHERE session_id = ? AND kind = 'video' AND state = 'complete') \
             AND NOT EXISTS(SELECT 1 FROM recording_artifacts WHERE session_id = ? AND state <> 'complete') THEN 'available' \
           ELSE status END WHERE session_id = ? AND status <> 'trashed'",
    ).bind(session).bind(session).bind(session).bind(session).execute(&mut **tx).await?;
    sqlx::query(
        "UPDATE outbox SET done_at = COALESCE(done_at, ?) WHERE id = ? AND \
         NOT EXISTS(SELECT 1 FROM recording_artifacts WHERE session_id = ? AND state IN ('pending', 'working', 'uploading'))",
    ).bind(timestamp(now)).bind(format!("recording.store:{session}")).bind(session).execute(&mut **tx).await?;
    Ok(())
}
