//! Wire-level tests for explicit consent and application management contracts.

#![allow(clippy::expect_used)]

use std::{
    io::{Read as _, Write as _},
    net::TcpListener,
    sync::mpsc,
    thread,
    time::Duration,
};

use serde_json::{Value, json};
use silicon_iam_client::{Client, Credential, IdempotencyKey, Mutation, models};
use uuid::Uuid;

fn service(
    response: Value,
) -> (
    Client,
    mpsc::Receiver<(String, Value)>,
    thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock service");
    let address = listener.local_addr().expect("mock address");
    let (send, receive) = mpsc::channel();
    let task = thread::spawn(move || {
        let (mut connection, _) = listener.accept().expect("accept one request");
        connection
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        let mut request = Vec::new();
        let boundary = loop {
            let mut bytes = [0; 4096];
            let length = connection.read(&mut bytes).expect("read request headers");
            assert!(length > 0, "request ended before its headers");
            request.extend_from_slice(&bytes[..length]);
            if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = String::from_utf8(request[..boundary].to_vec()).expect("ASCII HTTP headers");
        let length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().expect("body length"))
            })
            .unwrap_or(0);
        while request.len() - boundary < length {
            let mut bytes = [0; 4096];
            let count = connection.read(&mut bytes).expect("read request body");
            assert!(count > 0, "request ended before its declared body");
            request.extend_from_slice(&bytes[..count]);
        }
        let body =
            serde_json::from_slice(&request[boundary..boundary + length]).unwrap_or(Value::Null);
        send.send((headers, body)).expect("return captured request");
        let body = response.to_string();
        write!(connection, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).expect("send response");
    });
    let client = Client::builder(&format!("http://{address}"))
        .expect("local service URL")
        .auto_update(false)
        .build()
        .expect("client");
    (client, receive, task)
}

#[tokio::test]
async fn short_lived_token_sends_the_reviewed_version_and_exact_permission_set() {
    let (client, capture, server) = service(json!({"slt":"slt_one_use","expires_in":120}));
    let approved = vec![
        "self.identity.read".to_owned(),
        "obo:vendor>drive:files.read".to_owned(),
    ];
    client
        .auth()
        .short_lived_token_for_organizations(
            "checkout",
            &["customer".to_owned()],
            17,
            &approved,
            &Mutation::new(),
        )
        .await
        .expect("issue explicit SLT");
    let (headers, body) = capture.recv().expect("captured token request");
    assert!(headers.starts_with("POST /api/v1/app-auth/short-lived-tokens "));
    assert_eq!(
        body,
        json!({"app_id":"checkout","org_ids":["customer"],"scope_version":17,"approved_scopes":approved})
    );
    server.join().expect("mock completed");
}

