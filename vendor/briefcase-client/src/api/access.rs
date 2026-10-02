//! Grants and the notification inbox.

use reqwest::Method;
use uuid::Uuid;

use crate::{
    client::{Client, json_body},
    error::Result,
    models::{NotificationInbox, PermissionGrant, PermissionGrantPage, PermissionInspection},
    requests::{NewGrant, PermissionQuery},
};

impl Client {
    /// Lists the explicit grants on an entry.
    ///
    /// These are the grants somebody made, not every way the caller might have
    /// reached the entry: Public visibility, a tag, ownership, and
    /// administrative authority convey access without a grant.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry is not visible to the caller.
    pub async fn permissions(&self, entry_id: Uuid) -> Result<Vec<PermissionGrant>> {
        let url = self.api_url(&["entries", &entry_id.to_string(), "permissions"])?;
        let request = self
            .request(Method::GET, url)
            .timeout(self.request_timeout());
        let page: PermissionGrantPage = self.receive_json(request).await?;
        Ok(page.items)
    }

    /// Grants a member access to an entry.
    ///
    /// Granting a member who already holds a grant amends it: the rights and
    /// inheritance become exactly what this call names, so widening access
    /// never has to pass through a revocation.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller cannot manage permissions there, or
    /// the principal is not a current member of the organization.
    pub async fn grant(&self, entry_id: Uuid, grant: &NewGrant) -> Result<PermissionGrant> {
        let url = self.api_url(&["entries", &entry_id.to_string(), "permissions"])?;
        let body = json_body(grant)?;
        let request = self
            .request(Method::POST, url)
            .header("content-type", "application/json")
            .body(body)
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }

    /// Revokes one explicit grant.
    ///
    /// Access the member has by another route — a second grant, a tag, Public
    /// visibility, ownership, administration — is untouched.
    ///
    /// # Errors
    ///
    /// Returns a not-found error when the grant is already revoked or never
    /// existed.
    pub async fn revoke(&self, entry_id: Uuid, grant_id: Uuid) -> Result<()> {
        let url = self.api_url(&[
            "entries",
            &entry_id.to_string(),
            "permissions",
            &grant_id.to_string(),
        ])?;
        let request = self
            .request(Method::DELETE, url)
            .timeout(self.request_timeout());
        self.receive_empty(request).await
    }

    /// Reports what the caller may do on up to a hundred named targets.
    ///
    /// A target that does not exist and one the caller cannot read are both
    /// reported as unresolved, so the answer cannot be used to probe.
    ///
    /// # Errors
    ///
    /// Returns an error when no target was named, or more than a hundred were.
    pub async fn effective_access(&self, query: &PermissionQuery) -> Result<PermissionInspection> {
        let url = self.api_url(&["permissions", "effective"])?;
        let body = json_body(query)?;
        let request = self
            .request(Method::POST, url)
            .header("content-type", "application/json")
            .body(body)
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }

    /// Reads the caller's twenty newest notifications and unread count.
    ///
    /// # Errors
    ///
    /// Returns an error when the deployment cannot be reached.
    pub async fn notifications(&self) -> Result<NotificationInbox> {
        let url = self.api_url(&["notifications"])?;
        let request = self
            .request(Method::GET, url)
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }

    /// Marks the whole inbox read and returns it afterwards.
    ///
    /// # Errors
    ///
    /// Returns an error when the deployment cannot be reached.
    pub async fn mark_notifications_read(&self) -> Result<NotificationInbox> {
        let url = self.api_url(&["notifications", "read"])?;
        let request = self
            .request(Method::POST, url)
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }
}
