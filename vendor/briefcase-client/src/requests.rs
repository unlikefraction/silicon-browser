//! What a caller asks for, in the shapes the operations accept.

use std::path::PathBuf;

use serde::Serialize;
use uuid::Uuid;

use crate::{
    client::IdempotencyKey,
    config::{ApplicationId, IamApplicationSecret, IamEnvironmentKey},
    models::{AccessRight, ActorRef, RootType},
};

/// Creates an empty Briefcase plane coupled to one IAM testing plane.
#[derive(Clone, Debug, Serialize)]
pub struct TestingEnvironmentCreate {
    /// Human-readable environment name.
    pub name: String,
    /// Optional purpose or run description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optionally join an existing IAM dependency testing environment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iam_test_key: Option<IamEnvironmentKey>,
}

impl TestingEnvironmentCreate {
    /// Provisions IAM and Briefcase together with a single request.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            iam_test_key: None,
        }
    }

    /// Adds a description.
    #[must_use]
    pub fn described(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// Atomically replaces every IAM credential paired with a Briefcase test plane.
#[derive(Clone, Debug, Serialize)]
pub struct TestingEnvironmentIamPairing {
    /// Public UUID of the replacement IAM testing environment.
    pub iam_environment_id: Uuid,
    /// Replacement IAM testing-environment root key.
    pub iam_environment_key: IamEnvironmentKey,
    /// Canonical test-only Briefcase Application ID inside IAM.
    pub iam_app_id: ApplicationId,
    /// Fresh test-only Application secret returned by IAM.
    pub iam_app_secret: IamApplicationSecret,
}

impl TestingEnvironmentIamPairing {
    /// Builds a replacement pairing from one complete IAM credential set.
    #[must_use]
    pub fn new(
        iam_environment_id: Uuid,
        iam_environment_key: IamEnvironmentKey,
        iam_app_id: ApplicationId,
        iam_app_secret: IamApplicationSecret,
    ) -> Self {
        Self {
            iam_environment_id,
            iam_environment_key,
            iam_app_id,
            iam_app_secret,
        }
    }
}

/// Renames or re-describes a live testing environment.
#[derive(Clone, Debug, Default, Serialize)]
pub struct TestingEnvironmentUpdate {
    /// Replacement name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Replacement description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Option<String>>,
}

/// How a folder is named: by identifier, or by the path its URL shows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Destination {
    /// The folder's stable identifier.
    Id(Uuid),
    /// The folder's organization-relative path, such as `private/cos:tos/notes`.
    Path(String),
}

impl Destination {
    /// Names a folder by path.
    #[must_use]
    pub fn path(path: impl Into<String>) -> Self {
        Self::Path(path.into())
    }
}

impl From<Uuid> for Destination {
    fn from(value: Uuid) -> Self {
        Self::Id(value)
    }
}

/// Which entries to list, and how many.
#[derive(Clone, Debug, Default)]
pub struct ListEntries {
    /// Folder to browse; omitted lists the organization base.
    pub parent: Option<Destination>,
    /// Filter expression; with one and no parent, the whole tree is searched.
    pub filter: Option<String>,
    /// Cursor from a previous page.
    pub cursor: Option<String>,
    /// Page size, 1 through 100.
    pub limit: Option<u16>,
}

impl ListEntries {
    /// Lists the contents of one folder.
    #[must_use]
    pub fn in_folder(parent: impl Into<Destination>) -> Self {
        Self {
            parent: Some(parent.into()),
            ..Self::default()
        }
    }

    /// Filters everything the caller can reach.
    #[must_use]
    pub fn matching(filter: impl Into<String>) -> Self {
        Self {
            filter: Some(filter.into()),
            ..Self::default()
        }
    }

    /// Continues from a previous page.
    #[must_use]
    pub fn after(mut self, cursor: impl Into<String>) -> Self {
        self.cursor = Some(cursor.into());
        self
    }

    /// Asks for at most this many entries.
    #[must_use]
    pub const fn limit(mut self, limit: u16) -> Self {
        self.limit = Some(limit);
        self
    }
}

/// A grant to create, either on its own or with a new folder.
#[derive(Clone, Debug, Serialize)]
pub struct NewGrant {
    /// The member receiving access.
    pub principal: ActorRef,
    /// Rights to convey; read is always included.
    pub access: Vec<AccessRight>,
    /// Whether the grant reaches descendants.
    pub inherit: bool,
    /// Makes this a read-only expiring share that ends after 1 to 43,200 minutes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in_minutes: Option<u32>,
}

