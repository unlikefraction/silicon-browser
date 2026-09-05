//! Stateless primary interface to Silicon Browser.
//!
//! The [`Client`] owns only explicit configuration and an [`Auth`] value. It never reads CLI
//! state, environment variables, or credentials from disk. The `sb` binary is built entirely on
//! these public methods.

mod api;
mod auth;
pub mod controller;
mod error;
pub mod setup;
mod transport;

pub use api::{Client, normalize_backend_url};
pub use auth::Auth;
pub use error::Error;
pub use silicon_browser_shared as shared;
pub use transport::{HttpTransport, Method, Request, Response, Transport};

use silicon_browser_shared::ApiErrorEnvelope;

pub const DEFAULT_BACKEND_URL: &str = "https://backend.browser.teamofsilicons.com";

fn decode_error(status: u16, body: &[u8]) -> Error {
    serde_json::from_slice::<ApiErrorEnvelope>(body).map(Error::from).unwrap_or_else(|_| {
        Error::Protocol(format!("HTTP {status} carried neither a success nor the documented error envelope"))
    })
}
