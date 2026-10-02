//! Bounded operational events. No credentials, paths, user text or file data.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Component that observed an event. Remote reports are labeled unverified.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Briefcase API process.
    Backend,
    /// Background job process.
    Worker,
    /// Rust library caller.
    Sdk,
    /// Command-line interface.
    Cli,
    /// Persistent local background service.
    Daemon,
    /// Browser application.
    Web,
}

/// A stage in an operation, suitable for correlating progress and completion.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Operation began.
    Started,
    /// Intermediate work.
    Progress,
    /// Operation succeeded.
    Completed,
    /// Operation failed.
    Failed,
    /// Operation was cancelled.
    Cancelled,
}

/// One self-contained, content-free operational event.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    /// Unique identity for this observation.
    pub id: Uuid,
    /// Reporting component.
    pub source: Source,
    /// Bounded machine-readable operation name, never an argument or filename.
    pub operation: String,
    /// Operation lifecycle stage.
    pub stage: Stage,
    /// Whether the observation belongs to a sandbox.
    #[serde(default)]
    pub testing: bool,
    /// Public sandbox ID when known; never the application secret.
    pub environment_id: Option<Uuid>,
    /// Correlation with the API response request ID.
    pub request_id: Option<Uuid>,
    /// Time spent on this step.
    pub duration_ms: Option<u64>,
    /// HTTP result, when applicable.
    pub status: Option<u16>,
    /// Retry number or stage number.
    pub attempt: Option<u32>,
    /// Count or transferred byte count, without object names.
    pub count: Option<u64>,
    /// Progress expressed as 0–100.
    pub progress: Option<u8>,
}

impl Event {
    /// Starts a bounded event with a fresh identity.
    #[must_use]
    pub fn new(source: Source, operation: impl Into<String>, stage: Stage) -> Self {
        Self {
            id: Uuid::new_v4(),
            source,
            operation: operation.into(),
            stage,
            testing: false,
            environment_id: None,
            request_id: None,
            duration_ms: None,
            status: None,
            attempt: None,
            count: None,
            progress: None,
        }
    }

    /// Validates the fixed vocabulary boundary before accepting an event.
    #[must_use]
    pub fn valid(&self) -> bool {
        !self.id.is_nil()
            && self.operation.len() <= 96
            && !self.operation.is_empty()
            && self
                .operation
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-'))
            && ![
                "ask_", "slt_", "oat_", "ort_", "table-", "apikey-", "sscli-", "stk-",
            ]
            .iter()
            .any(|prefix| self.operation.contains(prefix))
            && self
                .environment_id
                .is_none_or(|id| !id.is_nil() && self.testing)
            && self.request_id.is_none_or(|id| !id.is_nil())
            && self
                .status
                .is_none_or(|status| (100..=599).contains(&status))
            && self.progress.is_none_or(|value| value <= 100)
    }
}

/// Sends an explicit diagnostic through Briefcase; table keys stay on its server.
/// Failures are intentionally independent of the operation being diagnosed.
/// # Errors
/// Returns validation or delivery errors. Callers should treat these as best effort.
pub async fn submit(base: &str, event: &Event) -> crate::Result<()> {
    if !event.valid() {
        return Err(crate::Error::Configuration(
            "invalid telemetry event".into(),
        ));
    }
    let config = crate::Config::for_sign_in(base)?
        .with_auto_update(false)
        .with_telemetry(false);
    let client = crate::Client::new_unchecked(config)?;
    client
        .receive_empty(
            client
                .anonymous_request(reqwest::Method::POST, client.api_url(&["telemetry"])?)
                .json(event)
                .timeout(std::time::Duration::from_secs(2)),
        )
        .await
}

/// Reads the conventional opt-out, defaulting to enabled when unset.
#[must_use]
pub fn enabled_from_env() -> bool {
    std::env::var("BRIEFCASE_TELEMETRY")
        .ok()
        .is_none_or(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "off" | "false" | "no"
            )
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[test]
    fn events_refuse_credentials_arbitrary_payloads_and_cross_plane_ids() {
        let mut event = Event::new(Source::Cli, "upload", Stage::Completed);
        assert!(event.valid());
        for value in [
            "ask_secret",
            "table-secret",
            "private/alice.txt",
            "a query",
            "",
            "sscli-session",
        ] {
            event.operation = value.into();
            assert!(!event.valid());
        }
        event.operation = "upload".into();
        event.environment_id = Some(Uuid::new_v4());
        assert!(!event.valid());
        event.testing = true;
        assert!(event.valid());
        let mut value = serde_json::to_value(event).unwrap();
        value["token"] = "secret".into();
        assert!(serde_json::from_value::<Event>(value).is_err());
    }
}
