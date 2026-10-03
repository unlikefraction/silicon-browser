use super::*;
use crate::providers::test_http::{spawn_header_only_server, spawn_json_server};
use tokio::io::AsyncWriteExt;

fn manifest() -> BriefcaseUploadManifest {
    BriefcaseUploadManifest {
        operation_id: Uuid::from_u128(1),
        parent_path: String::new(),
        name: "recording.mp4".into(),
        content_type: "video/mp4".into(),
        size: 3,
        sha256: hex::encode(Sha256::digest(b"abc")),
    }
}

fn status(state: &str) -> serde_json::Value {
    json!({"operation_id": Uuid::from_u128(1), "upload_id": Uuid::from_u128(2),
        "state": state, "expires_at": Utc::now() + chrono::TimeDelta::hours(1),
        "published_entry_id": if state == "committed" { Some(Uuid::from_u128(3)) } else { None }})
}

fn entry() -> serde_json::Value {
    json!({"id":Uuid::from_u128(3), "org_id":"client-org", "type":"file",
        "name":"recording.mp4", "path":"apps/browser/private/actor/recording.mp4",
        "content_type":"video/mp4", "size":3,
        "permanent_url":"https://briefcase.example/org/client-org/apps/browser/private/actor/recording.mp4",
        "origin_app_id":"browser"})
}

async fn file() -> tokio::fs::File {
    let mut file = tokio::fs::File::from_std(tempfile::tempfile().unwrap());
    file.write_all(b"abc").await.unwrap();
    file.flush().await.unwrap();
    file
}

