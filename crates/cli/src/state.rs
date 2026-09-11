//! Owner-only CLI state. The public library never reads this file.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const STATE_FILE: &str = "state.json";
const MAX_STATE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default = "default_backend")]
    pub backend_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    access_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_id: Option<String>,
    /// A local login generation changes on setup, but survives ordinary refresh rotation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_generation: Option<String>,
    /// Services advertised by the backend when this credential was exchanged or refreshed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_command: Option<String>,
}

impl State {
    pub fn load(home: &Path) -> io::Result<Self> {
        match secure_home(home, false) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::new_default()),
            Err(error) => return Err(error),
        }
        let mut file = match open_existing_state(&home.join(STATE_FILE)) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::new_default()),
            Err(error) => return Err(error),
        };
        if file.metadata()?.len() > MAX_STATE_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "CLI state exceeds 1 MiB"));
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file).take(MAX_STATE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "CLI state exceeds 1 MiB"));
        }
        serde_json::from_slice(&bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    fn new_default() -> Self {
        Self { backend_url: default_backend(), ..Self::default() }
    }

    /// Apply a field-level update to the latest state under an exclusive lock.
    ///
    /// Callers deliberately do not save a previously loaded `State` wholesale: another process
    /// may have rotated the refresh token since that snapshot was read.
    pub fn update(home: &Path, update: impl FnOnce(&mut Self)) -> io::Result<Self> {
        secure_home(home, true)?;
        let lock_path = home.join("state.lock");
        let lock = open_lock_file(&lock_path)?;
        lock.lock_exclusive()?;
        let result = (|| {
            let mut latest = Self::load(home)?;
            update(&mut latest);
            latest.write_atomic(home)?;
            Ok(latest)
        })();
        let _ = FileExt::unlock(&lock);
        result
    }

    fn write_atomic(&self, home: &Path) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        let (temporary, mut file) = create_temporary_state(home)?;
        let result = (|| {
            file.write_all(&bytes)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            let target = home.join(STATE_FILE);
            reject_symlink_or_special_target(&target)?;
            fs::rename(&temporary, target)?;
            secure_home(home, false)?.sync_all()
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn token(&self) -> Option<String> {
        std::env::var("SB_AUTHTOKEN")
            .ok()
            .filter(|value| !value.trim().is_empty() && value.starts_with("oat_"))
            .or_else(|| self.access_token.clone())
    }

    pub fn stored_token(&self) -> Option<&str> {
        self.access_token.as_deref()
    }

    pub fn refresh_token(&self) -> Option<&str> {
        self.refresh_token.as_deref()
    }

    pub fn set_tokens(&mut self, access: String, refresh: Option<String>, expires_at: Option<String>) {
        self.access_token = Some(access);
        self.refresh_token = refresh;
        self.token_expires_at = expires_at;
    }

    pub fn has_environment_access_token() -> bool {
        std::env::var("SB_AUTHTOKEN").is_ok_and(|value| !value.trim().is_empty() && value.starts_with("oat_"))
    }

    /// Merge activity into the newest state without touching credentials or selected defaults.
    pub fn record_activity(home: &Path, last_command: String, new_session_id: Option<String>) -> io::Result<Self> {
        Self::update(home, |latest| {
            latest.last_command = Some(last_command);
            if let Some(session_id) = new_session_id {
                latest.last_session_id = Some(session_id);
            }
        })
    }

    /// Remember structure but never values commonly used to enter credentials or headers.
    pub fn remember(&mut self, arguments: &[String]) {
        if let Some(run_index) = arguments.iter().position(|argument| argument == "run") {
            self.last_command = Some(
                arguments
                    .get(run_index + 1)
                    .map_or_else(|| "run".into(), |session| format!("run {session} [REDACTED]")),
            );
            return;
        }
        let mut redact_next = false;
        let sanitized = arguments
            .iter()
            .map(|argument| {
                if redact_next {
                    redact_next = false;
                    return "[REDACTED]".to_owned();
                }
                let lower = argument.to_ascii_lowercase();
                if ["--token", "--password", "--password-stdin", "--headers", "--credentials", "--backend"]
                    .iter()
                    .any(|secret| lower == *secret)
                {
                    redact_next = true;
                    return argument.clone();
                }
                if lower.starts_with("--token=")
                    || lower.starts_with("--password=")
                    || lower.starts_with("--headers=")
                    || lower.starts_with("--backend=")
                {
                    return format!(
                        "{}=[REDACTED]",
                        argument.split_once('=').map_or(argument.as_str(), |(key, _)| key)
                    );
                }
                argument.clone()
            })
            .collect::<Vec<_>>();
        self.last_command = Some(sanitized.join(" "));
    }
}

impl std::fmt::Debug for State {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("State")
            .field("backend_url", &self.backend_url)
            .field("access_token", &self.access_token.as_ref().map(|_| "[REDACTED]"))
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "[REDACTED]"))
            .field("token_expires_at", &self.token_expires_at)
            .field("org_id", &self.org_id)
            .field("identity_id", &self.identity_id)
            .field("services", &self.services)
            .field("last_session_id", &self.last_session_id)
            .field("last_command", &self.last_command)
            .finish()
    }
}

