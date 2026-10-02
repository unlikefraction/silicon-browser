//! Browsing, folders, and the recoverable bin.

use reqwest::Method;
use serde::Serialize;
use uuid::Uuid;

use crate::{
    client::{Client, IdempotencyKey, json_body},
    error::Result,
    models::{ActivityEvent, ActivityPage, Entry, EntryPage, RootType, SearchPage, SearchResult},
    requests::{Destination, EntryUpdate, ListEntries, NewFolder, NewGrant},
};

#[derive(Serialize)]
struct WireFolder<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_path: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    root_type: Option<RootType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<&'a str>,
    invitees: &'a [NewGrant],
}

impl Client {
    /// Lists a folder's contents, or filters everything the caller can reach.
    ///
    /// Entries the caller may not see are already gone from the answer, and a
    /// page is refilled rather than answered short, so a full page means what
    /// it says. Follow `next_cursor` until it is absent to walk everything.
    ///
    /// # Errors
    ///
    /// Returns an error when the folder is not visible, the filter cannot be
    /// parsed, or the deployment cannot be reached.
    pub async fn list_entries(&self, query: &ListEntries) -> Result<EntryPage> {
        let mut url = self.api_url(&["entries"])?;
        {
            let mut pairs = url.query_pairs_mut();
            match &query.parent {
                Some(Destination::Id(id)) => {
                    pairs.append_pair("parent_id", &id.to_string());
                }
                Some(Destination::Path(path)) => {
                    pairs.append_pair("path", path);
                }
                None => {}
            }
            if let Some(filter) = &query.filter {
                pairs.append_pair("filter", filter);
            }
            if let Some(cursor) = &query.cursor {
                pairs.append_pair("cursor", cursor);
            }
            if let Some(limit) = query.limit {
                pairs.append_pair("limit", &limit.to_string());
            }
        }
        let request = self
            .request(Method::GET, url)
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }

    /// Walks every page of a listing, gathering the entries.
    ///
    /// A convenience over [`Client::list_entries`] for callers that want the
    /// whole folder rather than one page. `max_entries` bounds the walk so a
    /// very large folder cannot surprise the caller.
    ///
    /// # Errors
    ///
    /// Returns the first error any page returns.
    pub async fn list_all_entries(
        &self,
        query: &ListEntries,
        max_entries: usize,
    ) -> Result<Vec<Entry>> {
        let mut gathered = Vec::new();
        let mut page_query = query.clone();
        loop {
            let page = self.list_entries(&page_query).await?;
            gathered.extend(page.items);
            let Some(cursor) = page.next_cursor.filter(|_| gathered.len() < max_entries) else {
                break;
            };
            page_query.cursor = Some(cursor);
        }
        gathered.truncate(max_entries);
        Ok(gathered)
    }

    /// Creates a folder.
    ///
    /// At the organization base, `root_type` defines the new folder's access
    /// boundary. The folder is a sibling of Public, Private, and tag containers.
    /// Specify a parent explicitly to create inside one of those containers.
    ///
    /// # Errors
    ///
    /// Returns an error when the destination is not writable, a tag the caller
    /// does not carry was named, or an invitee is not a current member.
    pub async fn create_folder(&self, folder: &NewFolder) -> Result<Entry> {
        let (parent_id, parent_path) = match &folder.parent {
            Some(Destination::Id(id)) => (Some(*id), None),
            Some(Destination::Path(path)) => (None, Some(path.as_str())),
            None => (None, None),
        };
        let body = json_body(&WireFolder {
            name: &folder.name,
            parent_id,
            parent_path,
            root_type: folder.root_type,
            tag: folder.tag.as_deref(),
            invitees: &folder.invitees,
        })?;
        let key = folder
            .idempotency_key
            .clone()
            .unwrap_or_else(IdempotencyKey::random);
        let request = self
            .request(Method::POST, self.api_url(&["entries"])?)
            .header("content-type", "application/json")
            .header("idempotency-key", key.as_str())
            .body(body)
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }

    /// Reads one entry's metadata and the caller's access to it.
    ///
    /// # Errors
    ///
    /// Returns a not-found error for an entry that does not exist and for one
    /// the caller may not read; Briefcase does not distinguish them.
    pub async fn entry(&self, entry_id: Uuid) -> Result<Entry> {
        let url = self.api_url(&["entries", &entry_id.to_string()])?;
        let request = self
            .request(Method::GET, url)
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }

    /// Reads one entry by the path its permanent URL shows.
    ///
    /// # Errors
    ///
    /// Returns a not-found error when nothing readable sits at that path.
    pub async fn entry_at(&self, path: &str) -> Result<Entry> {
        let request = self
            .request(Method::GET, self.permanent_url(path)?)
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }

    /// Renames an entry, moves it, or both.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller lacks update access on the entry, or
    /// write access on the destination folder.
    pub async fn update_entry(&self, entry_id: Uuid, update: &EntryUpdate) -> Result<Entry> {
        self.update_entry_with_key(entry_id, update, &IdempotencyKey::random())
            .await
    }