#[tokio::test]
async fn delegated_lifecycle_separates_control_tokens_from_capability_and_resolves_receipt() {
    let catalog: serde_json::Value =
        serde_json::from_str(include_str!("../../../../deploy/honeycomb-application.json")).unwrap();
    let mut registered = catalog["app_scope"]["external"]
        .as_array()
        .unwrap()
        .iter()
        .map(|endpoint| endpoint["endpoint_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    registered.sort_unstable();
    let mut expected = BRIEFCASE_RECORDING_ENDPOINTS;
    expected.sort_unstable();
    assert_eq!(registered, expected);
    for testing in [false, true] {
        let mut reserved = status("reserved");
        reserved["capability"] = json!("private-upload-capability");
        let (base, mut requests, server) = spawn_json_server(vec![
            (200, reserved.to_string()),
            (200, status("staged").to_string()),
            (200, status("committed").to_string()),
            (200, status("committed").to_string()),
            (200, json!({"items":[],"next_cursor":"page-2"}).to_string()),
            (200, json!({"items":[entry()],"next_cursor":null}).to_string()),
        ])
        .await;
        let test_key = format!("ask_{}", "a".repeat(43));
        let client = BriefcaseClient::new(&base, testing.then_some(test_key.as_str())).unwrap();
        let token_value = if testing { "oat_reusable-access-token" } else { "oba_reusable-access-token" };
        let token = OnBehalfOfGrant::new(token_value).unwrap();
        let manifest = manifest();
        let reservation = client.reserve_upload("client-org", "browser", &token, &manifest).await.unwrap();
        assert!(!format!("{reservation:?}").contains("private-upload-capability"));
        let staged = client.transfer_upload("client-org", &reservation, file().await, 3).await.unwrap();
        assert_eq!(staged.state, BriefcaseUploadState::Staged);
        let committed = client
            .commit_upload("client-org", "browser", &token, manifest.operation_id, staged.upload_id)
            .await
            .unwrap();
        let recovered = client.upload_status("client-org", "browser", &token, manifest.operation_id).await.unwrap();
        assert_eq!(recovered.published_entry_id, committed.published_entry_id);
        let receipt = client
            .resolve_upload_entry("client-org", "browser", &token, &manifest, committed.published_entry_id.unwrap())
            .await
            .unwrap();
        assert_eq!(receipt.org_id, "client-org");
        for (index, path) in ["reserve", "content", "commit", "status", "list", "list"].into_iter().enumerate() {
            let request = requests.recv().await.unwrap();
            assert!(request.target.ends_with(path));
            assert_eq!(request.headers.contains("x-briefcase-app-secret:"), testing);
            assert!(request.headers.contains("x-org-id: client-org"));
            for forbidden in ["authorization:", "x-iam-obo-access-proof:", "x-testing-environment-key:"] {
                assert!(!request.headers.contains(forbidden));
            }
            if index == 1 {
                assert_eq!(request.method, "PUT");
                assert_eq!(request.body, b"abc");
                assert!(request.headers.contains("content-length: 3"));
                assert!(request.headers.contains("x-briefcase-upload-capability: private-upload-capability"));
                assert!(!request.headers.contains("x-app-id:"));
                assert!(!request.headers.contains("x-iam-obo-access-token:"));
            } else {
                assert_eq!(request.method, "POST");
                assert!(request.headers.contains("x-app-id: browser"));
                assert!(request.headers.contains(&format!("x-iam-obo-access-token: {token_value}")));
                assert!(!request.headers.contains("x-briefcase-upload-capability:"));
                let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
                if index == 0 {
                    assert_eq!(body, serde_json::to_value(&manifest).unwrap());
                }
                if index == 5 {
                    assert_eq!(body["cursor"], "page-2");
                }
            }
        }
        server.await.unwrap();
    }
}

#[tokio::test]
async fn invalid_tokens_manifests_and_changed_files_fail_before_network() {
    let client = BriefcaseClient::new("http://127.0.0.1:1", None).unwrap();
    for token in ["obo_retired", "ort_refresh", "oba_", "oba_bad\nheader", "oba_bad token"] {
        assert!(OnBehalfOfGrant::new(token).is_err());
    }
    for (field, value) in [("name", "../escape"), ("sha256", "not-a-digest"), ("parent_path", "../other")] {
        let mut bad = manifest();
        match field {
            "name" => bad.name = value.into(),
            "sha256" => bad.sha256 = value.into(),
            _ => bad.parent_path = value.into(),
        }
        assert!(client.validate_manifest(&bad).is_err());
    }
    let mut reservation = status("reserved");
    reservation["capability"] = json!("private-capability");
    let reservation = serde_json::from_value(reservation).unwrap();
    assert!(client.transfer_upload("client-org", &reservation, file().await, 4).await.is_err());
    for origin in [
        "http://remote.example",
        "https://user:pass@example.com",
        "https://example.com/api/v1",
        "https://example.com?secret=hidden",
    ] {
        assert!(BriefcaseClient::new(origin, None).is_err());
    }
    assert!(BriefcaseClient::new("http://127.0.0.1:1", Some(&"a".repeat(32))).is_err());
}

#[tokio::test]
async fn rejected_or_changed_authority_requests_consent_without_erasing_acl_denials() {
    let token = OnBehalfOfGrant::new("oba_private").unwrap();
    for (status, code, needs_consent) in [
        (401, "unauthenticated", true),
        (403, "forbidden", false),
        (403, "obo_token_revoked", true),
        (412, "consent_changed", true),
        (429, "rate_limit", false),
        (503, "unavailable", false),
    ] {
        let (base, _requests, server) = spawn_json_server(vec![(
            status,
            json!({"error":{"code":code,"message":"private-body-content"}}).to_string(),
        )])
        .await;
        let error = BriefcaseClient::new(&base, None)
            .unwrap()
            .upload_status("client-org", "browser", &token, manifest().operation_id)
            .await
            .unwrap_err();
        assert_eq!(matches!(error, ProviderError::Http { status: 401, .. }), needs_consent);
        assert!(!error.to_string().contains("private-body-content"));
        if !needs_consent {
            assert!(matches!(error, ProviderError::Http { status: actual, .. } if actual == status));
        }
        server.await.unwrap();
    }
}

#[tokio::test]
async fn receipt_and_operation_mismatches_are_rejected_without_retries_or_secret_leaks() {
    let token = OnBehalfOfGrant::new("oba_secret-token").unwrap();
    for (field, value) in [
        ("org_id", json!("other")),
        ("name", json!("other.mp4")),
        ("size", json!(4)),
        ("permanent_url", json!("https://example.com/?secret=hidden")),
    ] {
        let mut bad = entry();
        bad[field] = value;
        let (base, _requests, server) =
            spawn_json_server(vec![(200, json!({"items":[bad], "next_cursor":null}).to_string())]).await;
        let client = BriefcaseClient::new(&base, None).unwrap();
        assert!(
            client
                .resolve_upload_entry("client-org", "browser", &token, &manifest(), Uuid::from_u128(3))
                .await
                .is_err()
        );
        server.await.unwrap();
    }
    let mut wrong = status("committed");
    wrong["operation_id"] = json!(Uuid::from_u128(9));
    let (base, _requests, server) = spawn_json_server(vec![(200, wrong.to_string())]).await;
    assert!(
        BriefcaseClient::new(&base, None)
            .unwrap()
            .upload_status("client-org", "browser", &token, manifest().operation_id)
            .await
            .is_err()
    );
    server.await.unwrap();
    let (base, _requests, server) = spawn_json_server(vec![(403, "oba_secret-token private-detail".into())]).await;
    let error = BriefcaseClient::new(&base, None)
        .unwrap()
        .upload_status("client-org", "browser", &token, manifest().operation_id)
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("secret-token"));
    assert!(!error.to_string().contains("private-detail"));
    server.await.unwrap();
    let (base, server) =
        spawn_header_only_server(307, vec![("Location".into(), "http://127.0.0.1:1/secret".into())]).await;
    assert!(
        BriefcaseClient::new(&base, None)
            .unwrap()
            .upload_status("client-org", "browser", &token, manifest().operation_id)
            .await
            .is_err()
    );
    server.await.unwrap();
}
