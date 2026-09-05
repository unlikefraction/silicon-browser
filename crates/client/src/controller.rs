//! Native controller execution on the caller's machine. Browser commands never traverse the API.
mod download;
mod upload;
use crate::{
    Error,
    shared::{CommandReport, RunEvent, RunResult, SessionConnection},
};
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

pub struct LocalController {
    binary: PathBuf,
}
#[derive(Debug)]
pub struct LocalExecution {
    pub result: RunResult,
    pub truncated: bool,
}
impl LocalExecution {
    /// Construct one idempotent report; retain this value when retrying delivery.
    pub fn command_report(&self, command: impl Into<String>, flags: &[String]) -> CommandReport {
        CommandReport {
            command_id: uuid::Uuid::now_v7(),
            command: command.into(),
            flags: flags.into(),
            started_at: self.result.started_at,
            finished_at: self.result.finished_at,
            exit_code: self.result.exit_code,
            truncated: self.truncated,
        }
    }
}
impl LocalController {
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self { binary: binary.into() }
    }
    pub fn namespace(backend: &str, org: &str, principal: &str, session: &str) -> String {
        let mut hash = Sha256::new();
        for value in [backend, org, principal, session] {
            hash.update(value.len().to_be_bytes());
            hash.update(value.as_bytes());
        }
        format!("sb-{:x}", hash.finalize())[..35].into()
    }
    pub fn run(
        &self,
        connection: &SessionConnection,
        namespace: &str,
        config: &Path,
        command: &str,
        flags: &[String],
        mut emit: impl FnMut(&RunEvent),
    ) -> Result<LocalExecution, Error> {
        let argv = controller_arguments(command, flags)?;
        if !config.is_absolute() {
            return Err(Error::Local("controller configuration path must be absolute".into()));
        }
        if namespace.len() > 48 || !namespace.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
            return Err(Error::Local("invalid native controller namespace".into()));
        }
        validate_connection(connection)?;
        let started_at = Utc::now();
        let remaining = connection
            .expires_at
            .signed_duration_since(started_at)
            .to_std()
            .map_err(|_| Error::Local("session expired; start another session".into()))?;
        if remaining.is_zero() {
            return Err(Error::Local("session expired; start another session".into()));
        }
        if remaining < Duration::from_secs(60) {
            emit(&RunEvent::Warning { message: format!("session expires in {} seconds", remaining.as_secs()) });
        }
        if first_controller_command(&argv) == Some("upload") {
            return self.upload_local(connection, namespace, config, &argv, started_at, &mut emit);
        }
        if first_controller_command(&argv) == Some("download") {
            return self.download_local(connection, namespace, config, &argv, started_at, &mut emit);
        }
        let mut process = self.process(connection, namespace, config);
        process.args(&argv).stdin(Stdio::inherit()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = process
            .spawn()
            .map_err(|_| Error::Local("could not start the local browser controller; run `sb setup`".into()))?;
        let (tx, rx) = mpsc::sync_channel(32);
        for (stderr, mut pipe) in [
            (false, Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>),
            (true, Box::new(child.stderr.take().unwrap()) as Box<dyn Read + Send>),
        ] {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut buffer = [0u8; 8192];
                loop {
                    match pipe.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if tx.send((stderr, buffer[..n].to_vec())).is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }
        drop(tx);
        let deadline = Instant::now() + remaining.min(Duration::from_secs(4 * 60 * 60));
        let mut out = RedactedOutput::new(&connection.cdp_url);
        let mut err = RedactedOutput::new(&connection.cdp_url);
        let mut status = None;
        let mut disconnected = false;
        let mut killed = false;
        while !disconnected || status.is_none() {
            if Instant::now() >= deadline && status.is_none() {
                let _ = child.kill();
                killed = true;
            }
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok((stderr, bytes)) => {
                    let target = if stderr { &mut err } else { &mut out };
                    target.push(&bytes, false, &mut |chunk| {
                        emit(&if stderr { RunEvent::Stderr { chunk } } else { RunEvent::Stdout { chunk } })
                    });
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => disconnected = true,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            status = child.try_wait().map_err(|_| Error::Local("could not observe local controller exit".into()))?;
            if disconnected && status.is_none() {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        out.push(&[], true, &mut |chunk| emit(&RunEvent::Stdout { chunk }));
        err.push(&[], true, &mut |chunk| emit(&RunEvent::Stderr { chunk }));
        let result = RunResult {
            session_id: connection.session_id.clone(),
            exit_code: if killed { 124 } else { status.unwrap().code().unwrap_or(1) },
            started_at,
            finished_at: Utc::now(),
            stdout: out.captured,
            stderr: err.captured,
        };
        emit(&RunEvent::Finished { result: result.clone() });
        Ok(LocalExecution { result, truncated: out.truncated || err.truncated })
    }
}

impl LocalController {
    fn process(&self, connection: &SessionConnection, namespace: &str, config: &Path) -> Command {
        let mut process = Command::new(&self.binary);
        // User files resolve in the caller's current working directory. Ambient controller
        // configuration cannot replace this authenticated managed connection.
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy();
            if ["AGENT_BROWSER_", "SB_", "IAM_", "BRIEFCASE_"].iter().any(|prefix| name.starts_with(prefix)) {
                process.env_remove(key);
            }
        }
        process
            .env("AGENT_BROWSER_CDP", &connection.cdp_url)
            .env("AGENT_BROWSER_SESSION", namespace)
            .env("AGENT_BROWSER_CONFIG", config)
            .env("AGENT_BROWSER_IDLE_TIMEOUT_MS", "300000");
        process
    }
}

pub fn validate_connection(connection: &SessionConnection) -> Result<(), Error> {
    let url = url::Url::parse(&connection.cdp_url).map_err(|_| Error::Protocol("invalid session connection".into()))?;
    let loopback = match url.host() {
        Some(url::Host::Domain(value)) => value.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(value)) => value.is_loopback(),
        Some(url::Host::Ipv6(value)) => value.is_loopback(),
        None => false,
    };
    if !(matches!(url.scheme(), "wss" | "https") || (matches!(url.scheme(), "ws" | "http") && loopback))
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::Protocol("invalid session connection".into()));
    }
    Ok(())
}

