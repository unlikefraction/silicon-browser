//! Handoff to Honeycomb's authenticated environment-management client.
//!
//! File commands keep using `Client` and the IAM-issued test app secret. Shared
//! lifecycle commands use Honeycomb's own saved session, authority and retry log.
use crate::{Error, Result};

/// Runs an official `honeycomb environments ...` command with inherited terminal I/O.
/// The package retains no Honeycomb credentials or state. Install Honeycomb first.
/// # Errors
/// Reports a missing executable or a failed Honeycomb operation without exposing secrets.
pub fn manage_environment(arguments: &[String]) -> Result<()> {
    let status=std::process::Command::new("honeycomb")
        .arg("environments").args(arguments)
        .env_remove("BRIEFCASE_TOKEN").env_remove("BRIEFCASE_APP_SECRET")
        .env_remove("BRIEFCASE_IAM_APP_SECRET").env_remove("BRIEFCASE_IAM_TEST_KEY")
        .env_remove("BRIEFCASE_IAM_ENVIRONMENT_KEY")
        .status().map_err(|source|Error::Io{path:"honeycomb (install it from https://docs.honeycomb.teamofsilicons.com/installation/)".to_owned(),source})?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Configuration(format!(
            "Honeycomb environment command failed ({status}); follow the Honeycomb error above and reuse its operation ID when retrying"
        )))
    }
}