pub fn default_home() -> PathBuf {
    std::env::var_os("SB_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("SILICON_HOME").map(|home| PathBuf::from(home).join(".silicon-browser")))
        .or_else(|| dirs::home_dir().map(|home| home.join(".silicon-browser")))
        .unwrap_or_else(|| PathBuf::from(".silicon-browser"))
}

/// Partition credentials, refresh locks, and activity by the normalized API base URL.
/// Legacy credentials are copied once, only into their own issuer's partition. The legacy
/// file is retained for rollback; an existing partition always wins, including a signed-out one.
pub fn home_for_backend(root: &Path, backend: &str) -> io::Result<PathBuf> {
    let normalize = |value: &str| silicon_browser::normalize_backend_url(value).map_err(io::Error::other);
    let backend = normalize(backend)?;
    let hash: String = Sha256::digest(backend.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect();
    secure_home(root, true)?;
    let partitions = root.join("backends");
    secure_home(&partitions, true)?;
    let home = partitions.join(hash);
    secure_home(&home, true)?;
    let lock = open_lock_file(&root.join("migration.lock"))?;
    lock.lock_exclusive()?;
    let result = (|| {
        if fs::symlink_metadata(home.join(STATE_FILE)).is_ok() {
            let existing = State::load(&home)?;
            if normalize(&existing.backend_url)? != backend {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "stored credentials belong to another backend"));
            }
        } else {
            let mut initial = State { backend_url: backend.clone(), ..State::default() };
            if fs::symlink_metadata(root.join(STATE_FILE)).is_ok() {
                let legacy = State::load(root)?;
                if normalize(&legacy.backend_url).is_ok_and(|value| value == backend) {
                    initial = legacy;
                    initial.backend_url.clone_from(&backend);
                }
            }
            // Use the partition's ordinary state lock too, so migration cannot race setup.
            State::update(&home, |state| *state = initial)?;
        }
        Ok(home)
    })();
    let _ = FileExt::unlock(&lock);
    result
}

/// A distinct lock keeps refresh-token rotation single-flight across simultaneous CLI calls.
pub fn refresh_lock(home: &Path) -> io::Result<File> {
    secure_home(home, true)?;
    let file = open_lock_file(&home.join("refresh.lock"))?;
    file.lock_exclusive()?;
    Ok(file)
}

fn default_backend() -> String {
    "https://backend.browser.teamofsilicons.com".into()
}

pub(crate) fn secure_home(path: &Path, create: bool) -> io::Result<File> {
    if path.as_os_str().is_empty()
        || path.file_name().is_none()
        || matches!(path.file_name(), Some(name) if name == "." || name == "..")
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "SB_HOME must name a dedicated directory"));
    }
    match open_directory_no_follow(path) {
        Ok(directory) => return validate_home_directory(directory),
        Err(error) if error.kind() != io::ErrorKind::NotFound || !create => return Err(error),
        Err(_) => {}
    }
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    validate_home_directory(open_directory_no_follow(path)?)
}