#[tokio::test]
async fn a_review_decision_carries_concurrency_and_replay_protection() {
    let request_id = Uuid::from_u128(7);
    let response = json!({
        "id": request_id, "app_id":"checkout", "target_app_id":null,
        "scopes":["directory.carbons.read"], "status":"denied", "version":3,
        "created_at":"2026-09-12T00:00:00Z", "updated_at":"2026-09-12T00:00:00Z",
        "can_decide":true, "messages":[{"id":Uuid::nil(),"author":{"principal_id":Uuid::nil(),"type":"system","public_id":"iam"},"message":"Explain each critical scope.","created_at":"2026-09-12T00:00:00Z"}]
    });
    let (client, capture, server) = service(response);
    let mutation =
        Mutation::with_key(IdempotencyKey::parse("scope-decision-replay-0001").expect("key"));
    let result = client
        .application_scopes()
        .decide(
            request_id,
            2,
            &models::ApplicationScopeDecision {
                decision: models::ApplicationScopeDecisionDecision::Deny,
                reason: Some("Please explain why directory data is needed.".to_owned()),
            },
            &mutation,
        )
        .await
        .expect("decision response including system author");
    assert!(matches!(
        result.messages[0].author.type_field,
        models::ApplicationScopeMessageAuthorType::System
    ));
    let (headers, body) = capture.recv().expect("captured decision");
    assert!(headers.starts_with(&format!(
        "POST /api/v1/application-scope-requests/{request_id}/decisions "
    )));
    let lower = headers.to_ascii_lowercase();
    assert!(lower.contains("if-match: \"2\"\r\n"));
    assert!(lower.contains("idempotency-key: scope-decision-replay-0001\r\n"));
    assert_eq!(body["decision"], "deny");
    assert!(
        body["reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty())
    );
    server.join().expect("mock completed");
}

#[tokio::test]
async fn application_testing_keeps_the_production_credential_out_of_its_payload() {
    let environment_id = Uuid::from_u128(11);
    let (client, capture, server) = service(json!({
        "environment_id":environment_id,"org_id":"acme","name":"checkout integration",
        "description":null,"iam_test_key":"0123456789abcdefghijklmnopqrstuv","app_id":"checkout",
        "app_secret":"ask_isolated_test_secret","dependencies":["vendor>drive","vendor>mail"],
        "secret_replay_expires_at":"2026-09-12T00:10:00Z"
    }));
    let client = client.with_credential(Credential::application("checkout", "production-secret"));
    let result = client
        .applications()
        .create_testing_environment(
            &models::ApplicationTestingEnvironmentCreate {
                name: "checkout integration".to_owned(),
                description: None,
                iam_test_key: Some("existing-test-key".to_owned()),
            },
            &Mutation::new(),
        )
        .await
        .expect("provision test dependency graph");
    assert_eq!(result.environment_id, environment_id);
    assert_eq!(result.dependencies.len(), 2);
    let (headers, body) = capture.recv().expect("captured provisioning");
    assert!(headers.starts_with("POST /api/v1/application/testing-environments "));
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("authorization: basic ")
    );
    assert_eq!(
        body,
        json!({"name":"checkout integration","iam_test_key":"existing-test-key"})
    );
    assert!(!body.to_string().contains("production-secret"));
    server.join().expect("mock completed");
}

#[test]
fn unclassified_obo_endpoints_are_rejected_by_the_client_contract() {
    let endpoint = json!({"endpoint_id":"files.read","path":"/files","metadata":{}});
    assert!(serde_json::from_value::<models::ApplicationOboEndpoint>(endpoint).is_err());
}

#[tokio::test]
async fn app_verification_issuance_uses_basic_auth_without_secret_replay() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    for lifetime in [None, Some(60), Some(3600)] {
        let (client, capture, server) = service(json!({
            "app_id":"checkout", "app_access_key":"aak_generated_secret",
            "valid_till":"2026-09-22T00:05:00Z"
        }));
        let issued = client
            .with_credential(Credential::application("checkout", "ask_issuer_secret"))
            .app_verification()
            .issue(&models::AppAccessKeyIssue {
                ttl_seconds: lifetime,
            })
            .await
            .expect("issue app identity key");
        assert_eq!(issued.app_access_key, "aak_generated_secret");
        assert!(!format!("{issued:?}").contains("aak_generated_secret"));
        let (headers, body) = capture.recv().expect("captured issuance");
        assert!(headers.starts_with("POST /api/v1/app-verification/keys "));
        assert!(headers.contains(&format!(
            "authorization: Basic {}\r\n",
            STANDARD.encode("checkout:ask_issuer_secret")
        )));
        assert!(!headers.to_ascii_lowercase().contains("idempotency-key:"));
        assert_eq!(
            body,
            lifetime.map_or_else(|| json!({}), |value| json!({"ttl_seconds":value}))
        );
        server.join().expect("mock completed");
    }
}

#[tokio::test]
async fn app_verification_uses_receiver_credentials_and_keeps_the_testing_context() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    for valid in [false, true] {
        let response = if valid {
            json!({"valid_key":true,"app_id":"checkout","valid_till":"2026-09-22T00:05:00Z"})
        } else {
            json!({"valid_key":false})
        };
        let (client, capture, server) = service(response);
        let input = models::AppAccessKeyVerify {
            app_id: "checkout".to_owned(),
            app_access_key: "aak_calling_key".to_owned(),
        };
        let verified = client
            .with_credential(Credential::application(
                "vendor>billing",
                "ask_receiver_secret",
            ))
            .with_environment(
                silicon_iam_client::EnvironmentKey::new("X".repeat(32)).expect("test key"),
            )
            .app_verification()
            .verify(&input)
            .await
            .expect("check calling app identity");
        assert_eq!(verified.valid_key, valid);
        assert_eq!(verified.app_id.as_deref(), valid.then_some("checkout"));
        assert_eq!(verified.valid_till.is_some(), valid);
        assert!(!format!("{input:?}").contains("aak_calling_key"));
        let (headers, body) = capture.recv().expect("captured verification");
        assert!(headers.starts_with("POST /api/v1/app-verification/verify "));
        assert!(headers.contains(&format!(
            "authorization: Basic {}\r\n",
            STANDARD.encode("vendor>billing:ask_receiver_secret")
        )));
        assert!(headers.contains(&format!(
            "x-testing-environment-key: {}\r\n",
            "X".repeat(32)
        )));
        assert!(!headers.to_ascii_lowercase().contains("idempotency-key:"));
        assert_eq!(
            body,
            json!({"app_id":"checkout", "app_access_key":"aak_calling_key"})
        );
        server.join().expect("mock completed");
    }
}