pub fn controller_arguments(command: &str, flags: &[String]) -> Result<Vec<String>, Error> {
    let mut args = shell_words::split(command).map_err(|_| Error::Local("command contains unclosed quoting".into()))?;
    args.extend_from_slice(flags);
    if args.is_empty() {
        return Err(Error::Local("a browser command is required".into()));
    }
    if first_controller_command(&args) == Some("wait")
        && args.iter().any(|arg| arg.split('=').next() == Some("--download"))
    {
        return Err(Error::Local(
            "wait --download cannot copy files from a remote browser; use `download <link-selector> <local-path>` for ordinary same-origin HTTP or blob/data links".into(),
        ));
    }
    if args.iter().any(|arg| arg.split('=').next() == Some("--download-path")) {
        return Err(Error::Local(
            "--download-path addresses the remote browser filesystem; use `download <link-selector> <local-path>` to copy supported link bytes locally".into(),
        ));
    }
    const OWNED: &[&str] = &[
        "--cdp",
        "--cdp-url",
        "--session",
        "--session-name",
        "--namespace",
        "--profile",
        "--provider",
        "-p",
        "--connect",
        "--auto-connect",
        "--config",
        "--proxy",
        "--proxy-username",
        "--proxy-password",
        "--engine",
        "--executable-path",
        "--idle-timeout",
    ];
    if args.iter().any(|arg| OWNED.contains(&arg.split('=').next().unwrap_or(arg)))
        || matches!(first_controller_command(&args), Some("connect" | "close" | "quit" | "exit" | "session" | "daemon"))
    {
        return Err(Error::Local(
            "connection and session lifecycle are managed by sb; use `sb session end` to stop a session".into(),
        ));
    }
    Ok(args)
}

