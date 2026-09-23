//! The caller-facing surface, grouped the way the contract groups it.
//!
//! Each group is reached from the [`Client`] -- `client.tags()`,
//! `client.silicons()` -- and borrows it, so obtaining one costs nothing and a
//! client can be shared freely.
//!
//! Only actions a caller performs are here. The platform-administration
//! routes, the inbound provider webhooks and the browser consent screens are
//! deliberately absent: they belong to the operator, to the provider and to
//! the browser respectively, not to an API caller.

pub mod app_verification;
pub mod application_mutations;
pub mod application_reads;
pub mod application_scopes;
pub mod applications;
pub mod auth;
pub mod bundles;
pub mod carbons;
pub mod environments;
pub mod governance;
pub mod invitations;
pub mod members;
pub mod oauth;
pub mod obo;
pub mod organizations;
pub mod signup;
pub mod silicons;
pub mod sso;
pub mod system;
pub mod tags;
pub mod trust;

use crate::Client;

impl Client {
    /// Version and readiness of the service itself.
    #[must_use]
    pub const fn system(&self) -> system::System<'_> {
        system::System(self)
    }

    /// Creating a Carbon: contact verification, then the account.
    #[must_use]
    pub const fn signup(&self) -> signup::Signup<'_> {
        signup::Signup(self)
    }

    /// Direct IAM session maintenance, logout, and step-up.
    ///
    /// Application login is available only as [`oauth::OAuth::login`] and
    /// accepts only a short-lived token.
    #[must_use]
    pub const fn auth(&self) -> auth::Auth<'_> {
        auth::Auth(self)
    }

    /// The signed-in Carbon: profile, sessions, history, and directory lookup.
    #[must_use]
    pub const fn carbons(&self) -> carbons::Carbons<'_> {
        carbons::Carbons(self)
    }

    /// Organizations the caller belongs to or is creating.
    #[must_use]
    pub const fn organizations(&self) -> organizations::Organizations<'_> {
        organizations::Organizations(self)
    }

    /// Members of an organization, and the directory view of them.
    #[must_use]
    pub const fn members(&self) -> members::Members<'_> {
        members::Members(self)
    }

    /// Inviting Carbons, and joining on an invitation.
    #[must_use]
    pub const fn invitations(&self) -> invitations::Invitations<'_> {
        invitations::Invitations(self)
    }

    /// Organization tags.
    #[must_use]
    pub const fn tags(&self) -> tags::Tags<'_> {
        tags::Tags(self)
    }

    /// Advisory trust: the default, the rules, and what they evaluate to.
    #[must_use]
    pub const fn trust(&self) -> trust::Trust<'_> {
        trust::Trust(self)
    }

    /// Role and tag changes that need approval, and the history they leave.
    #[must_use]
    pub const fn governance(&self) -> governance::Governance<'_> {
        governance::Governance(self)
    }

    /// Silicons, their credentials, and their webhooks.
    #[must_use]
    pub const fn silicons(&self) -> silicons::Silicons<'_> {
        silicons::Silicons(self)
    }

    /// Applications, their public base URLs, credentials, OBO surface, and webhooks.
    #[must_use]
    pub const fn applications(&self) -> applications::Applications<'_> {
        applications::Applications(self)
    }

    /// Short-lived application identity keys for inter-application calls.
    #[must_use]
    pub const fn app_verification(&self) -> app_verification::AppVerification<'_> {
        app_verification::AppVerification(self)
    }

    /// The OAuth endpoints an application calls as itself.
    #[must_use]
    pub const fn oauth(&self) -> oauth::OAuth<'_> {
        oauth::OAuth(self)
    }

    /// Scope-projected IAM data for an application's user access token.
    #[must_use]
    pub const fn application_reads(&self) -> application_reads::ApplicationReads<'_> {
        application_reads::ApplicationReads(self)
    }

    /// Scope-projected IAM mutation receipts for an application's user token.
    ///
    /// Use these methods when write scopes do not include the corresponding read
    /// permissions. They preserve omitted fields instead of decoding as a full
    /// first-party resource.
    #[must_use]
    pub const fn application_mutations(&self) -> application_mutations::ApplicationMutations<'_> {
        application_mutations::ApplicationMutations(self)
    }

    /// Application scope catalogs, approval requests, and their discussions.
    #[must_use]
    pub const fn application_scopes(&self) -> application_scopes::ApplicationScopes<'_> {
        application_scopes::ApplicationScopes(self)
    }

    /// Named groups of applications that share a login journey.
    #[must_use]
    pub const fn bundles(&self) -> bundles::Bundles<'_> {
        bundles::Bundles(self)
    }

    /// Delegated access between declared applications across organizations.
    #[must_use]
    pub const fn obo(&self) -> obo::Obo<'_> {
        obo::Obo(self)
    }

    /// An organization's SSO configuration.
    #[must_use]
    pub const fn sso(&self) -> sso::Sso<'_> {
        sso::Sso(self)
    }

    /// Testing environments: disposable replicas of the whole service.
    #[must_use]
    pub const fn environments(&self) -> environments::Environments<'_> {
        environments::Environments(self)
    }
}
