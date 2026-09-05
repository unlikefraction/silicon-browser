//! Explicit local runner setup. Nothing here runs unless the caller asks it to.

use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub use silicon_browser_shared::AGENT_BROWSER_VERSION;

use crate::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerStatus {
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    pub expected_version: &'static str,
    pub ready: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupEvent {
    Checking,
    InstallingCli,
    Ready(RunnerStatus),
}

pub fn runner_status(binary: impl AsRef<Path>) -> RunnerStatus {
    let binary = binary.as_ref();
    let output = Command::new(binary).arg("--version").stderr(Stdio::null()).output();
    let (path, version) = match output {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            let version = text.split_whitespace().last().map(str::to_owned);
            (Some(binary.to_owned()), version)
        }
        _ => (None, None),
    };
    let ready = version.as_deref() == Some(AGENT_BROWSER_VERSION);
    RunnerStatus { path, version, expected_version: AGENT_BROWSER_VERSION, ready }
}

/// File name of the private native controller installed by setup.
pub fn runner_file_name() -> &'static str {
    if cfg!(windows) { "sb-browser-engine.exe" } else { "sb-browser-engine" }
}

/// Install the native controller into an explicit caller-owned private directory. No package
/// manager, Node runtime, or local Chromium is installed. Existing pinned PATH installations
/// are reused. Downloads are bounded and verified against compiled-in release SHA-256 digests.
pub fn ensure_runner(directory: &Path, mut on_event: impl FnMut(SetupEvent)) -> Result<RunnerStatus, Error> {
    on_event(SetupEvent::Checking);
    let target = directory.join(runner_file_name());
    let metadata =
        fs::symlink_metadata(directory).map_err(|_| Error::Local("controller directory is unavailable".into()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error::Local("controller directory must be a private non-symlink directory".into()));
    }
    if fs::symlink_metadata(&target).is_ok_and(|metadata| metadata.file_type().is_symlink() || !metadata.is_file()) {
        return Err(Error::Local("controller target must be a regular non-symlink file".into()));
    }
    let existing = runner_status(&target);
    if existing.ready {
        on_event(SetupEvent::Ready(existing.clone()));
        return Ok(existing);
    }
    // Runtime prefers a private installation. Do not let a stale private binary shadow a
    // correctly pinned PATH binary after setup reports success.
    if !target.exists() {
        let existing = runner_status("agent-browser");
        if existing.ready {
            on_event(SetupEvent::Ready(existing.clone()));
            return Ok(existing);
        }
    }
    on_event(SetupEvent::InstallingCli);
    let (asset, digest) = release_asset(std::env::consts::OS, std::env::consts::ARCH, cfg!(target_env = "musl"))?;
    let url =
        format!("https://github.com/vercel-labs/agent-browser/releases/download/v{AGENT_BROWSER_VERSION}/{asset}");
    let agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(5 * 60))).build().new_agent();
    let bytes = agent
        .get(&url)
        .call()
        .map_err(|_| Error::Local("could not download the browser controller; retry `sb setup`".into()))?
        .body_mut()
        .with_config()
        .limit(32 * 1024 * 1024)
        .read_to_vec()
        .map_err(|_| Error::Local("browser controller download was incomplete".into()))?;
    if format!("{:x}", Sha256::digest(&bytes)) != digest {
        return Err(Error::Local("browser controller download failed its integrity check".into()));
    }
    let temporary = directory.join(format!(".controller-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o700);
        }
        let mut file =
            options.open(&temporary).map_err(|_| Error::Local("could not write the browser controller".into()))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| Error::Local("could not save the browser controller".into()))?;
        if !runner_status(&temporary).ready {
            return Err(Error::Local("downloaded browser controller could not run on this machine".into()));
        }
        fs::rename(&temporary, &target).map_err(|_| Error::Local("could not install the browser controller".into()))?;
        let status = runner_status(&target);
        on_event(SetupEvent::Ready(status.clone()));
        Ok(status)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