impl NewGrant {
    /// Grants a member a set of rights on the entry itself.
    #[must_use]
    pub fn new(principal: ActorRef, access: impl IntoIterator<Item = AccessRight>) -> Self {
        Self {
            principal,
            access: access.into_iter().collect(),
            inherit: false,
            expires_in_minutes: None,
        }
    }

    /// Makes the grant an expiring share: read-only, and gone after `minutes`
    /// (1 to 43,200). It never replaces a permanent grant the member holds.
    #[must_use]
    pub const fn expires_after(mut self, minutes: u32) -> Self {
        self.expires_in_minutes = Some(minutes);
        self
    }

    /// Extends the grant to everything inside the entry.
    #[must_use]
    pub const fn inheriting(mut self) -> Self {
        self.inherit = true;
        self
    }
}

/// A folder to create.
#[derive(Clone, Debug)]
pub struct NewFolder {
    /// Display name.
    pub name: String,
    /// Destination folder; omitted creates at the organization base.
    pub parent: Option<Destination>,
    /// Which container to create in, required at the organization base.
    pub root_type: Option<RootType>,
    /// IAM tag, required when `root_type` is [`RootType::Tag`].
    pub tag: Option<String>,
    /// Members invited as the folder is created.
    pub invitees: Vec<NewGrant>,
    /// Key that makes a retry return the same folder.
    pub idempotency_key: Option<IdempotencyKey>,
}

impl NewFolder {
    /// Creates a folder inside another folder.
    #[must_use]
    pub fn in_folder(name: impl Into<String>, parent: impl Into<Destination>) -> Self {
        Self {
            name: name.into(),
            parent: Some(parent.into()),
            root_type: None,
            tag: None,
            invitees: Vec::new(),
            idempotency_key: None,
        }
    }

    /// Creates a typed folder at the organization base.
    ///
    /// The folder is a sibling of the reserved containers. Its type defines
    /// access, not its location; use `in_folder` for an explicit parent.
    #[must_use]
    pub fn at_base(name: impl Into<String>, root_type: RootType) -> Self {
        Self {
            name: name.into(),
            parent: None,
            root_type: Some(root_type),
            tag: None,
            invitees: Vec::new(),
            idempotency_key: None,
        }
    }

    /// Creates a folder at the organization base with the given tag boundary.
    #[must_use]
    pub fn in_tag(name: impl Into<String>, tag: impl Into<String>) -> Self {
        Self {
            tag: Some(tag.into()),
            ..Self::at_base(name, RootType::Tag)
        }
    }

    /// Invites members as the folder is created.
    #[must_use]
    pub fn inviting(mut self, invitees: impl IntoIterator<Item = NewGrant>) -> Self {
        self.invitees = invitees.into_iter().collect();
        self
    }

    /// Uses a caller-owned idempotency key.
    #[must_use]
    pub fn with_idempotency_key(mut self, key: IdempotencyKey) -> Self {
        self.idempotency_key = Some(key);
        self
    }
}

/// A rename, a move, or both.
#[derive(Clone, Debug, Default, Serialize)]
pub struct EntryUpdate {
    /// New display name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// New parent folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<Uuid>,
}

impl EntryUpdate {
    /// Renames an entry, leaving it where it is.
    #[must_use]
    pub fn rename(name: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            parent_id: None,
        }
    }

    /// Moves an entry into another folder, keeping its name.
    #[must_use]
    pub const fn move_to(parent_id: Uuid) -> Self {
        Self {
            name: None,
            parent_id: Some(parent_id),
        }
    }

    /// Also renames the entry being moved.
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

/// Where a file's bytes come from.
#[derive(Clone, Debug)]
pub enum UploadSource {
    /// A local file, streamed rather than read into memory.
    File(PathBuf),
    /// Bytes already in hand.
    Bytes(Vec<u8>),
}