fn open_directory_no_follow(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW);
    }
    #[cfg(not(unix))]
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "SB_HOME must not be a symlink"));
    }
    options.open(path)
}

fn validate_home_directory(directory: File) -> io::Result<File> {
    let metadata = directory.metadata()?;
    if !metadata.is_dir() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "SB_HOME must be a directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "SB_HOME must be owned by the current user"));
        }
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "SB_HOME permissions must be 0700 or stricter",
            ));
        }
    }
    Ok(directory)
}

pub(crate) fn open_existing_state(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    add_no_follow_flags(&mut options);
    let file = options.open(path)?;
    validate_private_regular_file(&file, "state file")?;
    Ok(file)
}

pub(crate) fn open_lock_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    add_no_follow_flags(&mut options);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    validate_private_regular_file(&file, "lock file")?;
    Ok(file)
}

fn create_temporary_state(home: &Path) -> io::Result<(PathBuf, File)> {
    for _ in 0..4 {
        let path = home.join(format!(".{STATE_FILE}.{}.tmp", uuid::Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.create_new(true).read(true).write(true);
        add_no_follow_flags(&mut options);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => {
                validate_private_regular_file(&file, "temporary state file")?;
                return Ok((path, file));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "could not allocate a unique state file"))
}

pub(crate) fn reject_symlink_or_special_target(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "state target must be a regular non-symlink file"))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub(crate) fn add_no_follow_flags(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
}

pub(crate) fn validate_private_regular_file(file: &File, label: &str) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{label} must be a regular file")));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.nlink() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{label} must be owned by the current user and not hard-linked"),
            ));
        }
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{label} must be owner-only")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test group: issuer partitioning never sends a different server's saved credentials.
    #[test]
    fn backend_partitions_normalize_and_migrate_only_the_matching_issuer() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("sb");
        State::update(&root, |state| {
            state.backend_url = "https://ONE.example:443/".into();
            state.set_tokens("oat_legacy".into(), Some("ort_legacy".into()), None);
        })
        .unwrap();
        let other = home_for_backend(&root, "https://two.example").unwrap();
        assert!(State::load(&other).unwrap().stored_token().is_none());
        let matching = home_for_backend(&root, "https://one.example").unwrap();
        assert_ne!(other, matching);
        assert_eq!(matching, home_for_backend(&root, "https://ONE.example:443/").unwrap());
        assert_eq!(State::load(&matching).unwrap().stored_token(), Some("oat_legacy"));
        State::update(&matching, |state| {
            state.access_token = None;
            state.refresh_token = None;
        })
        .unwrap();
        home_for_backend(&root, "https://one.example").unwrap();
        assert!(State::load(&matching).unwrap().stored_token().is_none());
        assert!(home_for_backend(&root, "https://user:secret@one.example").is_err());
        assert!(home_for_backend(&root, "http://public.example").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn backend_partition_refuses_a_symlinked_parent() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("sb");
        secure_home(&root, true).unwrap();
        std::os::unix::fs::symlink(directory.path(), root.join("backends")).unwrap();
        assert!(home_for_backend(&root, "https://one.example").is_err());
    }

    /// Test group: CLI state is atomic, reloadable, and owner-only.
    #[test]
    fn state_round_trips_without_broad_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("sb");
        let mut state = State { backend_url: "http://127.0.0.1:8080".into(), ..State::default() };
        state.set_tokens("oat_secret".into(), Some("ort_secret".into()), None);
        state.org_id = Some("tos".into());
        State::update(&home, |saved| *saved = state.clone()).unwrap();
        let loaded = State::load(&home).unwrap();
        assert_eq!(loaded.stored_token(), Some("oat_secret"));
        assert_eq!(loaded.refresh_token(), Some("ort_secret"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(home.join(STATE_FILE)).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    /// Test group: last-command state cannot become an accidental credential store.
    #[test]
    fn obvious_secret_arguments_are_redacted() {
        let mut state = State::default();
        state.remember(&["run".into(), "s1".into(), "fill @e1 hello".into(), "--token=secret".into()]);
        let remembered = state.last_command.unwrap();
        assert!(remembered.contains("[REDACTED]"));
        assert!(!remembered.contains("secret"));
    }

    /// Test group: activity writes merge with credentials rotated by another process.
    #[test]
    fn activity_does_not_clobber_concurrently_refreshed_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("sb");
        State::update(&home, |state| {
            state.set_tokens("oat_old".into(), Some("ort_old".into()), Some("2026-09-04T00:00:00Z".into()));
            state.last_session_id = Some("session-old".into());
        })
        .unwrap();

        let mut stale = State::load(&home).unwrap();
        stale.remember(&["profile".into(), "ls".into()]);
        State::update(&home, |state| {
            state.set_tokens("oat_new".into(), Some("ort_new".into()), Some("2026-09-04T01:00:00Z".into()));
        })
        .unwrap();
        State::record_activity(&home, stale.last_command.unwrap(), None).unwrap();

        let merged = State::load(&home).unwrap();
        assert_eq!(merged.stored_token(), Some("oat_new"));
        assert_eq!(merged.refresh_token(), Some("ort_new"));
        assert_eq!(merged.last_session_id.as_deref(), Some("session-old"));
        assert_eq!(merged.last_command.as_deref(), Some("profile ls"));
    }

    /// Test group: a top-level override before `run` cannot make the remembered command retain
    /// browser command content or an endpoint that may contain credentials.
    #[test]
    fn run_redaction_handles_top_level_options() {
        let mut state = State::default();
        state.remember(&[
            "--backend=https://user:secret@example.test".into(),
            "run".into(),
            "s1".into(),
            "fill @e1 very-secret-value".into(),
        ]);
        assert_eq!(state.last_command.as_deref(), Some("run s1 [REDACTED]"));
    }

    /// Test group: no state operation follows an attacker-controlled SB_HOME symlink.
    #[cfg(unix)]
    #[test]
    fn symlinked_home_is_rejected() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let directory = tempfile::tempdir().unwrap();
        let real_home = directory.path().join("real-home");
        fs::create_dir(&real_home).unwrap();
        fs::set_permissions(&real_home, fs::Permissions::from_mode(0o700)).unwrap();
        let linked_home = directory.path().join("linked-home");
        symlink(&real_home, &linked_home).unwrap();
        assert!(State::load(&linked_home).is_err());
        assert!(State::update(&linked_home, |_| {}).is_err());
        assert!(!real_home.join(STATE_FILE).exists());
    }

    /// Test group: state and lock files are opened without following symlinks, leaving the
    /// symlink target byte-for-byte untouched.
    #[cfg(unix)]
    #[test]
    fn symlinked_state_and_lock_files_cannot_be_clobbered() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("sb");
        State::update(&home, |state| state.org_id = Some("safe-org".into())).unwrap();
        let victim = directory.path().join("victim");
        fs::write(&victim, b"do not change").unwrap();

        fs::remove_file(home.join(STATE_FILE)).unwrap();
        symlink(&victim, home.join(STATE_FILE)).unwrap();
        assert!(State::load(&home).is_err());
        assert!(State::update(&home, |_| {}).is_err());
        assert_eq!(fs::read(&victim).unwrap(), b"do not change");

        fs::remove_file(home.join(STATE_FILE)).unwrap();
        fs::remove_file(home.join("state.lock")).unwrap();
        symlink(&victim, home.join("state.lock")).unwrap();
        assert!(State::update(&home, |_| {}).is_err());
        assert_eq!(fs::read(&victim).unwrap(), b"do not change");
    }

    /// Test group: refresh serialization uses the same no-follow lock policy as state updates.
    #[cfg(unix)]
    #[test]
    fn symlinked_refresh_lock_is_rejected() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("sb");
        State::update(&home, |_| {}).unwrap();
        let victim = directory.path().join("victim");
        fs::write(&victim, b"do not change").unwrap();
        symlink(&victim, home.join("refresh.lock")).unwrap();
        assert!(refresh_lock(&home).is_err());
        assert_eq!(fs::read(&victim).unwrap(), b"do not change");
    }
}