// Mirror the pinned controller's global argument consumption only far enough to identify
// lifecycle commands. The command and all local paths are passed through unchanged.
fn first_controller_command(args: &[String]) -> Option<&str> {
    first_controller_command_index(args).map(|index| args[index].as_str())
}
fn first_controller_command_index(args: &[String]) -> Option<usize> {
    const VALUES: &[&str] = &[
        "--headers",
        "--extension",
        "--init-script",
        "--enable",
        "--state",
        "--proxy-bypass",
        "--args",
        "--user-agent",
        "--device",
        "--color-scheme",
        "--download-path",
        "--max-output",
        "--allowed-domains",
        "--action-policy",
        "--confirm-actions",
        "--screenshot-dir",
        "--screenshot-quality",
        "--screenshot-format",
        "--idle-timeout",
        "--ca-cert",
        "--model",
        "--restore-save",
        "--restore-check-url",
        "--restore-check-text",
        "--restore-check-fn",
    ];
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if VALUES.contains(&arg.as_str()) {
            index += 2;
            continue;
        }
        if arg.starts_with('-') {
            index += 1;
            if args.get(index).is_some_and(|value| matches!(value.as_str(), "true" | "false")) {
                index += 1;
            }
            continue;
        }
        return Some(index);
    }
    None
}

