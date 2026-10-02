//! The contract this build was written against, and how it is checked.
//!
//! Briefcase publishes the API majors it serves and, for every operation, the
//! revision of that operation's request and response. This build carries the
//! same list. Comparing them once, before the first real call, turns "the
//! response no longer means what I think" into a startup failure with a name
//! on it.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::error::{IncompatibleContract, OperationMismatch};

/// API major this build speaks.
pub const API_VERSION: &str = "v1";

/// One operation and the revision this build was written against.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationRevision {
    /// Operation identifier from the published contract.
    pub id: &'static str,
    /// Revision of its request and response shape.
    pub version: &'static str,
    /// HTTP method this operation uses.
    pub method: &'static str,
    /// Path below the versioned API base.
    pub path: &'static str,
}

const fn operation(
    id: &'static str,
    version: &'static str,
    method: &'static str,
    path: &'static str,
) -> OperationRevision {
    OperationRevision {
        id,
        version,
        method,
        path,
    }
}

/// Every operation this client calls, with the revision it expects.
pub const OPERATIONS: [OperationRevision; 62] = [
    operation(
        "reserveDelegatedUpload",
        "3.0.0",
        "POST",
        "/obo/uploads/reserve",
    ),
    operation(
        "commitDelegatedUpload",
        "3.0.0",
        "POST",
        "/obo/uploads/commit",
    ),
    operation(
        "getDelegatedUploadStatus",
        "3.0.0",
        "POST",
        "/obo/uploads/status",
    ),
    operation(
        "cancelDelegatedUpload",
        "3.0.0",
        "POST",
        "/obo/uploads/cancel",
    ),
    operation(
        "transferDelegatedUpload",
        "2.0.0",
        "PUT",
        "/obo/uploads/{upload_id}/content",
    ),
    operation("readIamInfo", "2.0.0", "GET", "/iam"),
    operation("readLoginStatus", "2.0.0", "GET", "/auth/status"),
    operation("readApiVersion", "1.0.0", "GET", "/version"),
    operation("exchangeShortLivedToken", "2.0.0", "POST", "/auth/slt"),
    operation(
        "refreshApplicationSession",
        "2.0.0",
        "POST",
        "/auth/refresh",
    ),
    operation(
        "listTestingEnvironments",
        "2.0.0",
        "GET",
        "/organizations/{org_id}/testing-environments",
    ),
    operation(
        "createTestingEnvironment",
        "2.0.0",
        "POST",
        "/organizations/{org_id}/testing-environments",
    ),
    operation(
        "getTestingEnvironment",
        "2.0.0",
        "GET",
        "/organizations/{org_id}/testing-environments/{environment_id}",
    ),
    operation(
        "updateTestingEnvironment",
        "1.0.0",
        "PATCH",
        "/organizations/{org_id}/testing-environments/{environment_id}",
    ),
    operation(
        "deleteTestingEnvironment",
        "1.0.0",
        "DELETE",
        "/organizations/{org_id}/testing-environments/{environment_id}",
    ),
    operation(
        "getTestingEnvironmentKey",
        "1.0.0",
        "GET",
        "/organizations/{org_id}/testing-environments/{environment_id}/key",
    ),
    operation(
        "replaceTestingEnvironmentIamPairing",
        "2.0.0",
        "POST",
        "/organizations/{org_id}/testing-environments/{environment_id}/iam-pairings",
    ),
    operation(
        "cleanTestingEnvironment",
        "1.0.0",
        "POST",
        "/organizations/{org_id}/testing-environments/{environment_id}/cleanings",
    ),
    operation(
        "restoreTestingEnvironment",
        "1.0.0",
        "POST",
        "/organizations/{org_id}/testing-environments/{environment_id}/restorations",
    ),
    operation(
        "describeCurrentTestingEnvironment",
        "2.0.0",
        "GET",
        "/testing-environment",
    ),
    operation(
        "cleanCurrentTestingEnvironment",
        "1.0.0",
        "POST",
        "/testing-environment/cleanings",
    ),
    operation("listEntries", "1.0.0", "GET", "/entries"),
    operation("createFolder", "1.0.0", "POST", "/entries"),
    operation("getEntry", "1.0.0", "GET", "/entries/{entry_id}"),
    operation("updateEntry", "1.0.0", "PATCH", "/entries/{entry_id}"),
    operation("moveEntryToBin", "1.0.0", "DELETE", "/entries/{entry_id}"),
    operation(
        "readEntryContent",
        "1.0.0",
        "GET",
        "/entries/{entry_id}/content",
    ),
    operation(
        "downloadEntry",
        "1.0.0",
        "GET",
        "/entries/{entry_id}/download",
    ),
    operation(
        "resolvePermanentUrl",
        "1.1.0",
        "GET",
        "/org/{org_id}/{path}",
    ),
    operation("uploadFile", "1.0.0", "POST", "/uploads"),
    operation("createFileOnBehalfOfMember", "3.0.0", "POST", "/obo/files"),
    operation(
        "createFolderOnBehalfOfMember",
        "3.0.0",
        "POST",
        "/obo/folders/create",
    ),
    operation(
        "listEntriesOnBehalfOfMember",
        "3.0.0",
        "POST",
        "/obo/entries/list",
    ),
    operation(
        "readFileOnBehalfOfMember",
        "3.0.0",
        "POST",
        "/obo/files/read",
    ),
    operation(
        "trashEntryOnBehalfOfMember",
        "3.0.0",
        "POST",
        "/obo/entries/trash",
    ),
    operation(
        "listPermissions",
        "1.0.0",
        "GET",
        "/entries/{entry_id}/permissions",
    ),
    operation(
        "grantPermission",
        "1.0.0",
        "POST",
        "/entries/{entry_id}/permissions",
    ),
    operation(
        "revokePermission",
        "1.0.0",
        "DELETE",
        "/entries/{entry_id}/permissions/{grant_id}",
    ),
    operation(
        "inspectEffectivePermissions",
        "1.0.0",
        "POST",
        "/permissions/effective",
    ),
    operation("searchFiles", "1.0.0", "GET", "/search"),
    operation("submitTelemetry", "1.0.0", "POST", "/telemetry"),
    operation("submitReport", "1.0.0", "POST", "/reports"),
    operation("listNotifications", "1.0.0", "GET", "/notifications"),
    operation("readNotifications", "1.0.0", "POST", "/notifications/read"),
    operation(
        "listEntryActivity",
        "1.0.0",
        "GET",
        "/entries/{entry_id}/activity",
    ),
    operation(
        "listVersions",
        "1.0.0",
        "GET",
        "/entries/{entry_id}/versions",
    ),
    operation(
        "restoreVersion",
        "1.0.0",
        "POST",
        "/entries/{entry_id}/versions/{version_id}/restore",
    ),
    operation("readOrganizationUsage", "1.0.0", "GET", "/usage"),
    operation("listBin", "1.0.0", "GET", "/bin"),
    operation("restoreEntry", "1.0.0", "POST", "/bin/{entry_id}/restore"),
    operation(
        "configureOrganizationBucket",
        "1.0.0",
        "PUT",
        "/storage/configuration",
    ),
    operation(
        "listInvitations",
        "1.0.0",
        "GET",
        "/entries/{entry_id}/invitations",
    ),
    operation(
        "createInvitation",
        "1.0.0",
        "POST",
        "/entries/{entry_id}/invitations",
    ),
    operation(
        "revokeInvitation",
        "1.0.0",
        "DELETE",
        "/entries/{entry_id}/invitations/{grant_id}",
    ),
    operation(
        "changeExpiringShare",
        "1.0.0",
        "PATCH",
        "/entries/{entry_id}/invitations/{grant_id}",
    ),
    operation(
        "makeEntryPermanent",
        "1.0.0",
        "DELETE",
        "/entries/{entry_id}/self-destruct",
    ),
    operation(
        "readLinkAccess",
        "1.1.0",
        "GET",
        "/entries/{entry_id}/link-access",
    ),
    operation(
        "setLinkAccess",
        "1.1.0",
        "PUT",
        "/entries/{entry_id}/link-access",
    ),
    operation("listEntryLogs", "2.0.0", "GET", "/entries/{entry_id}/logs"),
    operation("readPublicEntry", "1.1.0", "GET", "/public/{org_id}/{path}"),
    operation(
        "inviteOnBehalfOfMember",
        "3.0.0",
        "POST",
        "/obo/invitations",
    ),
    operation(
        "setLinkAccessOnBehalfOfMember",
        "3.0.0",
        "POST",
        "/obo/link-access",
    ),
];

