//! A Rust client for the Silicon IAM API.
//!
//! Silicon IAM manages identity, organization governance, application login,
//! and delegated access. This crate is the primary way to call it from Rust:
//! everything a caller can do over HTTP has a method here, with the contract's
//! own types.
//!
//! # Runtime state stays with the caller
//!
//! [`Client`] does not store sessions, cache API responses, or refresh
//! credentials behind your back. An expired token produces an error, and
//! deciding what to do about that -- refresh, re-authenticate, give up --
//! stays with the caller, because only the caller knows where its credentials
//! live and who is allowed to prompt for new ones.
//!
//! Anything that must be remembered between calls belongs to the program
//! holding the client. The `silicon-iam-cli` crate is one such program: it
//! stores tokens under `~/.silicon-iam/` and refreshes them, using nothing but
//! this crate to talk to the service.
//!
//! Dependency versions follow the consuming project's Cargo configuration.
//! The library never runs Cargo, checks for updates during API requests, or
//! changes a lockfile. Honeycomb manages installed IAM CLI releases.
//!
//! # Getting started
//!
//! An Application never collects a Carbon's email/SMS code or any other IAM
//! authentication credential. Send the user agent to the IAM-hosted login
//! page, receive its single-use short-lived token, and give only that `slt` to
//! [`api::oauth::OAuth::login`]:
//!
//! ```no_run
//! use silicon_iam_client::{Client, Credential, Mutation};
//!
//! # async fn run() -> silicon_iam_client::Result<()> {
//! let app_id = "checkout";
//! let application = Client::new("https://backend.iam.teamofsilicons.com")?
//!     .with_credential(Credential::application(app_id, "ask_example"));
//! let tokens = application
//!     .oauth()
//!     .login(app_id, "slt_from_the_iam_callback", &Mutation::new())
//!     .await?;
//! # let _ = tokens;
//! # Ok(())
//! # }
//! ```
//!
//! # Idempotency
//!
//! Ordinary mutating routes require an idempotency key, and this crate makes that
//! explicit: those mutations take a [`Mutation`], which carries one. Application
//! identity key issuance through [`Client::app_verification`] creates a fresh key
//! each time and deliberately has no idempotent replay.
//!
//! The service binds the key to the caller, the route and the exact request
//! body, then replays the original response for a repeat of the same request.
//! That is only useful if a retry presents the *same* key, so build one
//! [`Mutation`] per logical operation and reuse it:
//!
//! ```no_run
//! # use silicon_iam_client::{Client, Mutation, models};
//! # async fn run(client: &Client) -> silicon_iam_client::Result<()> {
//! let creating = Mutation::new();
//! let input = models::OrganizationCreate {
//!     org_id: "acme".to_owned(),
//!     name: "Acme".to_owned(),
//!     logo: None,
//!     description: None,
//! };
//!
//! let organization = match client.organizations().create(&input, &creating).await {
//!     Ok(organization) => organization,
//!     // The same `creating` replays the original outcome instead of
//!     // creating a second organization.
//!     Err(error) if error.is_retryable() => {
//!         client.organizations().create(&input, &creating).await?
//!     }
//!     Err(error) => return Err(error),
//! };
//! # let _ = organization;
//! # Ok(())
//! # }
//! ```
//!
//! # Optimistic concurrency
//!
//! Routes that change an existing resource take its current `version` as an
//! ordinary argument rather than hiding it in options, so it cannot be
//! forgotten. A stale version fails with
//! [`ApiError::is_version_conflict`], which means someone else changed the
//! resource first: re-read it, decide whether your change still applies, and
//! retry with the new version.
//!
//! # Step-up
//!
//! Routes that change authority or reveal a credential additionally require a
//! step-up assertion. Obtain one with [`api::auth::Auth::start_step_up`] and
//! [`api::auth::Auth::verify_step_up`], then attach it with
//! [`Mutation::step_up`]. The service answers `step_up_required` when one is
//! missing, which [`ApiError::requires_step_up`] recognizes.
//!
//! # Testing environments
//!
//! A testing environment is the same API against a separate database, starting
//! empty. Move a client onto one and every other method works unchanged:
//!
//! ```no_run
//! # use silicon_iam_client::{Client, EnvironmentKey};
//! # async fn run(client: &Client, key: &str) -> silicon_iam_client::Result<()> {
//! let sandbox = client.with_environment(EnvironmentKey::new(key)?);
//! // Creates an organization inside the environment, not in production.
//! let organizations = sandbox.organizations().list(&Default::default()).await?;
//! # let _ = organizations;
//! # Ok(())
//! # }
//! ```
//!
//! Environments deliver no email or SMS, and their verification steps accept
//! the fixed code `000000`. Webhooks are delivered inside an explicit `test`
//! envelope that includes the environment key and nests event metadata/data;
//! receivers must verify the exact outer bytes and redact that root key.
//!
//! # Errors
//!
//! [`Error`] separates the service's own answers from everything else.
//! [`Error::Api`] carries the contract's envelope, whose `code` is the stable
//! thing to match on; [`Error::RateLimited`] is held apart because it is the
//! one failure with a mechanical remedy. [`Error::Transport`] means the
//! request never reached a response, which is the case where a retry is
//! reasonable but the outcome is genuinely unknown. [`Error::ResponseTooLarge`]
//! rejects a body above the fixed 4 MiB memory-safety bound and is not
//! retryable because a mutation may already have completed.
//! [`Error::UnstructuredResponse`] preserves an HTTP failure status when no
//! IAM envelope arrived, without retaining raw HTML or mistaking an edge
//! response for an IAM permission decision. Investigate before retrying.
//!
//! # What is not here
//!
//! This crate exposes caller actions only. Platform administration, the
//! inbound provider webhooks, and the browser consent screens are absent: they
//! belong to the operator, to the provider, and to the browser.

