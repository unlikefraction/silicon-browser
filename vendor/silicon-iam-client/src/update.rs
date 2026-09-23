//! Crates.io version discovery and Cargo-aware updates.
//!
//! A compiled Rust library cannot replace the code already loaded into a
//! running process. The automatic client updater therefore advances the
//! consuming project's `Cargo.lock`; the next build uses the new version.
//! Automatic checks are driven by completed IAM requests, at most once per
//! hour per client and its clones. No idle background timer or daemon runs.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub use semver::Version;
use serde::Deserialize;

/// The crates.io package containing this client.
pub const CLIENT_CRATE: &str = "silicon-iam-client";

/// This compiled client's package version.
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const CRATES_IO: &str = "https://crates.io/api/v1/crates";
const CHECK_TIMEOUT: Duration = Duration::from_secs(3);

/// Whether a package should maintain itself from crates.io.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UpdatePolicy {
    /// Legacy setting, retained for source compatibility; runtime updates are disabled.
    Automatic,
    /// Do not contact crates.io or invoke Cargo.
    #[default]
    Disabled,
}

impl UpdatePolicy {
    /// Runtime dependency maintenance is always disabled.
    #[must_use]
    pub fn from_environment() -> Self {
        Self::Disabled
    }
}

/// A stable release comparison returned by crates.io.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Release {
    /// Version compiled into the caller.
    pub current: Version,
    /// Latest non-prerelease version published to crates.io.
    pub latest: Version,
}

impl Release {
    /// Whether crates.io contains a newer stable release.
    #[must_use]
    pub fn update_available(&self) -> bool {
        self.latest > self.current
    }
}

/// What an automatic client update has done in this process.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum UpdateStatus {
    /// No API request has completed an automatic update check yet.
    #[default]
    NotChecked,
    /// Automatic updates were explicitly disabled.
    Disabled,
    /// No Cargo project was discoverable from the process working directory.
    NoCargoProject,
    /// The project already resolves the newest stable client.
    Current {
        /// The current and latest version.
        version: Version,
    },
    /// `Cargo.lock` was advanced; rebuilding loads this version.
    Updated {
        /// The version used by the running process.
        from: Version,
        /// The version selected for the next build.
        to: Version,
    },
    /// The update was best-effort and could not be completed.
    Failed {
        /// A human-readable reason. IAM requests still continue.
        reason: String,
    },
}

/// An update check or Cargo operation failed.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// Dependency versions belong to the consuming project; CLI releases belong to Honeycomb.
    #[error(
        "Runtime updates are disabled. Update Rust dependencies through your project, or run honeycomb update <configured-iam-app-id> for the CLI."
    )]
    ManagedExternally,
    /// The crates.io endpoint could not be represented as a URL.
    #[error("invalid crates.io URL: {0}")]
    Url(#[from] url::ParseError),
    /// The compiled version or registry response was not semantic versioning.
    #[error("invalid package version: {0}")]
    Version(#[from] semver::Error),
    /// crates.io could not be reached or returned an unsuccessful response.
    #[error("cannot query crates.io: {0}")]
    Registry(#[from] reqwest::Error),
    /// crates.io returned a response without a stable package version.
    #[error("crates.io returned no stable version for {0}")]
    MissingStableVersion(String),
    /// The Cargo manifest path has no containing directory.
    #[error("Cargo manifest {0} has no parent directory")]
    InvalidManifest(PathBuf),
    /// Cargo could not be started.
    #[error("cannot run Cargo: {0}")]
    CargoIo(#[from] std::io::Error),
    /// Cargo ran but refused the requested update.
    #[error("Cargo failed while updating {package} to {version}")]
    CargoFailed {
        /// Package Cargo was asked to update.
        package: String,
        /// Exact release Cargo was asked to select.
        version: Version,
    },
}

/// Looks up the latest stable crates.io release for a package.
///
/// # Errors
///
/// Returns an error when the current version is invalid, crates.io is not
/// reachable, its response is malformed, or no stable version exists.
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

/// Advances one dependency in a Cargo project's lockfile to an exact release.
///
/// The compiled process keeps using its current code. The selected release is
/// loaded by the next Cargo build.
///
/// # Errors
///
/// Returns an error when Cargo cannot run or rejects the update, including
/// when the manifest pins an incompatible exact version.
pub fn update_dependency(
    _manifest: &Path,
    _package: &str,
    _version: &Version,
) -> Result<(), UpdateError> {
    Err(UpdateError::ManagedExternally)
}

/// Compatibility entry point. Use Honeycomb to install CLI releases.
/// # Errors
/// Always returns [`UpdateError::ManagedExternally`].
pub fn install_binary(_package: &str, _version: &Version) -> Result<(), UpdateError> {
    Err(UpdateError::ManagedExternally)
}

/// Compatibility entry point. Use Honeycomb to install CLI releases.
/// # Errors
/// Always returns [`UpdateError::ManagedExternally`].
pub fn install_binary_in(
    _package: &str,
    _version: &Version,
    _root: &Path,
) -> Result<(), UpdateError> {
    Err(UpdateError::ManagedExternally)
}

/// Finds the nearest Cargo project at or above a starting directory.
#[must_use]
pub fn find_manifest(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|directory| directory.join("Cargo.toml"))
        .find(|manifest| manifest.is_file())
}

#[derive(Debug, Deserialize)]
struct CratesIoResponse {
    #[serde(rename = "crate")]
    package: CratesIoPackage,
}

#[derive(Debug, Deserialize)]
struct CratesIoPackage {
    max_stable_version: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use semver::Version;

    use super::{CratesIoResponse, Release, UpdateStatus, find_manifest};

    #[test]
    fn release_comparison_never_downgrades() {
        let release = Release {
            current: Version::new(1, 2, 0),
            latest: Version::new(1, 1, 9),
        };
        assert!(!release.update_available());
    }

    #[test]
    fn the_workspace_manifest_is_discoverable() {
        let start = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(find_manifest(start).is_some());
    }

    #[test]
    fn status_starts_without_claiming_a_check_happened() {
        assert_eq!(UpdateStatus::default(), UpdateStatus::NotChecked);
    }

    #[test]
    fn crates_io_stable_version_shape_is_strictly_decoded() {
        let decoded =
            serde_json::from_str::<CratesIoResponse>(r#"{"crate":{"max_stable_version":"1.2.3"}}"#);
        assert_eq!(
            decoded
                .ok()
                .and_then(|response| response.package.max_stable_version),
            Some("1.2.3".to_owned())
        );
    }
}