// Verified against the immutable v0.36.0 GitHub release asset digests.
// https://github.com/vercel-labs/agent-browser/releases/tag/v0.36.0
fn release_asset(os: &str, arch: &str, musl: bool) -> Result<(&'static str, &'static str), Error> {
    match (os, arch, musl) {
        ("macos", "aarch64", _) => {
            Ok(("agent-browser-darwin-arm64", "b2106ab39db0838e7b1772f7f26f760518de56d09053150c56f9dddf15af997d"))
        }
        ("macos", "x86_64", _) => {
            Ok(("agent-browser-darwin-x64", "45d9ac061a7d72e61eaff905326e2e19365f4dadb12142ea2f2d76d84689c708"))
        }
        ("linux", "aarch64", false) => {
            Ok(("agent-browser-linux-arm64", "aeb556addca3903601a433de1acad3ace1c9c61d170084bf58d875884599a990"))
        }
        ("linux", "x86_64", false) => {
            Ok(("agent-browser-linux-x64", "56d15181e51e00213f907fcf39707cfc76bfa804ff20f5a9373661c73f96de5e"))
        }
        ("linux", "aarch64", true) => {
            Ok(("agent-browser-linux-musl-arm64", "1ca7e003c9cb185f174fc81e51a609db27c77e3bfe00a0edff60688f8cd14f88"))
        }
        ("linux", "x86_64", true) => {
            Ok(("agent-browser-linux-musl-x64", "a20cc2a5202a48f5820372803dedbcd5f556dff7a89421f1b0f2612962b10718"))
        }
        ("windows", "x86_64", _) => {
            Ok(("agent-browser-win32-x64.exe", "412ff72737a109e93f5304b0ff76c988fb6f1f451d0fc7e010577922bcc20ff3"))
        }
        _ => Err(Error::Local("no native browser controller release exists for this platform".into())),
    }
}

/// Load version-matched upstream documentation only when requested and rewrite its executable
/// examples for a managed Silicon Browser session.
pub fn runner_help(binary: impl AsRef<Path>, session_id: &str) -> Result<String, Error> {
    if session_id.trim().is_empty() {
        return Err(Error::Local("a session id is required to render run help".into()));
    }
    let binary = binary.as_ref();
    let status = runner_status(binary);
    if !status.ready {
        return Err(Error::Local(format!(
            "run help requires browser controller {AGENT_BROWSER_VERSION}; found {}. Run `sb setup` first",
            status.version.as_deref().unwrap_or("no runnable browser controller")
        )));
    }
    let output = Command::new(binary)
        .arg("--help")
        .output()
        .map_err(|error| Error::Local(format!("could not run browser controller: {error}")))?;
    if !output.status.success() {
        return Err(Error::Local(String::from_utf8_lossy(&output.stderr).trim().to_owned()));
    }
    let upstream = managed_help(&String::from_utf8_lossy(&output.stdout), session_id);
    Ok(format!(
        "Managed-session note: connection and session lifecycle are managed by Silicon Browser. File paths resolve on your machine; screenshots, PDF, upload, download, and local recording commands are available. Use `sb session end {session_id} --note \"...\"` instead of `close`.\n\n{upstream}"
    ))
}

