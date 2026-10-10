//! Read historical upload receipts without contacting the retired delivery service.
use serde::{Deserialize, Serialize};
use uuid::Uuid;
/// Metadata resolved after a committed upload; links require Briefcase authentication.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct BriefcaseEntry {
    pub id: Uuid,
    pub org_id: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub name: String,
    pub path: String,
    pub content_type: Option<String>,
    pub size: u64,
    pub permanent_url: String,
    pub origin_app_id: Option<String>,
}
