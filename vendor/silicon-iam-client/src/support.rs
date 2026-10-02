//! Public project discovery and explicit bug reporting, independent of IAM sessions.

use std::{
    io::Write as _,
    process::{Command, Stdio},
};

/// Canonical source repository and bug tracker.
pub const REPOSITORY: &str = "https://github.com/teamofsilicons/silicon-iam";
/// Public instructive and reference documentation.
pub const DOCUMENTATION: &str = "https://docs.iam.teamofsilicons.com";
/// Published Rust package.
pub const PACKAGE: &str = "https://crates.io/crates/silicon-iam-client";

/// Submit a user-authored report through an already authenticated GitHub CLI.
///
/// Only the supplied message, optional PR and compiled package version are sent.
/// No IAM credentials, local logs, environment or ISI are collected. This is an
/// explicit side effect: call only when the user asks to submit a report.
///
/// # Errors
/// Returns an actionable error for empty input, an invalid PR, missing `gh`,
/// missing GitHub authentication, or failed issue creation. Never retries.
pub fn report(message: &str, pr: Option<&str>) -> Result<String, String> {
    let body = report_body(message, pr)?;
    let title: String = message
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Bug report")
        .chars()
        .take(100)
        .collect();
    let mut child = Command::new("gh")
        .args(["issue", "create", "--repo", REPOSITORY, "--title", &title, "--body-file", "-"])
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e| format!("Cannot start GitHub CLI: {e}. Install gh and run `gh auth login`, or report at {REPOSITORY}/issues/new."))?;
    let written = child
        .stdin
        .take()
        .ok_or("GitHub CLI stdin unavailable")?
        .write_all(body.as_bytes());
    let output = child.wait_with_output().map_err(|e| {
        format!("Cannot read GitHub result: {e}; check {REPOSITORY}/issues before retrying.")
    })?;
    if !output.status.success() {
        return Err(format!(
            "GitHub report failed: {}. Run `gh auth status`; check {REPOSITORY}/issues before retrying.",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    written.map_err(|e| {
        format!("Could not finish writing report: {e}; check {REPOSITORY}/issues before retrying.")
    })?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Prepare precisely the content that [`report`] submits.
///
/// # Errors
/// Rejects empty reports and PR references outside this project's pull requests.
pub fn report_body(message: &str, pr: Option<&str>) -> Result<String, String> {
    if message.trim().is_empty() {
        return Err(
            "Report message cannot be empty; include reproduction steps and expected behavior."
                .into(),
        );
    }
    if let Some(pr) = pr {
        let valid = pr
            .strip_prefix(&format!("{REPOSITORY}/pull/"))
            .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()));
        if !valid {
            return Err(format!(
                "--pr must be a pull request URL like {REPOSITORY}/pull/123"
            ));
        }
    }
    Ok(format!(
        "{message}\n\n{}\n\nSubmitted with silicon-iam-client {}.\n",
        pr.map_or_else(String::new, |pr| format!("Proposed fix: {pr}")),
        env!("CARGO_PKG_VERSION")
    ))
}

#[cfg(test)]
mod tests {
    use super::report_body;
    #[test]
    fn reports_preserve_user_text_and_validate_prs() {
        let body = report_body(
            "Login failed\nSteps: ...",
            Some("https://github.com/teamofsilicons/silicon-iam/pull/42"),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(body.starts_with("Login failed\nSteps: ..."));
        assert!(body.contains("Proposed fix:"));
        assert!(report_body("  ", None).is_err());
        assert!(report_body("bug", Some("https://example.com/pull/42")).is_err());
        assert!(
            report_body(
                "bug",
                Some("https://github.com/teamofsilicons/silicon-iam/pull/42?token=secret")
            )
            .is_err()
        );
    }
}