/// A file to upload.
///
/// There is one upload operation for every size: Briefcase decides internally
/// whether the bytes travel as one provider request or a durable multipart
/// transfer. Uploading a name an active file already carries publishes that
/// file's next version.
#[derive(Clone, Debug)]
pub struct Upload {
    /// Folder to upload into.
    pub destination: Destination,
    /// Name the file takes in that folder.
    pub file_name: String,
    /// Media type; `application/octet-stream` when omitted.
    pub content_type: Option<String>,
    /// Where the bytes come from.
    pub source: UploadSource,
    /// Key that makes a retry return the same file rather than a second one.
    pub idempotency_key: Option<IdempotencyKey>,
    /// Makes a new file self-destruct this many minutes (1 to 43,200) after
    /// the upload finishes. Refused when the name is an existing file.
    pub self_destruct_minutes: Option<u32>,
}

impl Upload {
    /// Uploads a local file, keeping its own name.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Configuration`] when the path has no file name.
    pub fn file(
        destination: impl Into<Destination>,
        path: impl Into<PathBuf>,
    ) -> crate::Result<Self> {
        let path = path.into();
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| crate::Error::Configuration("the upload path has no file name".into()))?
            .to_owned();
        Ok(Self {
            destination: destination.into(),
            file_name,
            content_type: None,
            source: UploadSource::File(path),
            idempotency_key: None,
            self_destruct_minutes: None,
        })
    }

    /// Uploads bytes already in hand under a chosen name.
    #[must_use]
    pub fn bytes(
        destination: impl Into<Destination>,
        file_name: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            destination: destination.into(),
            file_name: file_name.into(),
            content_type: None,
            source: UploadSource::Bytes(bytes.into()),
            idempotency_key: None,
            self_destruct_minutes: None,
        }
    }

    /// Declares the media type of the bytes.
    #[must_use]
    pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    /// Stores the file under a different name than the local one.
    #[must_use]
    pub fn named(mut self, file_name: impl Into<String>) -> Self {
        self.file_name = file_name.into();
        self
    }

    /// Uses a caller-owned idempotency key.
    #[must_use]
    pub fn with_idempotency_key(mut self, key: IdempotencyKey) -> Self {
        self.idempotency_key = Some(key);
        self
    }

    /// Makes the new file self-destruct `minutes` (1 to 43,200) after the
    /// upload finishes. It is then deleted for good and never enters a bin.
    #[must_use]
    pub const fn self_destructing(mut self, minutes: u32) -> Self {
        self.self_destruct_minutes = Some(minutes);
        self
    }
}

/// One range of a file, for a player that seeks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ByteRange {
    /// First byte, inclusive.
    pub start: u64,
    /// Last byte, inclusive; omitted means to the end.
    pub end: Option<u64>,
}

impl ByteRange {
    /// A range from an offset to the end of the file.
    #[must_use]
    pub const fn from(start: u64) -> Self {
        Self { start, end: None }
    }

    /// An inclusive range.
    #[must_use]
    pub const fn inclusive(start: u64, end: u64) -> Self {
        Self {
            start,
            end: Some(end),
        }
    }

    pub(crate) fn header_value(self) -> String {
        self.end.map_or_else(
            || format!("bytes={}-", self.start),
            |end| format!("bytes={}-{end}", self.start),
        )
    }
}

/// Targets whose effective access the caller wants reported.
#[derive(Clone, Debug, Default, Serialize)]
pub struct PermissionQuery {
    /// Targets addressed by identifier.
    pub entry_ids: Vec<Uuid>,
    /// Targets addressed by path.
    pub paths: Vec<String>,
}

impl PermissionQuery {
    /// Asks about entries by identifier.
    #[must_use]
    pub fn entries(entry_ids: impl IntoIterator<Item = Uuid>) -> Self {
        Self {
            entry_ids: entry_ids.into_iter().collect(),
            paths: Vec::new(),
        }
    }

    /// Asks about entries by path.
    #[must_use]
    pub fn paths(paths: impl IntoIterator<Item = String>) -> Self {
        Self {
            entry_ids: Vec::new(),
            paths: paths.into_iter().collect(),
        }
    }

    /// Adds paths to a query that already names identifiers.
    #[must_use]
    pub fn and_paths(mut self, paths: impl IntoIterator<Item = String>) -> Self {
        self.paths.extend(paths);
        self
    }
}

/// A file an application creates for the member it represents.
///
/// The destination, name, and media type travel inside the IAM proof rather
/// than in this request: an application cannot redirect a proof it legitimately
/// obtained to somewhere else.
#[derive(Clone)]
pub struct OnBehalfOfUpload {
    /// The application's own IAM identifier.
    pub app_id: String,
    /// The single-use access proof IAM minted for these exact bytes.
    pub proof: String,
    /// The bytes the proof was minted over.
    pub source: UploadSource,
}