#[tokio::test]
async fn app_verification_rejects_invalid_lifetimes_before_sending() {
    let client = Client::new("http://127.0.0.1:1").expect("loopback client");
    for seconds in [-1, 0, 59, 3601, i64::MAX] {
        let result = client
            .app_verification()
            .issue(&models::AppAccessKeyIssue {
                ttl_seconds: Some(seconds),
            })
            .await;
        assert!(matches!(result, Err(silicon_iam_client::Error::Invalid(_))));
    }
}

#[tokio::test]
async fn app_verification_rejects_incomplete_or_inconsistent_identity_responses() {
    for response in [
        json!({"valid_key":true}),
        json!({"valid_key":true,"app_id":"checkout"}),
        json!({"valid_key":true,"valid_till":"2026-09-22T00:05:00Z"}),
        json!({"valid_key":true,"app_id":"other>app","valid_till":"2026-09-22T00:05:00Z"}),
        json!({"valid_key":false,"app_id":"checkout"}),
        json!({"valid_key":false,"valid_till":"2026-09-22T00:05:00Z"}),
        json!({"valid_key":false,"app_id":null}),
        json!({"valid_key":false,"valid_till":null}),
    ] {
        let (client, captured, server) = service(response);
        let result = client
            .with_credential(Credential::application(
                "vendor>billing",
                "ask_receiver_secret",
            ))
            .app_verification()
            .verify(&models::AppAccessKeyVerify {
                app_id: "checkout".to_owned(),
                app_access_key: "aak_calling_key".to_owned(),
            })
            .await;
        assert!(matches!(result, Err(silicon_iam_client::Error::Decode(_))));
        captured
            .recv_timeout(Duration::from_secs(5))
            .expect("request reached mock");
        server.join().expect("mock completed");
    }
}

#[tokio::test]
async fn scoped_profile_reads_preserve_undisclosed_fields_as_absent() {
    let (client, capture, server) = service(json!({"display_name":"Ada","version":8}));
    let client = client.with_credential(Credential::bearer("act_application_user"));
    let profile = client
        .application_reads()
        .me()
        .await
        .expect("scoped profile");
    assert_eq!(profile["display_name"], "Ada");
    for undisclosed in [
        "carbon_id",
        "type",
        "email",
        "phone_number",
        "org_role",
        "tags",
    ] {
        assert!(profile.get(undisclosed).is_none(), "invented {undisclosed}");
    }
    let (headers, body) = capture.recv().expect("captured profile read");
    assert!(headers.starts_with("GET /api/v1/me "));
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("authorization: bearer act_application_user\r\n")
    );
    assert_eq!(body, Value::Null);
    server.join().expect("mock completed");
}

#[tokio::test]
async fn contract_version_discovery_is_available_without_a_credential() {
    let manifest = json!({"items":[{"version":"v1","status":"current"}],"policy":{"sunset_after_idle_days":7}});
    let (client, capture, server) = service(manifest.clone());
    assert_eq!(
        client
            .system()
            .contracts()
            .await
            .expect("contract manifest"),
        manifest
    );
    let (headers, _) = capture.recv().expect("captured manifest read");
    assert!(headers.starts_with("GET /api/v1/contracts "));
    assert!(!headers.to_ascii_lowercase().contains("authorization:"));
    server.join().expect("mock completed");
}