/// What a Briefcase deployment says about itself.
#[derive(Clone, Debug, Deserialize)]
pub struct ServiceVersion {
    /// Service identity, always `silicon-briefcase`.
    pub service: String,
    /// API major selected for this exchange.
    pub selected_api_version: String,
    /// Every API major the deployment serves.
    pub supported_api_versions: Vec<String>,
    /// Version of the published contract document.
    pub contract_version: String,
    /// Build identity of the deployment.
    pub build: String,
    /// Every operation the deployment serves, with its revision.
    pub operations: Vec<ServedOperation>,
}

/// One operation as the deployment serves it.
#[derive(Clone, Debug, Deserialize)]
pub struct ServedOperation {
    /// Operation identifier.
    pub id: String,
    /// Revision of its request and response shape.
    pub version: String,
    /// HTTP method.
    pub method: String,
    /// Path below the versioned API base.
    pub path: String,
}

impl ServiceVersion {
    /// Checks this build's expectations against what the deployment serves.
    ///
    /// # Errors
    ///
    /// Returns [`IncompatibleContract`] when the service identity, selected or
    /// supported API major, or an operation this client calls differs from
    /// this build. Unknown operation IDs are additive and remain compatible.
    pub fn check_compatibility(&self) -> Result<(), IncompatibleContract> {
        let mut mismatched = Vec::new();
        let mut missing = Vec::new();

        if self.service != "silicon-briefcase" {
            mismatched.push(OperationMismatch {
                id: "service".to_owned(),
                expected: "silicon-briefcase".to_owned(),
                served: self.service.clone(),
            });
        }
        if self.selected_api_version != API_VERSION {
            mismatched.push(OperationMismatch {
                id: "selectedApiVersion".to_owned(),
                expected: API_VERSION.to_owned(),
                served: self.selected_api_version.clone(),
            });
        }

        if !self
            .supported_api_versions
            .iter()
            .any(|version| version == API_VERSION)
        {
            return Err(IncompatibleContract {
                served_api_versions: self.supported_api_versions.clone(),
                mismatched_operations: Vec::new(),
                missing_operations: Vec::new(),
            });
        }

        let mut counts = BTreeMap::<&str, usize>::new();
        for served in &self.operations {
            *counts.entry(&served.id).or_default() += 1;
        }
        for (id, count) in counts.into_iter().filter(|(_, count)| *count > 1) {
            mismatched.push(OperationMismatch {
                id: id.to_owned(),
                expected: "exactly one catalog entry".to_owned(),
                served: format!("{count} catalog entries"),
            });
        }

        for expected in OPERATIONS {
            let served: Vec<_> = self
                .operations
                .iter()
                .filter(|served| served.id == expected.id)
                .collect();
            match served.as_slice() {
                [served]
                    if served.version == expected.version
                        && served.method == expected.method
                        && served.path == expected.path => {}
                [served] => mismatched.push(OperationMismatch {
                    id: expected.id.to_owned(),
                    expected: operation_signature(expected.version, expected.method, expected.path),
                    served: operation_signature(&served.version, &served.method, &served.path),
                }),
                [] => missing.push(expected.id.to_owned()),
                _duplicates => {}
            }
        }

        if mismatched.is_empty() && missing.is_empty() {
            return Ok(());
        }
        Err(IncompatibleContract {
            served_api_versions: self.supported_api_versions.clone(),
            mismatched_operations: mismatched,
            missing_operations: missing,
        })
    }
}