    /// Renames or moves an entry with a caller-owned retry identity.
    ///
    /// Persist `idempotency_key` before the first attempt and reuse it after an
    /// uncertain transport failure so the mutation cannot be applied twice.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller lacks update access on the entry, or
    /// write access on the destination folder.
    pub async fn update_entry_with_key(
        &self,
        entry_id: Uuid,
        update: &EntryUpdate,
        idempotency_key: &IdempotencyKey,
    ) -> Result<Entry> {
        let url = self.api_url(&["entries", &entry_id.to_string()])?;
        let body = json_body(update)?;
        let request = self
            .request(Method::PATCH, url)
            .header("content-type", "application/json")
            .header("idempotency-key", idempotency_key.as_str())
            .body(body)
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }

    /// Moves an entry to the bin, where it stays recoverable for 45 days.
    ///
    /// Deleting a folder takes everything inside it, and all of it comes back
    /// together on restore.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller lacks delete access specifically:
    /// being able to change an entry is not enough.
    pub async fn delete_entry(&self, entry_id: Uuid) -> Result<()> {
        let url = self.api_url(&["entries", &entry_id.to_string()])?;
        let request = self
            .request(Method::DELETE, url)
            .timeout(self.request_timeout());
        self.receive_empty(request).await
    }

    /// Keeps a self-destructing file: stops its timer so it is never deleted.
    ///
    /// Only the file's creator, org admins and org owners may keep a file.
    ///
    /// # Errors
    ///
    /// Returns a forbidden error for anyone else who can see the file, a
    /// not-found error when it is not visible, and `not_self_destructing` when
    /// its timer is not running.
    pub async fn make_permanent(&self, entry_id: Uuid) -> Result<()> {
        let url = self.api_url(&["entries", &entry_id.to_string(), "self-destruct"])?;
        let request = self
            .request(Method::DELETE, url)
            .timeout(self.request_timeout());
        self.receive_empty(request).await
    }

    /// Reads the retained "who did what, when" history of one entry.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry is not visible to the caller.
    pub async fn activity(&self, entry_id: Uuid) -> Result<Vec<ActivityEvent>> {
        let url = self.api_url(&["entries", &entry_id.to_string(), "activity"])?;
        let request = self
            .request(Method::GET, url)
            .timeout(self.request_timeout());
        let page: ActivityPage = self.receive_json(request).await?;
        Ok(page.items)
    }

    /// Searches visible filenames and extracted document text.
    ///
    /// # Errors
    ///
    /// Returns an error when the query is empty or the limit is above twenty.
    pub async fn search(&self, query: &str, limit: Option<u8>) -> Result<Vec<SearchResult>> {
        let mut url = self.api_url(&["search"])?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("q", query);
            if let Some(limit) = limit {
                pairs.append_pair("limit", &limit.to_string());
            }
        }
        let request = self
            .request(Method::GET, url)
            .timeout(self.request_timeout());
        let page: SearchPage = self.receive_json(request).await?;
        Ok(page.items)
    }

    /// Lists the caller's recoverable entries, newest deletion first.
    ///
    /// # Errors
    ///
    /// Returns an error when the limit is above a hundred or the cursor is not
    /// one Briefcase issued.
    pub async fn bin(&self, cursor: Option<&str>, limit: Option<u16>) -> Result<EntryPage> {
        let mut url = self.api_url(&["bin"])?;
        {
            let mut pairs = url.query_pairs_mut();
            if let Some(cursor) = cursor {
                pairs.append_pair("cursor", cursor);
            }
            if let Some(limit) = limit {
                pairs.append_pair("limit", &limit.to_string());
            }
        }
        let request = self
            .request(Method::GET, url)
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }

    /// Restores a deleted entry, with everything that was inside it.
    ///
    /// If the original parent is gone, Briefcase puts it in the caller's own
    /// Private folder under a collision-safe name.
    ///
    /// # Errors
    ///
    /// Returns a not-found error once the 45-day window has passed.
    pub async fn restore_from_bin(&self, entry_id: Uuid) -> Result<Entry> {
        self.restore_from_bin_with_key(entry_id, &IdempotencyKey::random())
            .await
    }

    /// Restores a deleted entry with a stable operation identity.
    ///
    /// Keep the same key when recovering an uncertain response. A key belongs
    /// to one deletion cycle; use a new key after the entry is deleted again.
    ///
    /// # Errors
    ///
    /// Returns the API error if the entry cannot be restored, authorization
    /// fails, or the key belongs to a different deletion cycle.
    pub async fn restore_from_bin_with_key(
        &self,
        entry_id: Uuid,
        key: &IdempotencyKey,
    ) -> Result<Entry> {
        let url = self.api_url(&["bin", &entry_id.to_string(), "restore"])?;
        let request = self
            .request(Method::POST, url)
            .header("Idempotency-Key", key.as_str())
            .timeout(self.request_timeout());
        self.receive_json(request).await
    }

    /// Builds the permanent URL for an organization-relative path.
    ///
    /// Each path segment is escaped on its own, so a name may contain anything
    /// a name is allowed to contain without changing which entry is addressed.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Configuration`] when the client's base URL
    /// cannot carry a path.
    pub fn permanent_url(&self, path: &str) -> Result<url::Url> {
        let mut segments = vec!["org".to_owned(), self.organization().to_owned()];
        segments.extend(
            path.trim_matches('/')
                .split('/')
                .filter(|segment| !segment.is_empty())
                .map(ToOwned::to_owned),
        );
        let borrowed: Vec<&str> = segments.iter().map(String::as_str).collect();
        self.origin_url(&borrowed)
    }
}
