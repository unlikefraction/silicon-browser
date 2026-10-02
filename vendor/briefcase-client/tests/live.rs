//! Opt-in coverage against a running Briefcase test plane.
//!
//! Both tests are ignored by default because they require live credentials.
//! The smoke test is deliberately read-only. The mutation test must only run
//! against a disposable environment: it creates a UUID-named private tree,
//! exercises the package's file workflow, and moves that tree to the
//! recoverable bin when it finishes (including after an ordinary test error).

use std::{env, error::Error, io};

use briefcase_client::{
    API_VERSION, AccessRight, ActorRef, ByteRange, Client, Config, EffectiveAccess, EntryType,
    EntryUpdate, EnvironmentKey, IdempotencyKey, ListEntries, NewFolder, NewGrant, PermissionQuery,
    RootType, Upload,
};
use uuid::Uuid;

const API_URL: &str = "BRIEFCASE_LIVE_API_URL";
const ORG_ID: &str = "BRIEFCASE_LIVE_ORG_ID";
const BEARER_TOKEN: &str = "BRIEFCASE_LIVE_BEARER_TOKEN";
const TEST_ROOT: &str = "BRIEFCASE_LIVE_TEST_ROOT";
const GRANTEE_TYPE: &str = "BRIEFCASE_LIVE_GRANTEE_TYPE";
const GRANTEE_ID: &str = "BRIEFCASE_LIVE_GRANTEE_ID";

type LiveResult<T> = Result<T, Box<dyn Error>>;

fn required_environment_variable(name: &str) -> LiveResult<String> {
    let value = env::var(name).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} must be set to run the ignored live test: {error}"),
        )
    })?;
    if value.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} must not be empty"),
        )
        .into());
    }
    Ok(value)
}

fn optional_environment_variable(name: &str) -> LiveResult<Option<String>> {
    match env::var(name) {
        Ok(value) if value.trim().is_empty() => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} must not be empty when it is set"),
        )
        .into()),
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} could not be read: {error}"),
        )
        .into()),
    }
}

fn optional_grantee() -> LiveResult<Option<ActorRef>> {
    match (
        optional_environment_variable(GRANTEE_TYPE)?,
        optional_environment_variable(GRANTEE_ID)?,
    ) {
        (None, None) => Ok(None),
        (Some(actor_type), Some(id)) => match actor_type.as_str() {
            "carbon" => Ok(Some(ActorRef::carbon(id))),
            "silicon" => Ok(Some(ActorRef::silicon(id))),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{GRANTEE_TYPE} must be `carbon` or `silicon`"),
            )
            .into()),
        },
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{GRANTEE_TYPE} and {GRANTEE_ID} must be set together"),
        )
        .into()),
    }
}

fn ensure(condition: bool, message: impl Into<String>) -> LiveResult<()> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::other(message.into()).into())
    }
}

async fn live_client() -> LiveResult<(Client, String)> {
    let api_url = required_environment_variable(API_URL)?;
    let organization = required_environment_variable(ORG_ID)?;
    let bearer = required_environment_variable(BEARER_TOKEN)?;
    let root = EnvironmentKey::new(required_environment_variable(TEST_ROOT)?)?;
    let client = Client::connect(
        Config::new(&api_url, &organization)?
            .with_token(bearer)
            .with_environment(root)
            .with_auto_update(false),
    )
    .await?;
    Ok((client, organization))
}

#[tokio::test]
#[ignore = "requires a running Briefcase service and explicit live test credentials"]
async fn live_testing_plane_contract_and_reads() -> LiveResult<()> {
    let api_url = required_environment_variable(API_URL)?;
    let organization = required_environment_variable(ORG_ID)?;
    let bearer = required_environment_variable(BEARER_TOKEN)?;
    let root = EnvironmentKey::new(required_environment_variable(TEST_ROOT)?)?;

    // Keep the contract check genuinely anonymous. The root selects the plane
    // but is not actor authentication and must not be confused with the IAM
    // bearer used by ordinary organization operations below.
    let anonymous = Client::connect(
        Config::new(&api_url, &organization)?
            .with_environment(root.clone())
            .with_auto_update(false),
    )
    .await?;
    let version = anonymous.version().await?;
    assert_eq!(version.service, "silicon-briefcase");
    assert_eq!(version.selected_api_version, API_VERSION);
    assert!(
        version
            .supported_api_versions
            .iter()
            .any(|supported| supported == API_VERSION)
    );

    // Self-description is authorized by the test root alone and discloses no
    // IAM pairing secret. It proves that the supplied key selects a live plane.
    let environment = anonymous.current_testing_environment().await?;
    assert!(!environment.id.is_nil());
    assert!(!environment.name.trim().is_empty());
    assert!(environment.key_generation > 0);

    // A successful listing proves the package sends the independent IAM actor
    // credential and test-plane selector together on ordinary API operations.
    let authenticated = Client::connect(
        Config::new(&api_url, &organization)?
            .with_token(bearer)
            .with_environment(root)
            .with_auto_update(false),
    )
    .await?;
    let _page = authenticated.list_entries(&ListEntries::default()).await?;

    Ok(())
}