fn operation_signature(version: &str, method: &str, path: &str) -> String {
    format!("{version} {method} {path}")
}

#[cfg(test)]
mod tests {
    use super::{OPERATIONS, ServedOperation, ServiceVersion};

    fn served(operations: Vec<ServedOperation>) -> ServiceVersion {
        ServiceVersion {
            service: "silicon-briefcase".to_owned(),
            selected_api_version: "v1".to_owned(),
            supported_api_versions: vec!["v1".to_owned()],
            contract_version: "0.2.0".to_owned(),
            build: "0.1.0".to_owned(),
            operations,
        }
    }

    fn everything_this_build_expects() -> Vec<ServedOperation> {
        OPERATIONS
            .into_iter()
            .map(|operation| ServedOperation {
                id: operation.id.to_owned(),
                version: operation.version.to_owned(),
                method: operation.method.to_owned(),
                path: operation.path.to_owned(),
            })
            .collect()
    }

    #[test]
    fn a_matching_deployment_is_compatible() {
        assert!(
            served(everything_this_build_expects())
                .check_compatibility()
                .is_ok()
        );
    }

    #[test]
    fn one_changed_revision_names_the_operation() {
        let mut operations = everything_this_build_expects();
        let changed = operations
            .iter_mut()
            .find(|operation| operation.id == "listEntries");
        assert!(changed.is_some());
        if let Some(changed) = changed {
            changed.version = "2.0.0".to_owned();
        }
        let error = served(operations)
            .check_compatibility()
            .expect_err("a changed revision must be refused");

        assert_eq!(error.mismatched_operations.len(), 1);
        assert!(error.mismatched_operations[0].served.starts_with("2.0.0 "));
        assert!(error.to_string().contains("listEntries"));
    }