#![forbid(unsafe_code)]

pub mod api;
pub mod client;
pub mod credentials;
pub mod error;
pub mod honeycomb;
pub mod models;
mod models_manual;
pub mod request;
pub mod support;
pub mod telemetry;
pub mod update;
pub mod webhook;

pub use client::{API_VERSION, Client, ClientBuilder};
pub use credentials::{Credential, EnvironmentKey};
pub use error::{ApiError, Error, Result};
pub use request::{IdempotencyKey, Mutation, Paging};
pub use webhook::{
    VerifiedWebhook, WebhookError, WebhookSecret, WebhookSecretKeyring, WebhookVerifier,
};

impl Error {
    /// Whether repeating the identical request could plausibly succeed.
    ///
    /// True for the service's own "try again" signals and for a request that
    /// never reached a response. The second case is worth thinking about: the
    /// service may have processed it, which is exactly why a retry should
    /// reuse the original [`Mutation`] rather than build a new one.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Api(error) => error.is_retryable(),
            Self::RateLimited { .. } | Self::Transport(_) => true,
            Self::Decode(_)
            | Self::UnstructuredResponse { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ApiVersionUnsupported { .. }
            | Self::Invalid(_) => false,
        }
    }

    /// The service's envelope, when the failure came from the service.
    #[must_use]
    pub fn api(&self) -> Option<&ApiError> {
        match self {
            Self::Api(error) | Self::RateLimited { source: error, .. } => Some(error),
            _ => None,
        }
    }

    /// The correlation identifier to quote when reporting this failure.
    #[must_use]
    pub fn request_id(&self) -> Option<&str> {
        match self {
            Self::UnstructuredResponse { request_id, .. } => request_id.as_deref(),
            _ => self.api()?.request_id.as_deref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ApiError, Error};

    fn api(status: u16) -> ApiError {
        ApiError {
            status,
            code: "whatever".to_owned(),
            message: "something went wrong".to_owned(),
            details: None,
            request_id: Some("01a0-req".to_owned()),
        }
    }

    #[test]
    fn a_request_that_never_arrived_is_retryable_but_a_bad_one_is_not() {
        assert!(!Error::Api(Box::new(api(422))).is_retryable());
        assert!(Error::Api(Box::new(api(503))).is_retryable());
        assert!(!Error::Invalid("bad input".to_owned()).is_retryable());
        assert!(!Error::Decode("odd shape".to_owned()).is_retryable());
    }

    #[test]
    fn the_request_id_is_reachable_from_either_service_failure() {
        assert_eq!(
            Error::Api(Box::new(api(409))).request_id(),
            Some("01a0-req")
        );
        let limited = Error::RateLimited {
            retry_after: std::time::Duration::from_secs(2),
            limit: Some(60),
            remaining: Some(0),
            source: Box::new(api(429)),
        };
        assert_eq!(limited.request_id(), Some("01a0-req"));
        assert!(limited.is_retryable());
        assert_eq!(Error::Invalid("x".to_owned()).request_id(), None);
    }
}
