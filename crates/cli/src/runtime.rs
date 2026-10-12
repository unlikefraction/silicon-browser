//! Caller-owned connection cache for local browser actions.
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
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct CachedConnection {
    fetched_at: DateTime<Utc>,
    connection: SessionConnection,
}

pub struct Runtime {
    directory: PathBuf,
}
impl Runtime {
    pub fn new(home: &Path, state: &State) -> io::Result<Self> {
        let credential =
            if State::has_environment_access_token() { state.token() } else { state.credential_generation.clone() }
                .ok_or_else(|| io::Error::other("no local authentication binding"))?;
        let runtime = home.join("runtime");
        state::secure_home(&runtime, true)?;
        discard_legacy_reports(&runtime)?;
        let directory = runtime.join(hash(&["browser", &credential]));
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
        // Reject invalid input before doing anything in the browser.
        RunRequest { session_id: session.into(), command: command.into(), flags: flags.into() }.validate()?;
        controller_arguments(command, flags)?;
        let connection = self.connection(client, session)?;
        let namespace = LocalController::namespace(client.base_url(), &connection.principal_id, session);
        let lock = state::open_lock_file(&self.directory.join(format!("{namespace}.lock")))?;
        lock.lock_exclusive()?;
        let config = self.directory.join("controller.json");
        // An explicit empty configuration keeps unrelated local defaults from changing connection.
        write_json(&config, &serde_json::json!({}))?;
        let config = fs::canonicalize(config)?;
        let binary = controller_binary();
        let result = LocalController::new(binary).run(&connection, &namespace, &config, command, flags, &mut emit)?;
        Ok(result)
    }
    fn connection(&self, client: &Client, session: &str) -> Result<SessionConnection, Box<dyn std::error::Error>> {
        let file = self.directory.join(format!("connection-{}.json", hash(&[session])));
        let lock = state::open_lock_file(&file.with_extension("lock"))?;
        lock.lock_exclusive()?;
        if let Some(cached) = read_json::<CachedConnection>(&file)? {
            let age = Utc::now().signed_duration_since(cached.fetched_at);
            if cached.connection.session_id == session
                && account_id(&cached.connection.principal_id, "principal_id").is_ok()
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
    pub fn forget_connection(&self, session: &str) -> io::Result<()> {
        let path = self.directory.join(format!("connection-{}.json", hash(&[session])));
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}
// Older releases queued command reports here. Discard them without opening their contents.
fn discard_legacy_reports(runtime: &Path) -> io::Result<()> {
    for entry in fs::read_dir(runtime)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let directory = entry.path();
        state::secure_home(&directory, false)?;
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "reports.lock"
                || name.starts_with("report-") && (name.ends_with(".json") || name.ends_with(".tmp"))
            {
                match fs::remove_file(entry.path()) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
    }
    Ok(())
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
        state::sync_directory(path.parent().unwrap())
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
    if let Some(binary) = silicon_browser::setup::bundled_runner() {
        return binary;
    }
    let private = state::default_home().join("bin").join(silicon_browser::setup::runner_file_name());
    if private.is_file() { private } else { PathBuf::from("agent-browser") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrading_discards_queued_command_reports() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("state");
        let state = State::update(&home, |state| {
            state.credential_generation = Some("existing-login".into());
        })
        .unwrap();
        state::secure_home(&home.join("runtime"), true).unwrap();
        let previous_directory = home.join("runtime").join(hash(&["browser", "existing-login"]));
        state::secure_home(&previous_directory, true).unwrap();
        let report = previous_directory.join("report-pending.json");
        fs::write(&report, b"previous version report").unwrap();
        let connection = previous_directory.join("connection-existing.json");
        fs::write(&connection, b"cached connection").unwrap();
        let unrelated = root.path().join("unrelated");
        fs::create_dir(&unrelated).unwrap();
        fs::write(unrelated.join("report-private.json"), b"unrelated file").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&unrelated, home.join("runtime/linked-directory")).unwrap();
        let runtime = Runtime::new(&home, &state).unwrap();
        assert_eq!(runtime.directory, previous_directory);
        assert!(!report.exists());
        assert_eq!(fs::read(connection).unwrap(), b"cached connection");
        assert_eq!(fs::read(unrelated.join("report-private.json")).unwrap(), b"unrelated file");
    }

    #[test]
    fn stale_connection_identity_is_refetched() {
        struct ConnectionTransport(SessionConnection);
        impl silicon_browser::Transport for ConnectionTransport {
            fn send(
                &self,
                request: silicon_browser::Request,
            ) -> Result<silicon_browser::Response, silicon_browser::Error> {
                assert!(request.url.ends_with("/sessions/session/connection"));
                Ok(silicon_browser::Response {
                    status: 200,
                    body: serde_json::to_vec(&Envelope::new(self.0.clone())).unwrap(),
                })
            }
        }
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("runtime");
        state::secure_home(&directory, true).unwrap();
        let canonical = SessionConnection {
            session_id: "session".into(),
            principal_id: "11111111-1111-4111-8111-111111111115".into(),
            cdp_url: "wss://provider.example/cdp".into(),
            expires_at: Utc::now() + chrono::Duration::minutes(5),
        };
        let client = Client::with_transport(
            "https://backend.example",
            silicon_browser::Auth::new("oat_current").unwrap(),
            std::sync::Arc::new(ConnectionTransport(canonical.clone())),
        )
        .unwrap();
        let runtime = Runtime { directory };
        let file = runtime.directory.join(format!("connection-{}.json", hash(&["session"])));
        for old in ["c:alice0", "00000000-0000-0000-0000-000000000000"] {
            let mut legacy = canonical.clone();
            legacy.principal_id = old.into();
            write_json(&file, &CachedConnection { fetched_at: Utc::now(), connection: legacy }).unwrap();
            assert_eq!(runtime.connection(&client, "session").unwrap(), canonical);
        }
    }

    #[test]
    fn runtime_records_replace_existing_values_and_flush_portably() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("runtime");
        state::secure_home(&home, true).unwrap();
        let path = home.join("connection.json");
        write_json(&path, &"old").unwrap();
        write_json(&path, &"new").unwrap();
        assert_eq!(read_json::<String>(&path).unwrap().as_deref(), Some("new"));
        assert_eq!(fs::read_dir(home).unwrap().count(), 1);
    }
}
