//! Invitations, anonymous links, and the complete 365-day audit log.

use crate::{AccessRight, Client, ContentStream, IdempotencyKey, Result};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A current member, a verified contact, or a dynamic IAM tag.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum Recipient {
    /// Canonical Carbon ID.
    Carbon(String),
    /// Canonical Silicon ID.
    Silicon(String),
    /// Verified address belonging to a current member.
    Email(String),
    /// IAM tag ID or name.
    Tag(String),
}

/// Independently grant read, folder creation, and content updates.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Invite {
    /// Invitation recipient.
    pub principal: Recipient,
    /// Read is always included; delete cannot be granted.
    pub access: Vec<AccessRight>,
    /// Whether the grant applies to descendants.
    pub inherit: bool,
    /// Makes this a read-only expiring share that ends after 1 to 43,200 minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_minutes: Option<u32>,
}

/// A current explicit invitation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Invitation {
    /// Grant identifier for later revocation.
    pub id: Uuid,
    /// Recipient and access policy.
    #[serde(flatten)]
    pub invitation: Invite,
    /// When this expiring share ends (RFC 3339); absent for a permanent grant.
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// How to change a live expiring share.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpiryChange {
    /// Restart the share's clock: it now ends this many minutes (1 to 43,200)
    /// from now, which extends or shortens it.
    ExpireIn(u32),
    /// Keep the access for good. When the recipient already holds a permanent
    /// grant on the entry, the share folds into it.
    Permanent,
}

impl Serialize for ExpiryChange {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::ExpireIn(minutes) => {
                serde_json::json!({ "expires_in_minutes": minutes }).serialize(serializer)
            }
            Self::Permanent => serde_json::json!({ "permanent": true }).serialize(serializer),
        }
    }
}

/// Invitations on an entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvitationPage {
    /// Current grants.
    pub items: Vec<Invitation>,
    /// Continue with this opaque cursor.
    pub next_cursor: Option<String>,
}

/// Dynamic anyone-with-link visibility.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LinkAccess {
    /// Whether this authenticated caller can change this entry's explicit policy.
    pub can_manage: bool,
    /// This entry's own setting.
    pub enabled: bool,
    /// Whether this entry is currently public through any ancestor.
    pub effective: bool,
    /// Nearest shared ancestor, when inherited.
    pub inherited_from: Option<Uuid>,
    /// Shareable file or folder website URL when access is effective.
    /// Absent on older servers or when link access is disabled.
    #[serde(default)]
    pub url: Option<url::Url>,
    /// When this entry's own expiring link ends (RFC 3339); absent for a
    /// permanent link or when this entry's own setting is off.
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// A retained, attributable audit event.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogEvent {
    /// Stable event ID.
    pub id: Uuid,
    /// Carbon or Silicon.
    pub actor_type: String,
    /// Acting member ID.
    pub actor_id: String,
    /// Calling app for delegated operations.
    pub app_id: Option<String>,
    /// Versioned action name.
    pub action: String,
    /// Action-specific details.
    pub metadata: serde_json::Value,
    /// RFC3339 event timestamp.
    pub occurred_at: String,
}

/// A bounded page of the preceding 365 days of events.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogPage {
    /// Authorized events, newest first.
    pub items: Vec<LogEvent>,
    /// Continue from this opaque cursor.
    pub next_cursor: Option<String>,
}

/// Safe metadata available without an IAM login.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublicEntry {
    /// Stable entry identifier.
    pub id: Uuid,
    /// Display name.
    pub name: String,
    /// Organization-relative path.
    pub path: String,
    /// File or folder.
    pub entry_type: String,
    /// File media type.
    pub content_type: Option<String>,
    /// File length in bytes.
    pub size: Option<u64>,
}

/// One page of a publicly shared folder.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublicPage {
    /// Children of the shared folder.
    pub items: Vec<PublicEntry>,
    /// Continue with this cursor.
    pub next_cursor: Option<String>,
}

impl Client {
    /// Reads explicit member and tag invitations.
    /// # Errors
    /// Returns access or transport errors.
    pub async fn invitations(&self, id: Uuid, cursor: Option<&str>) -> Result<InvitationPage> {
        let mut request = self
            .request(
                Method::GET,
                self.api_url(&["entries", &id.to_string(), "invitations"])?,
            )
            .timeout(self.request_timeout());
        if let Some(cursor) = cursor {
            request = request.query(&[("cursor", cursor)]);
        }
        self.receive_json(request).await
    }

    /// Invites a member, email contact, or tag with independent rights.
    /// # Errors
    /// Returns validation, permission, idempotency, or transport errors.
    pub async fn invite(
        &self,
        id: Uuid,
        invite: &Invite,
        key: &IdempotencyKey,
    ) -> Result<Invitation> {
        self.receive_json(
            self.request(
                Method::POST,
                self.api_url(&["entries", &id.to_string(), "invitations"])?,
            )
            .header("idempotency-key", key.as_str())
            .json(invite)
            .timeout(self.request_timeout()),
        )
        .await
    }

    /// Revokes either a member or tag invitation.
    /// # Errors
    /// Returns access, idempotency, or transport errors.
    pub async fn revoke_invitation(
        &self,
        id: Uuid,
        grant: Uuid,
        key: &IdempotencyKey,
    ) -> Result<()> {
        self.receive_empty(
            self.request(
                Method::DELETE,
                self.api_url(&[
                    "entries",
                    &id.to_string(),
                    "invitations",
                    &grant.to_string(),
                ])?,
            )
            .header("idempotency-key", key.as_str())
            .timeout(self.request_timeout()),
        )
        .await
    }

