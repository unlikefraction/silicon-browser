use super::*;
use crate::delivery_auth::obo::RecordingTokens;
use serde_json::json;
use tokio::io::AsyncWriteExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_bytes, body_partial_json, header, method, path},
};

#[tokio::test]
async fn staged_upload_reconciles_same_operation_and_uses_selected_provider_context() {
    for testing in [false, true] {
        let server = MockServer::start().await;
        let operations = briefcase_client::OPERATIONS
            .iter()
            .map(|op| json!({"id":op.id,"version":op.version,"method":op.method,"path":op.path}))
            .collect::<Vec<_>>();
        Mock::given(path("/api/version")).respond_with(ResponseTemplate::new(200).insert_header("briefcase-api-version","v1").set_body_json(json!({"service":"silicon-briefcase","selected_api_version":"v1","supported_api_versions":["v1"],"contract_version":"1.0.0","build":"test","operations":operations}))).expect(2).mount(&server).await;
        let operation = Uuid::new_v4();
        let upload = Uuid::new_v4();
        let entry = Uuid::new_v4();
        let bytes = b"exact immutable browser recording";
        let status = |state| json!({"operation_id":operation,"upload_id":upload,"state":state,"expires_at":"2099-01-01T00:00:00Z","published_entry_id":if state=="committed"{Some(entry)}else{None}});
        let committed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let written = committed.clone();
        let reserved = status("reserved");
        let completed = status("committed");
        Mock::given(path("/api/v1/obo/uploads/reserve")).and(header("x-org-id","chosen-storage")).and(header("x-iam-obo-access-token","oba_reserve")).and(body_partial_json(json!({"operation_id":operation,"size":bytes.len(),"sha256":hex::encode(Sha256::digest(bytes)),"name":"session.mp4"}))).respond_with(move |_:&wiremock::Request| {
            let mut value=if written.load(std::sync::atomic::Ordering::SeqCst){completed.clone()}else{reserved.clone()};
            if !written.load(std::sync::atomic::Ordering::SeqCst){value["capability"]=json!("cap_private");}
            ResponseTemplate::new(200).set_body_json(value)
        }).expect(2).mount(&server).await;
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1/obo/uploads/{upload}/content")))
            .and(header("x-briefcase-upload-capability", "cap_private"))
            .and(body_bytes(bytes.to_vec()))
            .respond_with(ResponseTemplate::new(200).set_body_json(status("staged")))
            .expect(1)
            .mount(&server)
            .await;
        let written = committed.clone();
        Mock::given(path("/api/v1/obo/uploads/commit"))
            .and(header("x-iam-obo-access-token", "oba_commit"))
            .respond_with(move |_: &wiremock::Request| {
                written.store(true, std::sync::atomic::Ordering::SeqCst);
                ResponseTemplate::new(503)
                    .set_body_json(json!({"error":{"code":"unavailable","message":"response lost after commit"}}))
            })
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/api/v1/obo/entries/list")).and(header("x-iam-obo-access-token","oba_list")).and(body_partial_json(json!({"path":"apps/browser/private/si:chosen"}))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"items":[{"id":entry,"org_id":"chosen-storage","type":"file","visibility":"full","name":"session.mp4","path":"apps/browser/private/si:chosen/session.mp4","root_type":"private","content_type":"video/mp4","size":bytes.len(),"permanent_url":format!("{}/org/chosen-storage/file",server.uri()),"origin_app_id":"browser","effective_access":["read"],"created_at":"2099-01-01T00:00:00Z","updated_at":"2099-01-01T00:00:00Z","deleted_at":null}],"next_cursor":null}))).expect(1).mount(&server).await;
        let token = |value| OnBehalfOfGrant::new(value).unwrap();
        let tokens = RecordingTokens {
            credential_version: "fixture".into(),
            org_id: "chosen-storage".into(),
            actor_id: "si:chosen".into(),
            reserve: token("oba_reserve"),
            commit: token("oba_commit"),
            list: token("oba_list"),
            testing_secret: testing.then(|| secrecy::SecretString::from(format!("ask_{}", "S".repeat(43)))),
            expires_at: chrono::Utc::now() + chrono::TimeDelta::minutes(30),
        };
        let client = BriefcaseClient::new(&server.uri(), None).unwrap();
        for retry in [false, true] {
            let mut file = tokio::fs::File::from_std(tempfile::tempfile().unwrap());
            file.write_all(bytes).await.unwrap();
            file.flush().await.unwrap();
            let (hash, size) = client.hash_file(&mut file).await.unwrap();
            let result = client
                .upload_recording("browser", operation, &tokens, "session.mp4", "video/mp4", &hash, file, size)
                .await;
            if retry {
                assert_eq!(result.unwrap().id, entry);
            } else {
                assert!(result.is_err());
            }
        }
        for request in server.received_requests().await.unwrap() {
            assert!(!request.headers.contains_key("authorization"));
            assert!(!request.headers.contains_key("x-iam-obo-access-proof"));
            if request.url.path().contains("/content") {
                assert!(!request.headers.contains_key("x-iam-obo-access-token"));
            }
            if request.url.path().starts_with("/api/v1/") {
                assert_eq!(request.headers["x-org-id"], "chosen-storage");
            }
            assert_eq!(request.headers.contains_key("x-briefcase-app-secret"), testing);
        }
    }
}

#[tokio::test]
async fn raw_upload_is_retired_and_staging_bounds_still_fail_closed() {
    let server = MockServer::start().await;
    let client = BriefcaseClient::with_upload_limit(&server.uri(), None, 3).unwrap();
    assert!(client.upload_raw("tos", "browser", &OnBehalfOfGrant::new("obo_retired").unwrap(), vec![1]).await.is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
    assert!(client.body_sha256(b"four").is_err());
    for origin in [
        "https://user:pass@example.com",
        "http://example.com",
        "https://example.com/api/v1",
        "https://example.com/#fragment",
    ] {
        assert!(BriefcaseClient::new(origin, None).is_err());
    }
    let mut file = tokio::fs::File::from_std(tempfile::tempfile().unwrap());
    file.write_all(b"four").await.unwrap();
    assert!(client.hash_file(&mut file).await.is_err());
}
