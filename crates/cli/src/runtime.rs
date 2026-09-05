//! Caller-owned connection cache and durable cooperative telemetry. No browser action is retried.
use crate::state::{self, State};
use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use silicon_browser::{
    Client,
    controller::{LocalController, LocalExecution, controller_arguments},
    shared::*,
};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct CachedConnection {
    fetched_at: DateTime<Utc>,
    connection: SessionConnection,
}
#[derive(Serialize, Deserialize)]
struct PendingReport {
    session_id: String,
    report: CommandReport,
}

pub struct Runtime {
    directory: PathBuf,
}
impl Runtime {
    pub fn new(home: &Path, state: &State) -> io::Result<Self> {
        let org = state.org_id.as_deref().ok_or_else(|| io::Error::other("no organization selected"))?;
        let credential =
            if State::has_environment_access_token() { state.token() } else { state.credential_generation.clone() }
                .ok_or_else(|| io::Error::other("no local authentication binding"))?;
        let runtime = home.join("runtime");
        state::secure_home(&runtime, true)?;
        let directory = runtime.join(hash(&[org, &credential]));
        state::secure_home(&directory, true)?;
        Ok(Self { directory })
    }
    pub fn run(
        &self,
        client: &Client,
        session: &str,
        command: &str,
        flags: &[String],
        mut emit: impl FnMut(&RunEvent),
    ) -> Result<LocalExecution, Box<dyn std::error::Error>> {
        // Reject invalid input and an unwritable spool before doing anything in the browser.
        RunRequest { session_id: session.into(), command: command.into(), flags: flags.into() }.validate()?;
        controller_arguments(command, flags)?;
        let connection = self.connection(client, session)?;
        let namespace =
            LocalController::namespace(client.base_url(), client.org_id().unwrap(), &connection.principal_id, session);
        let lock = state::open_lock_file(&self.directory.join(format!("{namespace}.lock")))?;
        lock.lock_exclusive()?;
        let config = self.directory.join("controller.json");
        // An explicit empty configuration keeps unrelated local defaults from changing connection.
        write_json(&config, &serde_json::json!({}))?;
        let config = fs::canonicalize(config)?;
        let binary = controller_binary();
        let result = LocalController::new(binary).run(&connection, &namespace, &config, command, flags, &mut emit)?;
        let report = result.command_report(command, flags);
        let pending = PendingReport { session_id: session.into(), report };
        let saved = write_json(&self.directory.join(format!("report-{}.json", pending.report.command_id)), &pending);
        // Releasing execution before network telemetry lets the next local action proceed.
        let _ = FileExt::unlock(&lock);
        if saved.is_err() {
            emit(&RunEvent::Warning {
                message: "the browser action finished, but its command log could not be saved locally".into(),
            });
        } else if self.sync(client, Some(session), 8).is_err() {
            emit(&RunEvent::Warning {
                message: format!(
                    "command logs are queued locally; `sb session sync {session}` retries delivery without repeating browser actions"
                ),
            });
        }
        Ok(result)
    }
    fn connection(&self, client: &Client, session: &str) -> Result<SessionConnection, Box<dyn std::error::Error>> {
        let file = self.directory.join(format!("connection-{}.json", hash(&[session])));
        let lock = state::open_lock_file(&file.with_extension("lock"))?;
        lock.lock_exclusive()?;
        if let Some(cached) = read_json::<CachedConnection>(&file)? {
            let age = Utc::now().signed_duration_since(cached.fetched_at);
            if cached.connection.session_id == session
                && age >= chrono::Duration::zero()
                && age < chrono::Duration::seconds(60)
                && cached.connection.expires_at > Utc::now()
            {
                return Ok(cached.connection);
            }
        }
        let connection = client.session_connection(session)?;
        write_json(&file, &CachedConnection { fetched_at: Utc::now(), connection: connection.clone() })?;
        Ok(connection)
    }
    /// Deliver stored telemetry only. Success never means that an action was executed again.
    pub fn sync(
        &self,
        client: &Client,
        session: Option<&str>,
        limit: usize,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let lock = state::open_lock_file(&self.directory.join("reports.lock"))?;
        // Another local invocation owns delivery; leave its work undisturbed.
        if let Err(error) = lock.try_lock_exclusive() {
            if error.kind() == io::ErrorKind::WouldBlock {
                return Ok(0);
            }
            return Err(error.into());
        }
        let mut paths = fs::read_dir(&self.directory)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name().is_some_and(|name| {
                    name.to_string_lossy().starts_with("report-") && name.to_string_lossy().ends_with(".json")
                })
            })
            .collect::<Vec<_>>();
        paths.sort();
        let mut sent = 0;
        for path in paths {
            let Some(pending) = read_json::<PendingReport>(&path)? else {
                continue;
            };
            if session.is_some_and(|session| session != pending.session_id) {
                continue;
            }
            client.report_command(&pending.session_id, &pending.report)?;
            fs::remove_file(path)?;
            sent += 1;
            if sent >= limit {
                break;
            }
        }
        state::secure_home(&self.directory, false)?.sync_all()?;
        Ok(sent)
    }
    pub fn forget_connection(&self, session: &str) -> io::Result<()> {
        let path = self.directory.join(format!("connection-{}.json", hash(&[session])));
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}
fn hash(values: &[&str]) -> String {
    let mut digest = Sha256::new();
    for value in values {
        digest.update(value.len().to_be_bytes());
        digest.update(value.as_bytes());
    }
    format!("{:x}", digest.finalize())
}
fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    let file = match state::open_existing_state(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(io::Error::other("local runtime record exceeds 2 MiB"));
    }
    serde_json::from_slice(&bytes).map(Some).map_err(io::Error::other)
}
fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    state::add_no_follow_flags(&mut options);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    state::validate_private_regular_file(&file, "runtime file")?;
    let result = (|| {
        serde_json::to_writer(&mut file, value).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        state::reject_symlink_or_special_target(path)?;
        fs::rename(&temporary, path)?;
        File::open(path.parent().unwrap())?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

/// Runtime selection does not modify PATH or another application's global installation.
pub fn controller_binary() -> PathBuf {
    if let Some(binary) = std::env::var_os("SB_CONTROLLER_BIN") {
        return binary.into();
    }
    let private = state::default_home().join("bin").join(silicon_browser::setup::runner_file_name());
    if private.is_file() { private } else { PathBuf::from("agent-browser") }
}