    #[test]
    fn an_operation_the_deployment_does_not_serve_is_named() {
        let mut operations = everything_this_build_expects();
        operations.retain(|operation| operation.id != "searchFiles");
        let error = served(operations)
            .check_compatibility()
            .expect_err("a missing operation must be refused");

        assert_eq!(error.missing_operations, vec!["searchFiles".to_owned()]);
    }

    #[test]
    fn a_deployment_without_this_major_is_refused_before_anything_else() {
        let mut version = served(everything_this_build_expects());
        version.supported_api_versions = vec!["v2".to_owned()];
        let error = version
            .check_compatibility()
            .expect_err("an unserved major must be refused");

        assert!(error.mismatched_operations.is_empty());
        assert_eq!(error.served_api_versions, vec!["v2".to_owned()]);
    }

    #[test]
    fn identity_selected_major_and_route_catalog_are_verified() {
        let mut version = served(everything_this_build_expects());
        version.service = "not-briefcase".to_owned();
        version.selected_api_version = "v2".to_owned();
        version.operations[0].method = if version.operations[0].method == "POST" {
            "GET"
        } else {
            "POST"
        }
        .to_owned();
        version.operations[1].path = "/wrong".to_owned();
        let error = version
            .check_compatibility()
            .expect_err("identity, negotiation, method and path mismatches must be refused");

        assert!(
            error
                .mismatched_operations
                .iter()
                .any(|mismatch| mismatch.id == "service")
        );
        assert!(
            error
                .mismatched_operations
                .iter()
                .any(|mismatch| mismatch.id == "selectedApiVersion")
        );
        assert!(
            error
                .mismatched_operations
                .iter()
                .any(|mismatch| mismatch.id == OPERATIONS[0].id)
        );
        assert!(
            error
                .mismatched_operations
                .iter()
                .any(|mismatch| mismatch.id == OPERATIONS[1].id)
        );
    }

    #[test]
    fn duplicate_catalog_entries_are_refused() {
        let mut operations = everything_this_build_expects();
        operations.push(operations[0].clone());
        let error = served(operations)
            .check_compatibility()
            .expect_err("ambiguous duplicate operation IDs must be refused");

        assert!(
            error
                .mismatched_operations
                .iter()
                .any(|mismatch| mismatch.id == OPERATIONS[0].id)
        );
    }

    #[test]
    fn additive_unknown_operations_are_compatible() {
        let mut operations = everything_this_build_expects();
        operations.push(ServedOperation {
            id: "unknownOperation".to_owned(),
            version: "1.0.0".to_owned(),
            method: "GET".to_owned(),
            path: "/unknown".to_owned(),
        });

        assert!(
            served(operations).check_compatibility().is_ok(),
            "new operation IDs do not change operations this client calls"
        );
    }

    #[test]
    fn duplicate_unknown_operation_ids_are_still_refused() {
        let mut operations = everything_this_build_expects();
        let unknown = ServedOperation {
            id: "unknownOperation".to_owned(),
            version: "1.0.0".to_owned(),
            method: "GET".to_owned(),
            path: "/unknown".to_owned(),
        };
        operations.extend([unknown.clone(), unknown]);

        let error = served(operations)
            .check_compatibility()
            .expect_err("every operation ID must remain unambiguous");
        assert_eq!(error.mismatched_operations[0].id, "unknownOperation");
    }
}