struct RedactedOutput {
    secret: Vec<u8>,
    pending: Vec<u8>,
    utf8: Vec<u8>,
    captured: String,
    truncated: bool,
}
impl RedactedOutput {
    fn new(secret: &str) -> Self {
        Self {
            secret: secret.as_bytes().to_vec(),
            pending: Vec::new(),
            utf8: Vec::new(),
            captured: String::new(),
            truncated: false,
        }
    }
    fn push(&mut self, bytes: &[u8], finish: bool, emit: &mut impl FnMut(String)) {
        self.pending.extend_from_slice(bytes);
        loop {
            if let Some(index) = self.pending.windows(self.secret.len()).position(|value| value == self.secret) {
                self.utf8.extend(self.pending.drain(..index));
                self.pending.drain(..self.secret.len());
                self.utf8.extend_from_slice(b"[REDACTED]");
            } else {
                let count = if finish {
                    self.pending.len()
                } else {
                    {
                        let keep = (1..self.secret.len().min(self.pending.len() + 1))
                            .rev()
                            .find(|&n| self.pending.ends_with(&self.secret[..n]))
                            .unwrap_or(0);
                        self.pending.len() - keep
                    }
                };
                self.utf8.extend(self.pending.drain(..count));
                break;
            }
        }
        let count = match std::str::from_utf8(&self.utf8) {
            Ok(_) => self.utf8.len(),
            Err(error) if !finish && error.error_len().is_none() => error.valid_up_to(),
            Err(_) => self.utf8.len(),
        };
        if count == 0 {
            return;
        }
        let chunk = String::from_utf8_lossy(&self.utf8.drain(..count).collect::<Vec<_>>()).into_owned();
        let mut count = (65536 - self.captured.len()).min(chunk.len());
        while !chunk.is_char_boundary(count) {
            count -= 1;
        }
        self.captured.push_str(&chunk[..count]);
        self.truncated |= count < chunk.len();
        emit(chunk);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secure_cdp_discovery_and_websocket_endpoints_are_supported() {
        for url in [
            "https://remote.example/cdp?token=secret",
            "wss://remote.example/cdp?token=secret",
            "http://127.0.0.1:9222",
            "ws://localhost:9222/devtools/browser/id",
        ] {
            let connection = SessionConnection {
                session_id: "session".into(),
                principal_id: "principal".into(),
                cdp_url: url.into(),
                expires_at: Utc::now() + chrono::Duration::minutes(5),
            };
            assert!(validate_connection(&connection).is_ok(), "{url}");
        }
        for url in [
            "http://remote.example/cdp",
            "ws://remote.example/cdp",
            "https://user:secret@remote.example/cdp",
            "https://remote.example/cdp#secret",
            "file:///tmp/browser",
        ] {
            let connection = SessionConnection {
                session_id: "session".into(),
                principal_id: "principal".into(),
                cdp_url: url.into(),
                expires_at: Utc::now() + chrono::Duration::minutes(5),
            };
            let error = validate_connection(&connection).unwrap_err();
            assert!(!error.to_string().contains("secret"));
        }
    }

    #[test]
    fn local_files_and_upstream_commands_are_preserved() {
        for command in [
            "screenshot './my capture.png'",
            "pdf report.pdf",
            "upload @e1 ./input.csv",
            "download @e2 ./files",
            "eval 'document.title'",
            "state save ./cookies.json",
            "record start demo.webm",
        ] {
            assert_eq!(controller_arguments(command, &[]).unwrap(), shell_words::split(command).unwrap());
        }
        assert_eq!(
            controller_arguments("snapshot", &["--json".into(), "--full".into()]).unwrap(),
            ["snapshot", "--json", "--full"]
        );
        for command in [
            "close",
            "quit",
            "--json close",
            "--json false --headers '{\"x\":\"y\"}' exit",
            "connect wss://outside.example",
            "snapshot --cdp=wss://other.example",
            "open https://example.com --config=elsewhere.json",
            "snapshot --idle-timeout 0",
            "wait --download ./result.csv",
            "--json wait --download=./result.csv",
            "click @e1 --download-path ./files",
        ] {
            assert!(controller_arguments(command, &[]).is_err());
        }
    }
    #[test]
    fn output_is_prompt_and_redacts_secrets_across_chunks() {
        let secret = "wss://remote.example/cdp?token=abcdef";
        let mut output = RedactedOutput::new(secret);
        let mut chunks = Vec::new();
        output.push(b"early output", false, &mut |chunk| chunks.push(chunk));
        assert_eq!(chunks.join(""), "early output");
        for byte in secret.as_bytes() {
            output.push(&[*byte], false, &mut |chunk| chunks.push(chunk));
        }
        output.push(" café".as_bytes(), true, &mut |chunk| chunks.push(chunk));
        assert_eq!(chunks.join(""), "early output[REDACTED] café");
        assert!(!output.captured.contains(secret));
    }
    #[test]
    fn telemetry_capture_is_bounded_without_truncating_terminal_output() {
        let mut output = RedactedOutput::new("wss://secret.example/path");
        let mut emitted = 0;
        output.push(&vec![b'x'; 100_000], true, &mut |chunk| emitted += chunk.len());
        assert_eq!(emitted, 100_000);
        assert_eq!(output.captured.len(), 65_536);
        assert!(output.truncated);
    }
    #[test]
    fn isolation_namespace_is_stable_and_scoped() {
        let a = LocalController::namespace("https://api.example", "org", "principal", "session");
        assert_eq!(a, LocalController::namespace("https://api.example", "org", "principal", "session"));
        assert_ne!(a, LocalController::namespace("https://other.example", "org", "principal", "session"));
        assert_ne!(a, LocalController::namespace("https://api.example", "org", "other", "session"));
    }
    #[cfg(unix)]
    #[test]
    fn subprocess_output_is_observable_before_it_exits() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("controller");
        let config = directory.path().join("config.json");
        let marker = directory.path().join("continue");
        std::fs::write(&config, "{}").unwrap();
        std::fs::write(&binary,"#!/bin/sh\nprintf 'early'\nfor attempt in 1 2 3 4 5 6 7 8 9 10; do [ -f \"$1\" ] && exit 0; /bin/sleep 0.05; done\nexit 99\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let connection = SessionConnection {
            session_id: "session".into(),
            principal_id: "principal".into(),
            cdp_url: "wss://remote.example/long-secret".into(),
            expires_at: Utc::now() + chrono::Duration::minutes(5),
        };
        let result = LocalController::new(binary)
            .run(&connection, "sb-test", &config, &shell_words::quote(marker.to_str().unwrap()), &[], |event| {
                if let RunEvent::Stdout { chunk } = event
                    && chunk == "early"
                {
                    std::fs::write(&marker, "continue").unwrap();
                }
            })
            .unwrap();
        assert_eq!(result.result.exit_code, 0);
    }
    #[cfg(unix)]
    #[test]
    fn controller_runs_locally_with_connection_only_in_environment() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("controller");
        let config = directory.path().join("config.json");
        std::fs::write(&config, "{}").unwrap();
        std::fs::write(&binary,"#!/bin/sh\n[ \"$1\" = screenshot ] || exit 91\n[ \"$2\" = './capture.png' ] || exit 92\nprintf '%s' \"$AGENT_BROWSER_CDP\"\nprintf 'local failure' >&2\nexit 23\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let connection = SessionConnection {
            session_id: "session".into(),
            principal_id: "principal".into(),
            cdp_url: "wss://remote.example/secret".into(),
            expires_at: Utc::now() + chrono::Duration::minutes(5),
        };
        let result = LocalController::new(binary)
            .run(&connection, "sb-test", &config, "screenshot './capture.png'", &[], |_| {})
            .unwrap();
        assert_eq!(result.result.exit_code, 23);
        assert_eq!(result.result.stdout, "[REDACTED]");
        assert_eq!(result.result.stderr, "local failure");
    }
}