#[tokio::test]
#[ignore = "mutates an explicitly disposable Briefcase testing environment"]
#[allow(clippy::too_many_lines)]
async fn live_disposable_file_workflow() -> LiveResult<()> {
    let (client, organization) = live_client().await?;
    let grantee = optional_grantee()?;
    let run_id = Uuid::new_v4();
    let folder_name = format!("briefcase-client-live-{run_id}");
    let folder_key = IdempotencyKey::new(format!("live-folder-{run_id}"))?;
    let folder_request = NewFolder::at_base(&folder_name, RootType::Private)
        .with_idempotency_key(folder_key.clone());
    let folder = client.create_folder(&folder_request).await?;

    // Keep cleanup outside the workflow result so ordinary API/assertion
    // errors still leave the unique test tree in the recoverable bin.
    let workflow = async {
        ensure(
            folder.org_id == organization,
            "folder returned the wrong organization",
        )?;
        ensure(
            folder.is_folder(),
            "the created private entry is not a folder",
        )?;
        ensure(
            folder.root_type == RootType::Private,
            "folder is not private",
        )?;

        let folder_replay = client.create_folder(&folder_request).await?;
        ensure(
            folder_replay.id == folder.id,
            "replaying folder creation produced a second entry",
        )?;

        let nested = client
            .create_folder(&NewFolder::in_folder("nested", folder.id))
            .await?;
        ensure(
            nested.parent_id == Some(folder.id),
            "nested folder has the wrong parent",
        )?;

        let file_name = "client-live.txt";
        let first_bytes = b"briefcase client live version one\n".to_vec();
        let second_bytes = b"briefcase client live version two, changed\n".to_vec();
        let upload_key = IdempotencyKey::new(format!("live-upload-{run_id}"))?;
        let first_upload = Upload::bytes(nested.id, file_name, first_bytes.clone())
            .with_content_type("text/plain")
            .with_idempotency_key(upload_key);
        let file = client.upload(&first_upload).await?;
        ensure(
            file.entry_type == EntryType::File,
            "upload did not return a file",
        )?;
        ensure(
            file.parent_id == Some(nested.id),
            "uploaded file has the wrong parent",
        )?;
        ensure(
            file.size == Some(first_bytes.len() as u64),
            "uploaded size is wrong",
        )?;

        let upload_replay = client.upload(&first_upload).await?;
        ensure(
            upload_replay.id == file.id,
            "replaying an upload produced a second file",
        )?;
        let versions_after_replay = client.versions(file.id).await?;
        ensure(
            versions_after_replay.items.len() == 1,
            "replaying an upload published another version",
        )?;

        let by_id = client.entry(file.id).await?;
        let by_path = client.entry_at(&file.path).await?;
        ensure(
            by_id.id == file.id && by_path.id == file.id,
            "entry lookup disagrees",
        )?;
        let listed = client
            .list_all_entries(&ListEntries::in_folder(nested.id).limit(1), 10)
            .await?;
        ensure(
            listed.iter().any(|entry| entry.id == file.id),
            "folder listing did not contain the upload",
        )?;

        let full = client.read_content(file.id, None).await?.bytes().await?;
        ensure(
            full == first_bytes,
            "full content read returned different bytes",
        )?;
        let at_path = client.read_content_at(&file.path).await?.bytes().await?;
        ensure(
            at_path == first_bytes,
            "path content read returned different bytes",
        )?;
        let range = client
            .read_content(file.id, Some(ByteRange::inclusive(0, 8)))
            .await?;
        ensure(
            range.content_range() == Some(format!("bytes 0-8/{}", first_bytes.len()).as_str()),
            "range read returned the wrong Content-Range",
        )?;
        ensure(
            range.bytes().await? == first_bytes[..=8],
            "range read returned different bytes",
        )?;

        let second = client
            .upload(
                &Upload::bytes(nested.id, file_name, second_bytes.clone())
                    .with_content_type("text/plain"),
            )
            .await?;
        ensure(
            second.id == file.id,
            "second upload did not version the existing file",
        )?;
        let versions = client.versions(file.id).await?;
        ensure(
            versions.items.len() == 2
                && versions.items[0].number == 2
                && versions.items[1].number == 1,
            "version history is not newest-first with versions one and two",
        )?;

        let restore_key = IdempotencyKey::new(format!("live-restore-{run_id}"))?;
        client
            .restore_version_with_key(file.id, versions.items[1].id, &restore_key)
            .await?;
        client
            .restore_version_with_key(file.id, versions.items[1].id, &restore_key)
            .await?;
        ensure(
            client.versions(file.id).await?.items.len() == 3,
            "restoring once (and replaying it) did not leave exactly three versions",
        )?;
        ensure(
            client.read_content(file.id, None).await?.bytes().await? == first_bytes,
            "restoring version one did not restore its bytes",
        )?;

        ensure(
            client.permissions(file.id).await?.is_empty(),
            "a new private file unexpectedly has an explicit grant",
        )?;
        if let Some(principal) = grantee {
            let grant = client
                .grant(
                    file.id,
                    &NewGrant::new(principal.clone(), [AccessRight::Read]),
                )
                .await?;
            ensure(
                grant.principal == principal,
                "grant names the wrong principal",
            )?;
            ensure(
                client
                    .permissions(file.id)
                    .await?
                    .iter()
                    .any(|item| item.id == grant.id),
                "the created grant is absent from the permission listing",
            )?;
            client.revoke(file.id, grant.id).await?;
            ensure(
                !client
                    .permissions(file.id)
                    .await?
                    .iter()
                    .any(|item| item.id == grant.id),
                "the revoked grant remains in the permission listing",
            )?;
        }

        let missing_path = format!("{}/missing-{run_id}", nested.path);
        let inspection = client
            .effective_access(
                &PermissionQuery::entries([file.id])
                    .and_paths([file.path.clone(), missing_path.clone()]),
            )
            .await?;
        let access = inspection
            .items
            .iter()
            .find(|item| item.entry_id == file.id)
            .map(|item| &item.effective_access);
        ensure(
            access.is_some_and(|rights| {
                [
                    EffectiveAccess::Read,
                    EffectiveAccess::Update,
                    EffectiveAccess::Delete,
                    EffectiveAccess::ManagePermissions,
                ]
                .iter()
                .all(|right| rights.contains(right))
            }),
            "the owner is missing expected effective file access",
        )?;
        ensure(
            inspection.unresolved_paths.contains(&missing_path),
            "effective-access inspection resolved a nonexistent path",
        )?;
        ensure(
            client.activity(file.id).await?.len() >= 3,
            "file history omitted create/version/restore activity",
        )?;

        let renamed_name = "client-live-restored.txt";
        let renamed = client
            .update_entry(file.id, &EntryUpdate::rename(renamed_name))
            .await?;
        ensure(
            renamed.name == renamed_name,
            "file rename returned the old name",
        )?;
        ensure(
            client.entry_at(&renamed.path).await?.id == file.id,
            "the renamed permanent path does not resolve",
        )?;

        client.delete_entry(file.id).await?;
        let hidden = client.entry(file.id).await;
        ensure(
            hidden.is_err_and(|error| error.is_not_found()),
            "a binned file is still visible through entry lookup",
        )?;
        ensure(
            client
                .bin(None, Some(100))
                .await?
                .items
                .iter()
                .any(|item| item.id == file.id),
            "the deleted file is absent from the recoverable bin",
        )?;
        let restored = client.restore_from_bin(file.id).await?;
        ensure(
            restored.id == file.id,
            "bin restore returned a different entry",
        )?;
        ensure(
            client.read_content(file.id, None).await?.bytes().await? == first_bytes,
            "bin restore did not recover the current bytes",
        )?;

        let usage = client.usage().await?;
        ensure(
            usage.storage.used_bytes >= first_bytes.len() as u64,
            "usage did not account for the live file",
        )?;
        Ok::<(), Box<dyn Error>>(())
    }
    .await;

    let cleanup = client.delete_entry(folder.id).await;
    match (workflow, cleanup) {
        (Err(workflow_error), Err(cleanup_error)) => Err(io::Error::other(format!(
            "{workflow_error}; cleanup also failed: {cleanup_error}"
        ))
        .into()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error.into()),
        (Ok(()), Ok(())) => {
            ensure(
                client
                    .bin(None, Some(100))
                    .await?
                    .items
                    .iter()
                    .any(|item| item.id == folder.id),
                "the live test tree was not left in the recoverable bin",
            )?;
            Ok(())
        }
    }
}