#[tokio::test]
async fn bundle_availability_is_an_authenticated_organization_read() {
    let (client, capture, server) = service(json!({"available": false}));
    let client = client.with_credential(Credential::bearer("cat_direct_session"));
    let availability = client
        .bundles()
        .availability("acme")
        .await
        .expect("unavailable is a successful derived response");
    assert!(!availability.available);
    let (headers, body) = capture.recv().expect("captured availability request");
    assert!(headers.starts_with("GET /api/v1/organizations/acme/application-bundle-availability "));
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("authorization: bearer cat_direct_session\r\n")
    );
    assert_eq!(body, Value::Null);
    server.join().expect("mock completed");
}

#[tokio::test]
async fn organization_filtered_lists_preserve_pagination_and_return_the_next_cursor() {
    for bundles in [false, true] {
        let (client, capture, server) = service(json!({
            "items": [], "page": {"has_more": true, "next_cursor": "next-page"}
        }));
        let paging = silicon_iam_client::Paging::new().after("page+/=").limit(2);
        let page = if bundles {
            client
                .bundles()
                .list_for_organization("acme", &paging)
                .await
                .expect("organization bundle page")
                .page
        } else {
            client
                .applications()
                .list_for_organization("acme", Some("verified"), &paging)
                .await
                .expect("organization application page")
                .page
        };
        assert!(page.has_more);
        assert_eq!(page.next_cursor.as_deref(), Some("next-page"));
        let (headers, _) = capture.recv().expect("captured list request");
        let route = headers
            .split_whitespace()
            .nth(1)
            .expect("HTTP request target");
        let url = url::Url::parse(&format!("http://localhost{route}")).expect("request URL");
        let endpoint = if bundles {
            "application-bundles"
        } else {
            "applications"
        };
        assert_eq!(url.path(), format!("/api/v1/{endpoint}"));
        let query: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(
            query.get("org_id").map(std::borrow::Cow::as_ref),
            Some("acme")
        );
        assert_eq!(
            query.get("cursor").map(std::borrow::Cow::as_ref),
            Some("page+/=")
        );
        assert_eq!(query.get("limit").map(std::borrow::Cow::as_ref), Some("2"));
        if !bundles {
            assert_eq!(
                query.get("status").map(std::borrow::Cow::as_ref),
                Some("verified")
            );
        }
        server.join().expect("mock completed");
    }
}

#[tokio::test]
async fn bundle_logo_updates_distinguish_preserving_setting_and_clearing() {
    let original = "https://example.test/original.svg";
    let replacement = "https://example.test/replacement.svg";
    for (patch, expected) in [
        (None, Some(original)),
        (Some(Some(replacement.to_owned())), Some(replacement)),
        (Some(None), None),
    ] {
        let (client, capture, server) = service(json!({
            "id": Uuid::from_u128(17), "bundle_id": "acme>workspace", "org_id": "acme",
            "app_name": "Workspace", "app_logo": expected, "app_ids": ["billing"],
            "version": 5, "created_at": "2026-09-12T00:00:00Z", "updated_at": "2026-09-12T01:00:00Z"
        }));
        let updated = client
            .bundles()
            .update(
                "acme>workspace",
                4,
                &models::ApplicationBundlePatch {
                    app_name: Some(Some("Workspace".to_owned())),
                    app_logo: patch.clone(),
                    app_ids: None,
                },
                &Mutation::new(),
            )
            .await
            .expect("bundle presentation update");
        assert_eq!(updated.app_logo.as_deref(), expected);
        let (headers, body) = capture.recv().expect("captured bundle patch");
        assert!(headers.starts_with("PATCH /api/v1/application-bundles/acme%3Eworkspace "));
        assert!(headers.to_ascii_lowercase().contains("if-match: \"4\"\r\n"));
        match patch {
            None => assert!(body.get("app_logo").is_none()),
            Some(None) => assert_eq!(body["app_logo"], Value::Null),
            Some(Some(logo)) => assert_eq!(body["app_logo"], logo),
        }
        server.join().expect("mock completed");
    }
}

