//! Explicit user-submitted bug reports.
use crate::{Client, IdempotencyKey, Result, client::json_body};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A user-authored report. No files, credentials, or diagnostics are attached automatically.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BugReport {
    /// Reproduction steps and observed/expected behavior, at most 16 KiB.
    pub message: String,
    /// Optional HTTPS pull request URL in the Briefcase repository.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,
    /// Optional internal Silicon attribution, at most 256 bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isi: Option<String>,
}

/// Durable acknowledgement of a submitted report.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReportReceipt {
    /// Stable report identifier returned on retries.
    pub id: Uuid,
    /// True once the report has been stored.
    pub accepted: bool,
}

impl Client {
    /// Submits a bug report in this client's organization and environment.
    ///
    /// # Errors
    /// Returns authentication, validation, or transport errors. Use
    /// [`Self::submit_report_with_key`] when a lost response must be retried.
    pub async fn submit_report(&self, report: &BugReport) -> Result<ReportReceipt> {
        self.submit_report_with_key(report, &IdempotencyKey::random())
            .await
    }

    /// Submits a report with a caller-persisted retry identity.
    ///
    /// # Errors
    /// Returns a conflict if this identity was used with another report.
    pub async fn submit_report_with_key(
        &self,
        report: &BugReport,
        key: &IdempotencyKey,
    ) -> Result<ReportReceipt> {
        let request = self
            .request(Method::POST, self.api_url(&["reports"])?)
            .header("content-type", "application/json")
            .header("idempotency-key", key.as_str())
            .body(json_body(report)?)
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }
}
