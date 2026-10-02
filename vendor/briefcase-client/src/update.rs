//! Explicit dependency maintenance helpers. API requests never run updates.
//! CLI installation and updates are managed by Honeycomb.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

pub use semver::Version;
use serde::Deserialize;

/// Crates.io package containing the Rust client.
pub const CLIENT_CRATE: &str = "briefcase-client";
/// Version compiled into this process.
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const CRATES_IO: &str = "https://crates.io/api/v1/crates";
const CHECK_TIMEOUT: Duration = Duration::from_secs(3);

/// Result of the most recent package update attempt for a Cargo project.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum UpdateStatus {
    /// No request has triggered the check yet.
    #[default]
    NotChecked,
    /// Automatic maintenance was explicitly disabled.
    Disabled,
    /// No consuming Cargo project could be found.
    NoCargoProject,
    /// The lockfile already resolves the newest stable release.
    Current {
        /// Current stable version.
        version: Version,
    },
    /// The lockfile was advanced; the next build loads the new version.
    Updated {
        /// Version running now.
        from: Version,
        /// Version selected for the next build.
        to: Version,
    },
    /// Maintenance failed, without changing the API request result.
    Failed {
        /// Human-readable non-secret failure.
        reason: String,
    },
}

/// A stable crates.io comparison.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Release {
    /// Version currently running.
    pub current: Version,
    /// Latest stable published version.
    pub latest: Version,
}

impl Release {
    /// Whether a newer stable release exists.
    #[must_use]
    pub fn update_available(&self) -> bool {
        self.latest > self.current
    }
}

/// A registry lookup or Cargo operation failed.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// CLI binaries are installed and updated through Honeycomb.
    #[error("Honeycomb manages Briefcase CLI updates; run honeycomb update 'briefcase'")]
    HoneycombManaged,
    /// The registry URL was invalid.
    #[error("invalid crates.io URL: {0}")]
    Url(#[from] url::ParseError),
    /// A version was not semantic versioning.
    #[error("invalid package version: {0}")]
    Version(#[from] semver::Error),
    /// crates.io could not be queried.
    #[error("cannot query crates.io: {0}")]
    Registry(#[from] reqwest::Error),
    /// No stable release was present.
    #[error("crates.io returned no stable version for {0}")]
    MissingStableVersion(String),
    /// A manifest has no containing directory.
    #[error("Cargo manifest {0} has no parent directory")]
    InvalidManifest(PathBuf),
    /// Cargo could not start.
    #[error("cannot run Cargo: {0}")]
    CargoIo(#[from] std::io::Error),
    /// Cargo refused the requested change.
    #[error("Cargo failed while updating {package} to {version}")]
    CargoFailed {
        /// Crate Cargo was asked to update.
        package: String,
        /// Exact version requested.
        version: Version,
    },
}

/// Looks up the latest stable crates.io release for a package.
///
/// # Errors
///
/// Returns an error for an unavailable/malformed registry response or invalid
/// semantic version.
pub async fn check(package: &str, current: &str) -> Result<Release, UpdateError> {
    let current = Version::parse(current)?;
    let mut url = url::Url::parse(CRATES_IO)?;
    url.path_segments_mut()
        .map_err(|()| UpdateError::MissingStableVersion(package.to_owned()))?
        .push(package);
    let response = reqwest::Client::builder()
        .timeout(CHECK_TIMEOUT)
        .user_agent(format!("{CLIENT_CRATE}/{CLIENT_VERSION} updater"))
        .build()?
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json::<CratesIoResponse>()
        .await?;
    let Some(latest) = response.package.max_stable_version else {
        return Err(UpdateError::MissingStableVersion(package.to_owned()));
    };
    Ok(Release {
        current,
        latest: Version::parse(&latest)?,
    })
}

/// Advances one dependency in a Cargo lockfile to an exact release.
///
/// # Errors
///
/// Returns an error when Cargo cannot start or refuses the update.
pub fn update_dependency(
    manifest: &Path,
    package: &str,
    version: &Version,
) -> Result<(), UpdateError> {
    let Some(project) = manifest.parent() else {
        return Err(UpdateError::InvalidManifest(manifest.to_path_buf()));
    };
    let status = Command::new(cargo_program())
        .current_dir(project)
        .env("CARGO_HTTP_TIMEOUT", "10")
        .env("CARGO_NET_RETRY", "0")
        .arg("update")
        .arg("--manifest-path")
        .arg(manifest)
        .arg("-p")
        .arg(package)
        .arg("--precise")
        .arg(version.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(UpdateError::CargoFailed {
            package: package.to_owned(),
            version: version.clone(),
        })
    }
}

/// Compatibility shim: CLI installation is managed by Honeycomb.
///
/// # Errors
/// Always returns [`UpdateError::HoneycombManaged`].
pub fn install_binary(
    _package: &str,
    _binary: &str,
    _version: &Version,
) -> Result<(), UpdateError> {
    Err(UpdateError::HoneycombManaged)
}

/// Compatibility shim: CLI installation is managed by Honeycomb.
///
/// # Errors
/// Always returns [`UpdateError::HoneycombManaged`].
pub fn install_binary_at(
    _package: &str,
    _binary: &str,
    _version: &Version,
    _root: &Path,
) -> Result<(), UpdateError> {
    Err(UpdateError::HoneycombManaged)
}

/// Finds the nearest `Cargo.toml` at or above `start`.
#[must_use]
pub fn find_manifest(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|directory| directory.join("Cargo.toml"))
        .find(|manifest| manifest.is_file())
}

#[derive(Deserialize)]
struct CratesIoResponse {
    #[serde(rename = "crate")]
    package: CratesIoPackage,
}

#[derive(Deserialize)]
struct CratesIoPackage {
    max_stable_version: Option<String>,
}

fn cargo_program() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

/// Whether an environment value explicitly disables an updater.
#[must_use]
pub fn explicitly_disabled(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

#[cfg(test)]
mod tests {
    use super::{UpdateError, UpdateStatus, Version, explicitly_disabled, find_manifest};

    #[test]
    fn legacy_binary_installers_direct_callers_to_honeycomb() {
        let version = Version::new(99, 0, 0);
        assert!(matches!(
            super::install_binary("briefcase-cli", "briefcase", &version),
            Err(UpdateError::HoneycombManaged)
        ));
        assert!(matches!(
            super::install_binary_at(
                "briefcase-cli",
                "briefcase",
                &version,
                std::path::Path::new("/missing")
            ),
            Err(UpdateError::HoneycombManaged)
        ));
    }

    #[test]
    fn false_spellings_are_explicit() {
        for value in ["0", "false", "NO", " off "] {
            assert!(explicitly_disabled(value));
        }
        assert!(!explicitly_disabled("true"));
    }

    #[test]
    fn status_starts_not_checked() {
        assert_eq!(UpdateStatus::default(), UpdateStatus::NotChecked);
    }

    #[test]
    fn manifest_search_walks_upward() {
        let current = std::env::current_dir().unwrap();
        assert!(find_manifest(&current).is_some());
    }
}