#[tokio::test]
async fn application_login_history_preserves_events_with_private_actor_identifiers() {
    let (client, capture, server) = service(json!({
        "items": [{
            "id": Uuid::from_u128(24),
            "actor": {"type": "silicon", "public_id": null},
            "app_id": "workspace", "org_id": "customer",
            "event_type": "oauth_token_exchange", "success": true,
            "request_id": "history-private-actor", "occurred_at": "2026-09-12T00:00:00Z"
        }],
        "page": {"has_more": false, "next_cursor": null}
    }));
    let history = client
        .applications()
        .login_history("workspace", &silicon_iam_client::Paging::new())
        .await
        .expect("private actor identifier must not invalidate the history page");
    assert_eq!(history.items.len(), 1);
    assert!(history.items[0].actor.public_id.is_none());
    assert!(history.items[0].success);
    let (headers, _) = capture.recv().expect("captured history request");
    assert!(headers.starts_with("GET /api/v1/applications/workspace/login-history "));
    server.join().expect("mock completed");
}

#[tokio::test]
async fn scoped_organization_creation_accepts_a_write_only_receipt() {
    let receipt =
        json!({"id": Uuid::from_u128(41), "org_id":"new_org", "version":1, "status":"active"});
    let (client, capture, server) = service(receipt.clone());
    let client = client.with_credential(Credential::bearer("act_scoped_creator"));
    let created = client
        .application_mutations()
        .create_organization(
            &models::OrganizationCreate {
                org_id: "new_org".to_owned(),
                name: "New organization".to_owned(),
                logo: None,
                description: None,
            },
            &Mutation::new(),
        )
        .await
        .expect("successful scoped creation must not decode as a full organization");
    assert_eq!(created, receipt);
    assert!(created.get("name").is_none());
    assert!(created.get("created_at").is_none());
    let (headers, body) = capture.recv().expect("captured creation");
    assert!(headers.starts_with("POST /api/v1/organizations "));
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("authorization: bearer act_scoped_creator\r\n")
    );
    assert_eq!(body, json!({"org_id":"new_org", "name":"New organization"}));
    server.join().expect("mock completed");
}

#[tokio::test]
async fn scoped_member_patch_preserves_receipts_and_mutation_preconditions() {
    let membership_id = "member[customer]";
    let receipt = json!({"id":membership_id,"version":9,"status":"active"});
    let (client, capture, server) = service(receipt.clone());
    let patch: models::MembershipDirectoryPatch =
        serde_json::from_value(json!({"first_silicon_membership_id":null}))
            .expect("clear a Silicon assignment");
    let mutation = Mutation::with_key(
        IdempotencyKey::parse("scoped-member-update-0001").expect("idempotency key"),
    )
    .step_up("stu_verified_action");
    let updated = client
        .application_mutations()
        .update_member("customer", membership_id, 8, &patch, &mutation)
        .await
        .expect("successful patch must accept omitted profile and authorization");
    assert_eq!(updated, receipt);
    for field in [
        "principal",
        "display_name",
        "org_role",
        "capabilities",
        "tags",
    ] {
        assert!(updated.get(field).is_none(), "invented {field}");
    }
    let (headers, body) = capture.recv().expect("captured patch");
    assert!(
        headers.starts_with("PATCH /api/v1/organizations/customer/members/member%5Bcustomer%5D ")
    );
    let headers = headers.to_ascii_lowercase();
    assert!(headers.contains("if-match: \"8\"\r\n"));
    assert!(headers.contains("idempotency-key: scoped-member-update-0001\r\n"));
    assert!(headers.contains("x-step-up-token: stu_verified_action\r\n"));
    assert!(headers.contains("content-type: application/merge-patch+json\r\n"));
    assert_eq!(body, json!({"first_silicon_membership_id":null}));
    server.join().expect("mock completed");
}

#[tokio::test]
async fn scoped_trust_write_accepts_an_empty_receipt() {
    let (client, capture, server) = service(json!({}));
    let result = client
        .application_mutations()
        .replace_default_trust(
            "customer",
            2,
            &models::TrustValue {
                boundary: models::TrustValueBoundary::Internal,
                level: models::TrustValueLevel::NeedsApproval,
            },
            &Mutation::new(),
        )
        .await
        .expect("write-only trust success must not require readable trust fields");
    assert_eq!(result, json!({}));
    let (headers, body) = capture.recv().expect("captured trust replacement");
    assert!(headers.starts_with("PUT /api/v1/organizations/customer/trust/default "));
    assert_eq!(
        body,
        json!({"boundary":"internal","level":"needs_approval"})
    );
    server.join().expect("mock completed");
}