fn managed_help(upstream: &str, session: &str) -> String {
    // The native release embeds command help; bundled npm skill files are not required.
    // Drop upstream installation/configuration/provider surfaces owned by sb, retaining the
    // complete browser action reference and local-file arguments from the installed version.
    let mut lines = Vec::new();
    let mut keep = false;
    let mut skip_entry = false;
    for line in upstream.lines() {
        if !line.starts_with(' ') && line.contains(':') {
            let title = line.split(':').next().unwrap();
            keep = !matches!(
                title,
                "Start here (for AI agents)"
                    | "Plugins"
                    | "Sessions"
                    | "MCP"
                    | "Chat (AI)"
                    | "Dashboard"
                    | "Setup"
                    | "Configuration"
                    | "Environment"
                    | "Install"
                    | "iOS Simulator (requires Xcode and Appium)"
            );
            skip_entry = false;
        }
        if !keep {
            continue;
        }
        let trimmed = line.trim_start();
        if line.starts_with("  ") && !line.starts_with("   ") {
            skip_entry = false;
            let command = trimmed.strip_prefix("agent-browser ").unwrap_or(trimmed);
            let first = command.split_whitespace().next().unwrap_or("").trim_end_matches(',');
            if matches!(
                first,
                "connect"
                    | "close"
                    | "session"
                    | "profiles"
                    | "skills"
                    | "chat"
                    | "--provider"
                    | "-p"
                    | "--profile"
                    | "--auto-connect"
                    | "--session-name"
                    | "--session"
                    | "--namespace"
                    | "--executable-path"
                    | "--idle-timeout"
                    | "--download-path"
                    | "--proxy"
                    | "--cdp"
                    | "--engine"
                    | "--config"
                    | "--model"
            ) || (first == "wait" && command.contains("--download"))
                || trimmed.starts_with("SESSION=")
                || (trimmed.starts_with("agent-browser ")
                    && crate::controller::controller_arguments(command, &[]).is_err())
            {
                skip_entry = true;
            }
        }
        if !skip_entry {
            // These ambient controller variables are intentionally overridden by the wrapper.
            let line = line.split_once("(or AGENT_BROWSER_").map_or(line, |(prefix, _)| prefix.trim_end());
            if !line.is_empty() {
                lines.push(line.replace("agent-browser", &format!("sb run {session}")));
            }
        }
    }
    lines.push("\nLocal file transfers:\n  upload transfers local file bytes to file inputs in the current page or same-origin frames.\n  download copies ordinary same-origin HTTP link targets or blob/data link bytes to a local file.\n  Button/script/POST downloads, cross-origin frames, wait --download and --download-path are unsupported.\n  Local controller daemons exit after five idle minutes; use sb session end for managed lifecycle.".into());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_runner_is_reported_without_installing_anything() {
        let status = runner_status("definitely-not-an-agent-browser-binary");
        assert!(!status.ready);
        assert!(status.path.is_none());
    }

    #[test]
    fn release_targets_are_explicit_and_have_pinned_integrity() {
        for (os, arch, musl) in [
            ("macos", "aarch64", false),
            ("macos", "x86_64", false),
            ("linux", "aarch64", false),
            ("linux", "x86_64", false),
            ("linux", "aarch64", true),
            ("linux", "x86_64", true),
            ("windows", "x86_64", false),
        ] {
            let (name, digest) = release_asset(os, arch, musl).unwrap();
            assert!(name.starts_with("agent-browser-"));
            assert_eq!(digest.len(), 64);
            assert!(digest.bytes().all(|value| value.is_ascii_hexdigit()));
        }
        assert!(release_asset("unsupported", "unknown", false).is_err());
    }

    #[test]
    fn native_help_keeps_actions_and_local_paths_without_upstream_setup() {
        let text = "Start here (for AI agents):\n  agent-browser skills get core --full\nCore Commands:\n  screenshot [path]          Take screenshot\n  connect <url>              Connect\n  upload <sel> <files>        Upload local files\nSetup:\n  install                    Install Chromium\nOptions:\n  -p, --provider <name> Browser provider browseruse\n  --cdp <url>                Override connection\n                             private continuation\n  --download-path <path>     Local output\nEnvironment:\n  AGENT_BROWSER_PROVIDER browseruse\nExamples:\n  agent-browser screenshot ./file.png\n  agent-browser --cdp 9222 snapshot\n";
        let help = managed_help(text, "session");
        assert!(help.contains("sb run session screenshot ./file.png"));
        assert!(help.contains("upload <sel>"));
        assert!(help.contains("--download-path"));
        for hidden in ["agent-browser", "browseruse", "Chromium", "skills get", "--cdp", "private continuation"] {
            assert!(!help.contains(hidden), "{hidden}");
        }
    }

    #[test]
    fn runner_version_is_exactly_pinned() {
        assert_eq!(AGENT_BROWSER_VERSION, "0.36.0");
    }
}
