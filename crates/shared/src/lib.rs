//! Stable domain and wire contracts shared by every Silicon Browser component.
//!
//! Identifiers deliberately remain strings: IAM, the browser provider, and
//! Briefcase own their formats. Validation here checks only guarantees Silicon
//! Browser itself can safely enforce.

mod access;
mod delivery;
mod error;
mod filter;
mod model;
mod validation;

pub use access::AccessList;
pub use delivery::*;
pub use error::{ApiError, ApiErrorEnvelope, Envelope, FieldError};
pub use filter::{
    FilterError, RecordingFacet, RecordingFilter, RecordingPredicate, SessionFacet, SessionFilter, SessionPredicate,
    TextPattern, UsageFilter, UsagePredicate,
};
pub use model::*;
pub use validation::{Validate, ValidationError};

pub type IdentityId = String;
pub type OrgId = String;
pub type ProfileId = String;
pub type SessionId = String;
pub type RecordingId = String;
pub type RequestId = String;

/// The native local controller version installed by setup and used by the client.
pub const AGENT_BROWSER_VERSION: &str = "0.36.0";