#[tokio::test]
async fn scoped_silicon_creation_keeps_the_one_time_credential_and_sparse_identity() {
    let receipt = json!({
        "silicon": {"principal_id":Uuid::from_u128(43), "membership_id":"helper:customer[customer]", "silicon_id":"helper:customer", "org_id":"customer", "version":1},
        "silicon_token":"sit_generated_one_time", "secret_replay_expires_at":"2026-09-13T02:00:00Z"
    });
    let (client, capture, server) = service(receipt.clone());
    let input: models::SiliconCreate =
        serde_json::from_value(json!({"silicon_id":"helper","job_description":"Assistant"}))
            .expect("Silicon create input");
    let created = client
        .application_mutations()
        .create_silicon("customer", &input, &Mutation::new())
        .await
        .expect("generated credential must survive omitted nested Silicon profile");
    assert_eq!(created, receipt);
    assert!(created["silicon"].get("display_name").is_none());
    assert!(created["silicon"].get("reports_to_membership_id").is_none());
    let (headers, body) = capture.recv().expect("captured Silicon creation");
    assert!(headers.starts_with("POST /api/v1/organizations/customer/silicons "));
    assert_eq!(
        body,
        json!({"silicon_id":"helper","job_description":"Assistant"})
    );
    server.join().expect("mock completed");
}

#[tokio::test]
async fn honeycomb_service_keeps_actor_and_step_up_separate_on_the_wire() {
    let id = Uuid::now_v7();
    let (base, capture, server) = service(
        json!({"operation_id":id,"state":"accepted","iam_revision":8,"credential_version":2}),
    );
    let credential = format!("hck_{}", "a".repeat(43));
    let actor = format!("oat_{}", "b".repeat(43));
    let client = silicon_iam_client::honeycomb::ManagementClient::new(
        base.base_url().as_str(),
        credential.clone().into(),
    )
    .expect("management client");
    let input: models::HoneycombSecretRotation =
        serde_json::from_value(json!({"operation_id":id,"expected_iam_revision":7}))
            .expect("rotation input");
    let mutation =
        Mutation::with_key(IdempotencyKey::parse(id.to_string()).expect("idempotency key"))
            .step_up("sup_test_assertion");
    let receipt = client
        .rotate_secret("app", &actor.clone().into(), &input, &mutation)
        .await
        .expect("rotation receipt");
    assert_eq!(receipt.credential_version, Some(2));
    let (headers, body) = capture
        .recv_timeout(Duration::from_secs(5))
        .expect("captured request");
    assert!(headers.contains(&format!("authorization: Bearer {credential}")));
    assert!(headers.contains(&format!("x-honeycomb-actor-token: {actor}")));
    assert!(!headers.contains("x-testing-environment-key"));
    assert!(headers.contains("x-step-up-token: sup_test_assertion"));
    assert_eq!(body["operation_id"], id.to_string());
    server.join().expect("mock server");
}

#[tokio::test]
async fn honeycomb_application_authority_and_environment_key_are_separate() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use silicon_iam_client::{
        EnvironmentKey,
        honeycomb::{ManagementAuthority, ManagementClient},
    };
    let id = Uuid::now_v7();
    let environment = Uuid::now_v7();
    let (base, capture, server) = service(
        json!({"operation_id":id,"state":"accepted","iam_revision":8,"environment_id":environment}),
    );
    let service_credential = format!("hck_{}", "a".repeat(43));
    let client = ManagementClient::new(base.base_url().as_str(), service_credential.clone().into())
        .expect("client");
    let app_secret = secrecy::SecretString::from("ask_production_secret");
    let key = EnvironmentKey::new("X".repeat(32)).expect("key");
    let authority = ManagementAuthority::Application {
        app_id: "vendor>app",
        app_secret: &app_secret,
        environment_key: Some(&key),
    };
    let input:models::HoneycombTestingInstruction=serde_json::from_value(json!({"operation_id":id,"environment_id":environment,"generation":3,"expected_iam_revision":7,"expected_key_version":2,"operation":"import","app_id":"vendor>app","source_revisions":{"vendor>app":5}})).expect("input");
    client
        .testing_instruction_as(&authority, &input, &Mutation::new())
        .await
        .expect("receipt");
    let (headers, body) = capture
        .recv_timeout(Duration::from_secs(5))
        .expect("request");
    assert!(headers.contains(&format!("authorization: Bearer {service_credential}")));
    assert!(headers.contains(&format!(
        "x-honeycomb-application-authorization: Basic {}",
        STANDARD.encode("vendor>app:ask_production_secret")
    )));
    assert!(headers.contains(&format!("x-honeycomb-testing-key: {}", "X".repeat(32))));
    assert!(!headers.contains("x-honeycomb-actor-token"));
    assert!(!headers.contains("x-testing-environment-key"));
    assert_eq!(body["environment_id"], environment.to_string());
    assert_eq!(body["expected_key_version"], 2);
    assert!(!format!("{authority:?}").contains("ask_production_secret"));
    server.join().expect("server");
}

