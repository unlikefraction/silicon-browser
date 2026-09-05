//! Idempotent cooperative telemetry from client-owned browser controllers.
//! A report cannot cause a subprocess or a provider request. Command content is
//! encrypted once. Browser output is neither accepted nor persisted.
use super::*;
use sha2::{Digest, Sha256};
use silicon_browser_shared::{CommandReport, CommandReportReceipt};

impl Store {
    /// Issue a direct capability only for an active browser and a current ACL.
    /// Historical participation is sufficient for history, not renewed access
    /// to a profile after its owner has removed the caller.
    pub async fn connection_runtime(
        &self,
        org: &str,
        actor: &Identity,
        session_id: &str,
        secrets: &SecretBox,
        now: DateTime<Utc>,
    ) -> StoreResult<(ProviderRuntime, DateTime<Utc>)> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT s.*, p.access_json, p.status AS profile_status FROM sessions s \
             LEFT JOIN profiles p ON p.id = s.profile_id AND p.org_id = s.org_id \
             WHERE s.org_id = ? AND s.id = ?",
        )
        .bind(org)
        .bind(session_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| not_found("session", session_id))?;
        let authorized = if row.try_get::<Option<&str>, _>("profile_id")?.is_some() {
            row.try_get::<Option<&str>, _>("profile_status")? == Some("active")
                && serde_json::from_str::<AccessList>(row.try_get("access_json")?).map_err(corrupt_json)?.allows(actor)
        } else {
            let mut participant = false;
            for id in actor.principal_ids() {
                participant |= sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM session_participants WHERE session_id = ? AND actor_id = ?)",
                )
                .bind(session_id)
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
            }
            participant
        };
        if !authorized {
            return Err(not_found("session", session_id));
        }
        let status: &str = row.try_get("status")?;
        let expires_at = parse_timestamp(row.try_get("expires_at")?, "session.expires_at")?;
        if status != "active" || expires_at <= now {
            return Err(StoreError::SessionState { session_id: session_id.into(), status: status.into() });
        }
        let runtime = runtime_from_row(&row, org, session_id, secrets)?;
        tx.commit().await?;
        Ok((runtime, expires_at))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn report_command(
        &self,
        org: &str,
        actor: &Identity,
        principal: &str,
        session_id: &str,
        report: &CommandReport,
        secrets: &SecretBox,
        now: DateTime<Utc>,
    ) -> StoreResult<CommandReportReceipt> {
        report.validate().map_err(invalid)?;
        safe_id(principal, "principal_id")?;
        let session = self.session(org, actor, session_id).await?;
        if report.started_at < session.started_at - ChronoDuration::minutes(5)
            || report.finished_at > now + ChronoDuration::minutes(5)
            || report.finished_at > session.expires_at + ChronoDuration::minutes(5)
        {
            return Err(StoreError::Invalid("command report timestamps are outside the session window".into()));
        }
        let body = serde_json::to_vec(report).map_err(corrupt_json)?;
        let digest = hex::encode(Sha256::digest(&body));
        // Reserve the SQLite write lock before reading the receipt/sequence.
        // Deferred read -> write upgrades can fail with SQLITE_BUSY_SNAPSHOT
        // under concurrent clients even when busy_timeout is configured.
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        if let Some(row) = sqlx::query(
            "SELECT principal_id, payload_sha256, sequence FROM command_reports WHERE session_id = ? AND command_id = ?",
        )
        .bind(session_id)
        .bind(report.command_id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        {
            if row.try_get::<&str, _>("principal_id")? != principal
                || row.try_get::<&str, _>("payload_sha256")? != digest
            {
                return Err(StoreError::ReportConflict { code: "command_report_conflict" });
            }
            let sequence = row
                .try_get::<Option<i64>, _>("sequence")?
                .map(|n| nonnegative(n, "command.sequence"))
                .transpose()?;
            tx.commit().await?;
            return Ok(CommandReportReceipt { command_id: report.command_id, sequence });
        }
        let row = sqlx::query(
            "SELECT s.status, s.started_by_kind, s.ended_at, r.status AS recording_status \
             FROM sessions s JOIN recordings r ON r.session_id = s.id WHERE s.org_id = ? AND s.id = ?",
        )
        .bind(org)
        .bind(session_id)
        .fetch_one(&mut *tx)
        .await?;
        if !matches!(row.try_get::<&str, _>("status")?, "active" | "ending" | "ended" | "expired" | "failed") {
            return Err(StoreError::Invalid("session cannot accept command reports".into()));
        }
        if let Some(ended) = row.try_get::<Option<&str>, _>("ended_at")?
            && report.finished_at > parse_timestamp(ended, "session.ended_at")? + ChronoDuration::minutes(5)
        {
            return Err(StoreError::Invalid("command report finished after the session window".into()));
        }
        let silicon = row.try_get::<&str, _>("started_by_kind")? == "silicon";
        // Intent creation and claim commit before the archive reads its first
        // page. Closing only once body_sha256 exists would allow an accepted
        // report to race between the final page read and that digest binding.
        let frozen: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM recording_artifacts WHERE session_id = ? AND kind = 'commands')",
        )
        .bind(session_id)
        .fetch_one(&mut *tx)
        .await?;
        if silicon && (frozen || row.try_get::<&str, _>("recording_status")? == "trashed") {
            return Err(StoreError::ReportConflict { code: "report_window_closed" });
        }
        let sequence = if silicon {
            let sequence: i64 =
                sqlx::query_scalar("SELECT COALESCE(MAX(sequence), 0) + 1 FROM commands WHERE session_id = ?")
                    .bind(session_id)
                    .fetch_one(&mut *tx)
                    .await?;
            let command = if report.flags.is_empty() {
                report.command.clone()
            } else {
                format!("{} {}", report.command, shell_words::join(&report.flags))
            };
            let encrypted = secrets
                .seal_for(&command_secret_context(org, session_id, sequence, &actor.id), &command)
                .map_err(StoreError::Crypto)?;
            sqlx::query(
                "INSERT INTO commands(session_id, sequence, actor_id, command_enc, started_at, ended_at, exit_code) \
                 VALUES(?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(session_id)
            .bind(sequence)
            .bind(&actor.id)
            .bind(encrypted)
            .bind(timestamp(report.started_at))
            .bind(timestamp(report.finished_at))
            .bind(report.exit_code)
            .execute(&mut *tx)
            .await?;
            Some(sequence)
        } else {
            None
        };
        sqlx::query(
            "INSERT INTO command_reports(session_id, command_id, actor_id, principal_id, payload_sha256, sequence, received_at) \
             VALUES(?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session_id)
        .bind(report.command_id.to_string())
        .bind(&actor.id)
        .bind(principal)
        .bind(digest)
        .bind(sequence)
        .bind(timestamp(now))
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT OR IGNORE INTO session_participants(session_id, actor_id, role, first_seen_at) VALUES(?, ?, 'runner', ?)",
        )
        .bind(session_id)
        .bind(&actor.id)
        .bind(timestamp(now))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(CommandReportReceipt { command_id: report.command_id, sequence: sequence.map(|n| n as u64) })
    }
}
