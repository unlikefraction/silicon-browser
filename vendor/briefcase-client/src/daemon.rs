//! Stateless control of a local Briefcase daemon. Callers own runtime paths;
//! these functions do not discover homes, save credentials, or start services.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// A command sent over the daemon's private local control socket.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    /// Register an existing, absolute Briefcase state directory with the daemon.
    Register {
        /// Private state directory; credentials are never included in IPC.
        state: PathBuf,
    },
    /// Read the daemon's live process status and registered directories.
    Status,
    /// Ask the daemon to stop after any active installer completes.
    Stop,
}

/// Local control errors are independent of the backend's HTTP errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The daemon socket is unavailable or cannot be read or written.
    #[error("cannot contact the Briefcase daemon: {0}")]
    Io(#[from] std::io::Error),
    /// The request or response did not follow the control protocol.
    #[error("invalid daemon response: {0}")]
    Protocol(String),
    /// The daemon did not answer before the local control deadline.
    #[error("daemon did not respond within three seconds")]
    Timeout,
    /// This platform has no supported local control transport.
    #[error("the shared daemon currently requires macOS or Linux")]
    Unsupported,
}

/// Sends one command to a caller-selected daemon runtime directory.
///
/// # Errors
/// Returns an error for an absent daemon, refused registration, timeout, or
/// malformed response. It never starts a daemon or falls back to another path.
#[cfg(unix)]
pub async fn request(runtime: &Path, request: &Request) -> Result<Value, Error> {
    use tokio::{
        io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader},
        net::UnixStream,
    };
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        let mut stream = UnixStream::connect(runtime.join("control.sock")).await?;
        let mut body =
            serde_json::to_vec(request).map_err(|error| Error::Protocol(error.to_string()))?;
        if body.len() >= 8192 {
            return Err(Error::Protocol("control request exceeds 8192 bytes".into()));
        }
        body.push(b'\n');
        stream.write_all(&body).await?;
        let mut response = String::new();
        BufReader::new(stream)
            .take(1_048_577)
            .read_line(&mut response)
            .await?;
        if response.len() > 1_048_576 || !response.ends_with('\n') {
            return Err(Error::Protocol(
                "oversized or incomplete control response".into(),
            ));
        }
        let value: Value =
            serde_json::from_str(&response).map_err(|error| Error::Protocol(error.to_string()))?;
        if let Some(error) = value.get("error").and_then(Value::as_str) {
            return Err(Error::Protocol(error.into()));
        }
        Ok(value)
    })
    .await
    .map_err(|_| Error::Timeout)?
}

/// Local daemon control is currently supported on Unix platforms.
///
/// # Errors
/// Always returns [`Error::Unsupported`] on other platforms.
#[cfg(not(unix))]
pub async fn request(_runtime: &Path, _request: &Request) -> Result<Value, Error> {
    Err(Error::Unsupported)
}
