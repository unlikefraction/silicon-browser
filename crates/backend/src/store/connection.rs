//! Direct browser capabilities, bounded by session lifetime and current access.
use super::*;

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
}