impl std::fmt::Debug for OnBehalfOfUpload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let source = match &self.source {
            UploadSource::File(path) => format!("file {}", path.display()),
            UploadSource::Bytes(bytes) => format!("{} bytes", bytes.len()),
        };
        formatter
            .debug_struct("OnBehalfOfUpload")
            .field("app_id", &self.app_id)
            .field("proof", &"<redacted>")
            .field("source", &source)
            .finish()
    }
}

impl OnBehalfOfUpload {
    /// Creates a file from a local file's bytes.
    #[must_use]
    pub fn file(
        app_id: impl Into<String>,
        proof: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            app_id: app_id.into(),
            proof: proof.into(),
            source: UploadSource::File(path.into()),
        }
    }

    /// Creates a file from bytes already in hand.
    #[must_use]
    pub fn bytes(
        app_id: impl Into<String>,
        proof: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            app_id: app_id.into(),
            proof: proof.into(),
            source: UploadSource::Bytes(bytes.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ByteRange, Destination, ListEntries, NewFolder, NewGrant, OnBehalfOfUpload,
        TestingEnvironmentIamPairing, Upload,
    };
    use crate::{
        config::{ApplicationId, IamApplicationSecret, IamEnvironmentKey},
        models::{AccessRight, ActorRef, RootType},
    };

    #[test]
    fn a_range_reads_as_the_header_a_player_sends() {
        assert_eq!(ByteRange::inclusive(0, 1023).header_value(), "bytes=0-1023");
        assert_eq!(ByteRange::from(2048).header_value(), "bytes=2048-");
    }

    #[test]
    fn an_upload_takes_the_local_file_name_unless_told_otherwise() {
        let upload = Upload::file(Destination::path("public"), "/tmp/report.pdf").unwrap();
        assert_eq!(upload.file_name, "report.pdf");
        assert_eq!(upload.named("q3.pdf").file_name, "q3.pdf");
        assert!(Upload::file(Destination::path("public"), "/").is_err());
    }

    #[test]
    fn folder_builders_carry_the_container_they_name() {
        let public = NewFolder::at_base("handbook", RootType::Public);
        assert!(public.parent.is_none());
        assert_eq!(public.root_type, Some(RootType::Public));

        let tagged = NewFolder::in_tag("specs", "engineering");
        assert_eq!(tagged.tag.as_deref(), Some("engineering"));
        assert_eq!(tagged.root_type, Some(RootType::Tag));

        let invited = NewFolder::in_folder("drafts", Destination::path("public/handbook"))
            .inviting([
                NewGrant::new(ActorRef::carbon("cos:tos"), [AccessRight::Read]).inheriting(),
            ]);
        assert_eq!(invited.invitees.len(), 1);
        assert!(invited.invitees[0].inherit);
    }

    #[test]
    fn listing_builders_compose() {
        let query = ListEntries::matching("is:md").limit(5).after("cursor");
        assert_eq!(query.filter.as_deref(), Some("is:md"));
        assert_eq!(query.limit, Some(5));
        assert_eq!(query.cursor.as_deref(), Some("cursor"));
    }

    #[test]
    fn obo_debug_redacts_the_single_use_proof_and_content() {
        let request =
            OnBehalfOfUpload::bytes("notes", "obo_secret_proof", b"secret file body".to_vec());
        let rendered = format!("{request:?}");
        assert!(!rendered.contains("obo_secret_proof"));
        assert!(!rendered.contains("secret file body"));
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn iam_pairing_debug_redacts_both_secrets() {
        let root = "a2345678901234567890123456789012";
        let app_secret = format!("ask_{}", "b".repeat(43));
        let request = TestingEnvironmentIamPairing::new(
            uuid::Uuid::from_u128(1),
            IamEnvironmentKey::new(root).unwrap(),
            ApplicationId::new("briefcase").unwrap(),
            IamApplicationSecret::new(app_secret.clone()).unwrap(),
        );
        let rendered = format!("{request:?}");

        assert!(!rendered.contains(root));
        assert!(!rendered.contains(&app_secret));
        assert_eq!(rendered.matches("<redacted>").count(), 2);
    }
}