#[tokio::test]
async fn honeycomb_environment_authority_needs_no_testing_enable_key() {
    use silicon_iam_client::{
        EnvironmentKey,
        honeycomb::{ManagementAuthority, ManagementClient},
    };
    let id = Uuid::now_v7();
    let environment = Uuid::now_v7();
    let (base, capture, server) = service(
        json!({"operation_id":id,"state":"accepted","iam_revision":8,"environment_id":environment}),
    );
    let credential = format!("hck_{}", "a".repeat(43));
    let client =
        ManagementClient::new(base.base_url().as_str(), credential.clone().into()).expect("client");
    let key = EnvironmentKey::new("X".repeat(32)).expect("key");
    let authority = ManagementAuthority::Environment(&key);
    let input: models::HoneycombTestingInstruction = serde_json::from_value(json!({"operation_id":id,"environment_id":environment,"generation":3,"expected_iam_revision":7,"expected_key_version":2,"operation":"import","app_id":"vendor>app","source_revisions":{"vendor>app":5}})).expect("input");
    client
        .testing_instruction_as(&authority, &input, &Mutation::new())
        .await
        .expect("receipt");
    let (headers, body) = capture
        .recv_timeout(Duration::from_secs(5))
        .expect("request");
    assert!(headers.contains(&format!("authorization: Bearer {credential}")));
    assert!(headers.contains(&format!("x-honeycomb-testing-key: {}", "X".repeat(32))));
    assert!(!headers.contains("x-honeycomb-actor-token"));
    assert!(!headers.contains("x-honeycomb-application-authorization"));
    assert!(!headers.contains("x-testing-environment-key"));
    assert_eq!(body["expected_key_version"], 2);
    assert!(!format!("{authority:?}").contains(&"X".repeat(32)));
    server.join().expect("server");
}

#[tokio::test]
async fn honeycomb_publication_decision_has_a_typed_exact_plan_receipt() {
    use silicon_iam_client::honeycomb::ManagementClient;
    let id = Uuid::now_v7();
    let plan = Uuid::now_v7();
    let request = Uuid::now_v7();
    let response = json!({"operation_id":id,"decision_id":id,"state":"accepted","request_id":request,"plan_id":plan,"app_id":"vendor>app","configuration_revision":4,"provider":"honeycomb","scopes":[],"decision":"approve","reason":null});
    let (base, capture, server) = service(response.clone());
    let client = ManagementClient::new(
        base.base_url().as_str(),
        format!("hck_{}", "a".repeat(43)).into(),
    )
    .expect("client");
    let input:models::HoneycombPublicationDecision=serde_json::from_value(json!({"operation_id":id,"request_id":request,"plan_id":plan,"app_id":"vendor>app","configuration_revision":4,"provider":"honeycomb","scopes":[],"decision":"approve"})).expect("input");
    let actor = format!("oat_{}", "b".repeat(43)).into();
    let receipt = client
        .publication_decision(&actor, &input, &Mutation::new())
        .await
        .expect("decision");
    assert_eq!(receipt.decision_id, id);
    assert_eq!(receipt.plan_id, plan);
    let (headers, body) = capture
        .recv_timeout(Duration::from_secs(5))
        .expect("request");
    assert!(
        headers
            .starts_with("POST /api/v1/honeycomb/applications/vendor%3Eapp/publication-decisions ")
    );
    assert_eq!(body["request_id"], request.to_string());
    server.join().expect("server");
}