    /// Extends, shortens, or makes permanent a live expiring share.
    /// # Errors
    /// Returns not-found for an expired or unknown share, `not_an_expiring_share`
    /// for a permanent grant, and access, idempotency, or transport errors.
    pub async fn change_expiring_share(
        &self,
        id: Uuid,
        grant: Uuid,
        change: ExpiryChange,
        key: &IdempotencyKey,
    ) -> Result<Invitation> {
        self.receive_json(
            self.request(
                Method::PATCH,
                self.api_url(&[
                    "entries",
                    &id.to_string(),
                    "invitations",
                    &grant.to_string(),
                ])?,
            )
            .header("idempotency-key", key.as_str())
            .json(&change)
            .timeout(self.request_timeout()),
        )
        .await
    }

    /// Reads the explicit and inherited link setting.
    /// # Errors
    /// Returns visibility or transport errors.
    pub async fn link_access(&self, id: Uuid) -> Result<LinkAccess> {
        self.receive_json(
            self.request(
                Method::GET,
                self.api_url(&["entries", &id.to_string(), "link-access"])?,
            )
            .timeout(self.request_timeout()),
        )
        .await
    }

    /// Sets read/download access for anyone with the link.
    /// # Errors
    /// Returns protected-folder, authorization, idempotency, or transport errors.
    pub async fn set_link_access(
        &self,
        id: Uuid,
        enabled: bool,
        key: &IdempotencyKey,
    ) -> Result<LinkAccess> {
        self.receive_json(
            self.request(
                Method::PUT,
                self.api_url(&["entries", &id.to_string(), "link-access"])?,
            )
            .header("idempotency-key", key.as_str())
            .json(&serde_json::json!({"enabled":enabled}))
            .timeout(self.request_timeout()),
        )
        .await
    }

    /// Turns on an expiring link: anyone with the link can view and download for
    /// `minutes` (1 to 43,200), then it stops working. Calling it again on a
    /// live expiring link restarts the clock; [`Self::set_link_access`] with
    /// `true` makes it permanent and with `false` ends it.
    /// # Errors
    /// Returns `link_already_permanent` when a permanent link is already on,
    /// and protected-folder, authorization, idempotency, or transport errors.
    pub async fn set_expiring_link_access(
        &self,
        id: Uuid,
        minutes: u32,
        key: &IdempotencyKey,
    ) -> Result<LinkAccess> {
        self.receive_json(
            self.request(
                Method::PUT,
                self.api_url(&["entries", &id.to_string(), "link-access"])?,
            )
            .header("idempotency-key", key.as_str())
            .json(&serde_json::json!({"enabled":true,"expires_in_minutes":minutes}))
            .timeout(self.request_timeout()),
        )
        .await
    }

    /// Reads one page of retained file/folder audit events.
    /// # Errors
    /// Returns visibility, cursor, or transport errors.
    pub async fn logs(&self, id: Uuid, cursor: Option<&str>) -> Result<LogPage> {
        let mut request = self
            .request(
                Method::GET,
                self.api_url(&["entries", &id.to_string(), "logs"])?,
            )
            .timeout(self.request_timeout());
        if let Some(cursor) = cursor {
            request = request.query(&[("cursor", cursor)]);
        }
        self.receive_json(request).await
    }

    fn public_url(&self, org: &str, path: &str) -> Result<url::Url> {
        let mut parts = vec!["public", org];
        parts.extend(path.trim_start_matches('/').split('/'));
        let mut url = self.api_url(&parts)?;
        if let Some(environment) = self.config().public_environment {
            url.query_pairs_mut()
                .append_pair("test_environment", &environment.to_string());
        }
        Ok(url)
    }

    /// Resolves a shared file or folder without sending the configured bearer.
    /// # Errors
    /// Returns not-found when link access is absent or revoked.
    pub async fn public_entry(&self, org: &str, path: &str) -> Result<PublicEntry> {
        self.receive_json(
            self.anonymous_request(Method::GET, self.public_url(org, path)?)
                .timeout(self.request_timeout()),
        )
        .await
    }

    /// Lists a publicly shared folder, without authentication.
    /// # Errors
    /// Returns visibility, cursor, or transport errors.
    pub async fn public_children(
        &self,
        org: &str,
        path: &str,
        cursor: Option<&str>,
    ) -> Result<PublicPage> {
        let mut request = self
            .anonymous_request(Method::GET, self.public_url(org, path)?)
            .query(&[("view", "contents")])
            .timeout(self.request_timeout());
        if let Some(cursor) = cursor {
            request = request.query(&[("cursor", cursor)]);
        }
        self.receive_json(request).await
    }

    /// Streams public file bytes or a folder's tar.zst archive.
    /// # Errors
    /// Returns visibility or transport errors, including errors during streaming.
    pub async fn public_download(&self, org: &str, path: &str) -> Result<ContentStream> {
        self.open_public_content(org, path, "attachment", None)
            .await
    }

    /// Streams a publicly shared file, optionally reading one byte range.
    /// # Errors
    /// Returns not-found, invalid-range, or transport errors.
    pub async fn public_content(
        &self,
        org: &str,
        path: &str,
        range: Option<crate::ByteRange>,
    ) -> Result<ContentStream> {
        self.open_public_content(org, path, "inline", range).await
    }

    async fn open_public_content(
        &self,
        org: &str,
        path: &str,
        view: &str,
        range: Option<crate::ByteRange>,
    ) -> Result<ContentStream> {
        let mut request = self
            .anonymous_request(Method::GET, self.public_url(org, path)?)
            .query(&[("view", view)])
            .timeout(self.transfer_timeout());
        if let Some(range) = range {
            request = request.header(reqwest::header::RANGE, range.header_value());
        }
        Ok(ContentStream::new(self.receive(request).await?))
    }
}
