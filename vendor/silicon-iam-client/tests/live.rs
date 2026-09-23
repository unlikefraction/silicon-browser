//! End-to-end checks against a running Silicon IAM.
//!
//! Ignored by default because they need a service. Point them at one and run:
//!
//! ```sh
//! SILICON_IAM_LIVE_URL=http://127.0.0.1:8097 \
//!   cargo test -p silicon-iam-client --test live -- --ignored --test-threads=1
//! ```
//!
//! The service must be running with `IAM_ALLOW_LOCAL_PROVIDERS=true` and
//! `IAM_EXPOSE_LOCAL_OTPS=true`, which is what lets a test read the
//! verification codes it is supposed to receive out of band.

// Direct OTP login exists only for the stateful CLI. Application integrations
// build the client without this feature and can begin a session only with an
// SLT through `OAuth::login`.
#![cfg(feature = "cli-session")]
// A failing step here should stop the test at the step that failed, naming it.
// The crate's own ban on panicking exists to keep library code from taking a
// caller's process down, which is not what a test binary does.
#![allow(clippy::expect_used)]
// Each test walks one whole flow end to end, and splitting the walk into
// helpers would hide the order the assertions depend on.
#![allow(clippy::too_many_lines)]

use silicon_iam_client::{
    Client, Credential, Mutation, Paging,
    api::{governance::ApprovalFilter, members::MemberFilter},
    models,
};

/// The verification code to present: the environment's fixed one, or the one
/// the local provider echoed back.
fn code_for(fixed: Option<&str>, echoed: Option<String>) -> String {
    fixed.map_or_else(
        || echoed.expect("the local provider echoes codes"),
        str::to_owned,
    )
}

fn service() -> Option<Client> {
    let base = std::env::var("SILICON_IAM_LIVE_URL").ok()?;
    Client::builder(&base)
        .and_then(|builder| builder.user_agent("silicon-iam-client-live-test").build())
        .ok()
}

/// A distinct handle per run, in the alphabet the contract accepts: lowercase
/// letters and 1-9, with zero excluded.
fn unique(prefix: &str) -> String {
    let suffix: String = uuid::Uuid::now_v7()
        .simple()
        .to_string()
        .chars()
        .filter(|character| *character != '0')
        .take(10)
        .collect();
    format!("{prefix}{suffix}")
}

/// A distinct E.164 number per run, inside the reserved 555 test range.
fn unique_phone() -> String {
    let digits = uuid::Uuid::now_v7().as_u128() % 10_000_000;
    format!("+1415{digits:07}")
}

/// Signs a fresh Carbon up and logs it in, returning an authenticated client.
///
/// `fixed_code` is `Some("000000")` inside a testing environment, which
/// delivers nothing and accepts that code in place of a delivered one.
async fn enrol(anonymous: &Client, handle: &str, fixed_code: Option<&str>) -> Client {
    let session = anonymous
        .signup()
        .start(&Mutation::new())
        .await
        .expect("a signup session")
        .session_id;

    let email = format!("{handle}@example.test");
    let phone = unique_phone();

    let dispatched = anonymous
        .signup()
        .send_email_code(session, &email, &Mutation::new())
        .await
        .expect("an email code");
    let code = code_for(fixed_code, dispatched.local_otp);
    anonymous
        .signup()
        .verify_email(session, &code, &Mutation::new())
        .await
        .expect("the emailed code verifies");

    let dispatched = anonymous
        .signup()
        .send_phone_code(session, &phone, &Mutation::new())
        .await
        .expect("a phone code");
    let code = code_for(fixed_code, dispatched.local_otp);
    anonymous
        .signup()
        .verify_phone(session, &code, &Mutation::new())
        .await
        .expect("the texted code verifies");

    anonymous
        .signup()
        .complete(
            session,
            &models::CarbonSignupComplete {
                carbon_id: handle.to_owned(),
                display_name: handle.to_owned(),
                timezone: None,
                profile_photo: None,
            },
            &Mutation::new(),
        )
        .await
        .expect("the Carbon is created");

    let challenge = anonymous
        .auth()
        .start_login(
            &models::LoginChallengeCreate {
                email: Some(email),
                phone_number: None,
                carbon_id: None,
            },
            &Mutation::new(),
        )
        .await
        .expect("a login challenge");
    let code = code_for(fixed_code, challenge.local_otp);
    let tokens = anonymous
        .auth()
        .verify_login(challenge.session_id, &code, &Mutation::new())
        .await
        .expect("the login code verifies");

    anonymous.with_credential(Credential::bearer(tokens.access_token))
}

#[tokio::test]
#[ignore = "needs a running Silicon IAM"]
async fn the_client_speaks_the_contract_end_to_end() {
    let Some(anonymous) = service() else {
        eprintln!("set SILICON_IAM_LIVE_URL to run this");
        return;
    };

    // The version handshake, before anything else depends on it.
    let negotiated = anonymous
        .system()
        .negotiate()
        .await
        .expect("a mutually supported version");
    assert_eq!(
        negotiated.selected_api_version,
        silicon_iam_client::API_VERSION
    );
    anonymous
        .system()
        .readiness()
        .await
        .expect("a ready service");

    let handle = unique("live");
    let client = enrol(&anonymous, &handle, None).await;

    let me = client.carbons().me().await.expect("the caller's profile");
    assert_eq!(me.carbon_id, handle);

    // A profile update, which exercises merge-patch and If-Match together.
    let renamed = client
        .carbons()
        .update_me(
            me.version,
            &models::CarbonProfilePatch {
                display_name: Some("Renamed".to_owned()),
                timezone: None,
                profile_photo: None,
            },
            &Mutation::new(),
        )
        .await
        .expect("the profile updates");
    assert_eq!(renamed.display_name, "Renamed");
    assert!(renamed.version > me.version);

    // A stale version has to be refused, or optimistic concurrency is a lie.
    let stale = client
        .carbons()
        .update_me(
            me.version,
            &models::CarbonProfilePatch {
                display_name: Some("Again".to_owned()),
                timezone: None,
                profile_photo: None,
            },
            &Mutation::new(),
        )
        .await;
    let Err(error) = stale else {
        panic!("a stale version must be refused");
    };
    assert!(
        error
            .api()
            .is_some_and(silicon_iam_client::ApiError::is_version_conflict),
        "{error}"
    );

    let org_id = unique("org");
    let availability = client
        .organizations()
        .handle_available(&org_id)
        .await
        .expect("an availability answer");
    assert!(availability.available);

    let organization = client
        .organizations()
        .create(
            &models::OrganizationCreate {
                org_id: org_id.clone(),
                name: "Live Test".to_owned(),
                logo: None,
                description: None,
            },
            &Mutation::new(),
        )
        .await
        .expect("the organization is created");
    assert_eq!(organization.org_id, org_id);

    // Listing, which exercises the paged shape.
    let listed = client
        .organizations()
        .list(&Paging::new().limit(10))
        .await
        .expect("the caller's organizations");
    assert!(listed.items.iter().any(|entry| entry.org_id == org_id));

    // Application responses carry OffsetDateTime fields that the generated
    // client accepts only as RFC3339 strings. This create -> list -> get walk
    // therefore guards both the public timestamp encoding and the tenant /
    // application RLS context each read needs after an application exists.
    let application_handle = unique("app");
    let qualified_app_id = format!("{org_id}>{application_handle}");
    let created_application = client
        .applications()
        .create(
            &models::ApplicationCreate {
                app_scope: None,
                webhook_scope: None,
                obo_review_message: None,
                testing_idle_days: None,
                app_id: application_handle,
                org_id: org_id.clone(),
                app_name: Some("Live Application".to_owned()),
                app_logo: None,
                webhook_url: "https://hooks.example.test/iam".to_owned(),
                webhook_secret: "live-client-webhook-secret-00001".to_owned(),
                base_url: "https://application.example.test".to_owned(),
                obo_endpoints: None,
            },
            &Mutation::new(),
        )
        .await
        .expect("the application is created and its timestamps decode");
    assert_eq!(created_application.application.app_id, qualified_app_id);
    assert!(
        created_application.application.updated_at >= created_application.application.created_at
    );
    assert!(
        created_application.secret_replay_expires_at > created_application.application.created_at
    );

    let applications = client
        .applications()
        .list(None, &Paging::new().limit(10))
        .await
        .expect("the newly created application lists through RLS");
    let listed_application = applications
        .items
        .iter()
        .find(|application| application.app_id == qualified_app_id)
        .expect("the application is present in its owner's list");
    assert_eq!(listed_application.id, created_application.application.id);

    // These list routes share the organization-filtered HTTP query parser.
    // Exercise numeric pagination through the real API, not just a mock server.
    let filtered = client
        .applications()
        .list_for_organization(&org_id, None, &Paging::new().limit(10))
        .await
        .expect("application organization filtering accepts numeric pagination");
    assert_eq!(filtered.items.len(), 1);
    assert_eq!(filtered.items[0].id, created_application.application.id);
    let availability = client
        .bundles()
        .availability(&org_id)
        .await
        .expect("a current organization owner can read bundle availability");
    assert!(!availability.available);
    let bundles = client
        .bundles()
        .list_for_organization(&org_id, &Paging::new().limit(10))
        .await
        .expect("bundle organization filtering accepts numeric pagination");
    assert!(bundles.items.is_empty());
    assert!(!bundles.page.has_more);

    let fetched_application = client
        .applications()
        .get(&qualified_app_id)
        .await
        .expect("the newly created application reads through RLS");
    assert_eq!(fetched_application.id, created_application.application.id);
    assert_eq!(
        fetched_application.created_at,
        listed_application.created_at
    );

    // The Application-login contract is form encoded and starts only from an
    // IAM-issued SLT with explicit organization consent. Walk the whole token
    // lifecycle so media-type drift, application Basic auth, token RLS, retry
    // semantics, and revocation are all exercised against the real service.
    let consent = client
        .auth()
        .login_organizations(&qualified_app_id)
        .await
        .expect("scope consent");
    let short_lived = client
        .auth()
        .short_lived_token_for_organizations(
            &qualified_app_id,
            std::slice::from_ref(&org_id),
            consent.scope_version,
            &consent
                .scopes
                .iter()
                .map(|scope| scope.scope.clone())
                .collect::<Vec<_>>(),
            &Mutation::new(),
        )
        .await
        .expect("an IAM-issued short-lived token");
    let application_client = anonymous.with_credential(Credential::application(
        qualified_app_id.clone(),
        created_application.app_secret.clone(),
    ));

    let login_mutation = Mutation::new();
    let application_tokens = application_client
        .oauth()
        .login(&qualified_app_id, &short_lived.slt, &login_mutation)
        .await
        .expect("the Application exchanges an SLT as a form request");
    let replayed_tokens = application_client
        .oauth()
        .login(&qualified_app_id, &short_lived.slt, &login_mutation)
        .await
        .expect("an exact token-exchange retry replays safely");
    assert_eq!(
        replayed_tokens.access_token,
        application_tokens.access_token
    );
    assert_eq!(
        replayed_tokens.refresh_token,
        application_tokens.refresh_token
    );

    let spent_slt = application_client
        .oauth()
        .login(&qualified_app_id, &short_lived.slt, &Mutation::new())
        .await;
    let Err(error) = spent_slt else {
        panic!("a spent SLT must not start another Application session");
    };
    assert_eq!(
        error.api().map(|api| api.code.as_str()),
        Some("invalid_grant"),
        "{error}"
    );

    let access_introspection = application_client
        .oauth()
        .introspect(
            &models::TokenIntrospectionRequest {
                token: application_tokens.access_token.clone(),
                token_type_hint: Some(models::TokenIntrospectionRequestTokenTypeHint::AccessToken),
            },
            None,
        )
        .await
        .expect("the Application introspects its access token through RLS");
    assert!(access_introspection.active);
    assert_eq!(
        access_introspection.client_id.as_deref(),
        Some(qualified_app_id.as_str())
    );
    assert_eq!(access_introspection.org_id, None);

    let refresh_mutation = Mutation::new();
    let refreshed_tokens = application_client
        .oauth()
        .refresh(
            &qualified_app_id,
            &application_tokens.refresh_token,
            &refresh_mutation,
        )
        .await
        .expect("the Application rotates its refresh token as a form request");
    let replayed_refresh = application_client
        .oauth()
        .refresh(
            &qualified_app_id,
            &application_tokens.refresh_token,
            &refresh_mutation,
        )
        .await
        .expect("an exact refresh retry replays safely");
    assert_eq!(replayed_refresh.access_token, refreshed_tokens.access_token);
    assert_eq!(
        replayed_refresh.refresh_token,
        refreshed_tokens.refresh_token
    );

    application_client
        .oauth()
        .revoke(
            &models::OAuthRevocationRequest {
                token: refreshed_tokens.refresh_token.clone(),
                token_type_hint: Some(models::OAuthRevocationRequestTokenTypeHint::RefreshToken),
            },
            &Mutation::new(),
        )
        .await
        .expect("the Application revokes a refresh family as a form request");
    let revoked_refresh = application_client
        .oauth()
        .introspect(
            &models::TokenIntrospectionRequest {
                token: refreshed_tokens.refresh_token,
                token_type_hint: Some(models::TokenIntrospectionRequestTokenTypeHint::RefreshToken),
            },
            None,
        )
        .await
        .expect("a revoked token introspects as inactive, not as an error");
    assert!(!revoked_refresh.active);
    application_client
        .oauth()
        .revoke(
            &models::OAuthRevocationRequest {
                token: refreshed_tokens.access_token.clone(),
                token_type_hint: Some(models::OAuthRevocationRequestTokenTypeHint::AccessToken),
            },
            &Mutation::new(),
        )
        .await
        .expect("the Application revokes an access token as a form request");
    let revoked_access = application_client
        .oauth()
        .introspect(
            &models::TokenIntrospectionRequest {
                token: refreshed_tokens.access_token,
                token_type_hint: Some(models::TokenIntrospectionRequestTokenTypeHint::AccessToken),
            },
            None,
        )
        .await
        .expect("access-token revocation is immediately visible to introspection");
    assert!(!revoked_access.active);

    // Tags, through their whole lifecycle including the cascade on delete.
    let tag = client
        .tags()
        .create(
            &org_id,
            &models::TagCreate {
                name: "Engineering".to_owned(),
            },
            &Mutation::new(),
        )
        .await
        .expect("the tag is created");

    let membership_id = organization.owner_membership_id;
    let member = client
        .members()
        .get(&org_id, &membership_id)
        .await
        .expect("the owner membership");
    client
        .governance()
        .replace_tags(
            &org_id,
            &membership_id,
            member.version,
            &models::DirectTagSetReplace {
                tag_ids: vec![tag.id],
            },
            &Mutation::new(),
        )
        .await
        .expect("the tag is assigned");

    let tagged = client
        .members()
        .list(
            &org_id,
            &MemberFilter {
                tag_id: Some(tag.id),
                ..MemberFilter::default()
            },
            &Paging::new(),
        )
        .await
        .expect("members carrying the tag");
    assert_eq!(tagged.items.len(), 1);

    client
        .tags()
        .delete(&org_id, tag.id, tag.version, &Mutation::new())
        .await
        .expect("the tag is deleted");

    let gone = client.tags().get(&org_id, tag.id).await;
    let Err(error) = gone else {
        panic!("a deleted tag must be gone");
    };
    assert!(
        error
            .api()
            .is_some_and(silicon_iam_client::ApiError::is_not_found),
        "{error}"
    );

    // The cascade: the member no longer carries it.
    let after = client
        .members()
        .get(&org_id, &membership_id)
        .await
        .expect("the owner membership");
    assert!(after.tags.is_empty());

    // Approvals list cleanly even when empty.
    let approvals = client
        .governance()
        .list_approvals(&org_id, &ApprovalFilter::actionable(), &Paging::new())
        .await
        .expect("the approval queue");
    assert!(!approvals.page.has_more);
}

#[tokio::test]
#[ignore = "needs a running Silicon IAM with a testing database"]
async fn a_testing_environment_is_the_same_api_against_its_own_data() {
    let Some(anonymous) = service() else {
        eprintln!("set SILICON_IAM_LIVE_URL to run this");
        return;
    };

    let handle = unique("env");
    let client = enrol(&anonymous, &handle, None).await;

    let org_id = unique("envorg");
    client
        .organizations()
        .create(
            &models::OrganizationCreate {
                org_id: org_id.clone(),
                name: "Environment Test".to_owned(),
                logo: None,
                description: None,
            },
            &Mutation::new(),
        )
        .await
        .expect("the organization is created");

    let created = match client
        .environments()
        .create(
            &org_id,
            &models::TestingEnvironmentCreate {
                name: "Sandbox".to_owned(),
                description: None,
            },
            &Mutation::new(),
        )
        .await
    {
        Ok(created) => created,
        Err(error) => {
            eprintln!("no testing database configured; skipping: {error}");
            return;
        }
    };

    let key = silicon_iam_client::EnvironmentKey::new(created.key.clone())
        .expect("the service returns a well-formed key");
    let sandbox = client.with_environment(key);

    // The key alone describes the environment it opens.
    let described = sandbox
        .environments()
        .current()
        .await
        .expect("the environment describes itself");
    assert_eq!(described.name, "Sandbox");

    // A production token is worthless inside the environment. This is the
    // isolation the whole feature rests on, so it is asserted before anything
    // that would depend on it.
    let refused = sandbox.carbons().me().await;
    let Err(error) = refused else {
        panic!("a production token must not authenticate inside an environment");
    };
    assert_eq!(error.api().map(|api| api.status), Some(401), "{error}");

    // An environment is the same API, so it is entered the same way: sign up
    // inside it. Nothing is delivered there, and 000000 stands in for a
    // delivered code.
    let inside_handle = unique("inenv");
    let inside = enrol(
        &anonymous.with_environment(
            silicon_iam_client::EnvironmentKey::new(created.key.clone())
                .expect("a well-formed key"),
        ),
        &inside_handle,
        Some("000000"),
    )
    .await;

    // The same Carbon handle exists in both planes without collision, and the
    // environment starts with nothing in it.
    let inside_me = inside
        .carbons()
        .me()
        .await
        .expect("the environment profile");
    assert_eq!(inside_me.carbon_id, inside_handle);
    let organizations = inside
        .organizations()
        .list(&Paging::new())
        .await
        .expect("organizations inside the environment");
    assert!(
        organizations.items.is_empty(),
        "a fresh environment must start empty"
    );

    // Production's organization handle is free inside the environment.
    let availability = inside
        .organizations()
        .handle_available(&org_id)
        .await
        .expect("an availability answer");
    assert!(
        availability.available,
        "an environment must not see production's handles"
    );

    // Meanwhile the environment itself is visible from production.
    let outside = client
        .environments()
        .get(&org_id, created.id)
        .await
        .expect("the environment is visible from production");
    assert_eq!(outside.status, models::TestingEnvironmentStatus::Active);

    let cleaning = sandbox
        .environments()
        .clean_current(&Mutation::new())
        .await
        .expect("the environment cleans itself");
    assert_eq!(cleaning.environment_id, created.id);

    let retired = client
        .environments()
        .delete(&org_id, created.id, &Mutation::new())
        .await
        .expect("the environment retires");
    assert_eq!(retired.status, models::TestingEnvironmentStatus::Deleted);
    assert!(retired.purge_after.is_some());

    // The key stops working the moment the environment is retired.
    let refused = sandbox.environments().current().await;
    assert!(
        refused.is_err(),
        "a retired environment must refuse its key"
    );
}

/// Batch authorization uses the same real application secrets and token exchange
/// as single login. Failed batches must not persist even the first app's consent.
#[tokio::test]
#[ignore = "needs a running disposable Silicon IAM"]
async fn batch_login_is_atomic_and_application_bound() {
    let Some(anonymous) = service() else {
        return;
    };
    let client = enrol(&anonymous, &unique("batch"), None).await;
    exercise_batch_login(&anonymous, &client).await;

    // When the isolated test plane is configured, repeat the exact protocol
    // there; production credentials must not authorize its batch endpoints.
    if std::env::var("SILICON_IAM_LIVE_BATCH_TEST_PLANE").is_ok() {
        let org = unique("batchenv");
        client
            .organizations()
            .create(
                &models::OrganizationCreate {
                    org_id: org.clone(),
                    name: "Batch test environment owner".to_owned(),
                    logo: None,
                    description: None,
                },
                &Mutation::new(),
            )
            .await
            .expect("environment owner org");
        let created = client
            .environments()
            .create(
                &org,
                &models::TestingEnvironmentCreate {
                    name: "Batch isolation".to_owned(),
                    description: None,
                },
                &Mutation::new(),
            )
            .await
            .expect("test environment");
        let key = silicon_iam_client::EnvironmentKey::new(created.key).expect("environment key");
        assert!(
            client
                .with_environment(key.clone())
                .auth()
                .batch_login_organizations(&["app".to_owned()])
                .await
                .is_err()
        );
        let sandbox = anonymous.with_environment(key);
        let inside = enrol(&sandbox, &unique("batchinside"), Some("000000")).await;
        exercise_batch_login(&sandbox, &inside).await;
    }
}

async fn exercise_batch_login(anonymous: &Client, client: &Client) {
    let mut orgs = Vec::new();
    for prefix in ["batchorga", "batchorgb"] {
        let org = unique(prefix);
        client
            .organizations()
            .create(
                &models::OrganizationCreate {
                    org_id: org.clone(),
                    name: "Batch organization".to_owned(),
                    logo: None,
                    description: None,
                },
                &Mutation::new(),
            )
            .await
            .expect("create batch organization");
        orgs.push(org);
    }
    let mut apps = Vec::new();
    for handle in ["batch-a", "batch-b"] {
        let created = client
            .applications()
            .create(
                &models::ApplicationCreate {
                    app_scope: Some(models::ApplicationScope {
                        iam: vec![
                            "self.identity.read".to_owned(),
                            "self.organizations.read".to_owned(),
                        ],
                        external: vec![],
                    }),
                    webhook_scope: None,
                    obo_review_message: None,
                    testing_idle_days: None,
                    app_id: handle.to_owned(),
                    org_id: orgs[0].clone(),
                    app_name: Some(handle.to_owned()),
                    app_logo: None,
                    webhook_url: "https://batch.example.test/hooks".to_owned(),
                    webhook_secret: "batch-webhook-secret-at-least-32-characters".to_owned(),
                    base_url: "https://batch.example.test".to_owned(),
                    obo_endpoints: None,
                },
                &Mutation::new(),
            )
            .await
            .expect("create batch application");
        apps.push((created.application.app_id, created.app_secret));
    }
    let ids = apps.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
    let choices = client
        .auth()
        .batch_login_organizations(&ids)
        .await
        .expect("batch choices");
    assert_eq!(choices.items.len(), 2);
    assert!(
        choices
            .items
            .iter()
            .flat_map(|app| &app.items)
            .all(|org| !org.authorized)
    );
    let mut input = models::BatchLoginRequest {
        applications: vec![
            models::BatchLoginSelection {
                app_id: ids[0].clone(),
                scope_version: choices.items[0].scope_version,
                approved_scopes: choices.items[0]
                    .scopes
                    .iter()
                    .map(|scope| scope.scope.clone())
                    .collect(),
                org_ids: vec![orgs[0].clone()],
            },
            models::BatchLoginSelection {
                app_id: ids[1].clone(),
                scope_version: choices.items[1].scope_version,
                approved_scopes: choices.items[1]
                    .scopes
                    .iter()
                    .map(|scope| scope.scope.clone())
                    .collect(),
                org_ids: vec!["missing-org".to_owned()],
            },
        ],
        redirect_uri: None,
    };
    let mutation = Mutation::new();
    assert!(
        client
            .auth()
            .batch_short_lived_tokens(&input, &mutation)
            .await
            .is_err()
    );
    let choices = client
        .auth()
        .batch_login_organizations(&ids)
        .await
        .expect("choices after rollback");
    assert!(
        choices
            .items
            .iter()
            .flat_map(|app| &app.items)
            .all(|org| !org.authorized),
        "failed batch persisted partial consent"
    );
    input.applications[1].org_ids = vec![orgs[1].clone()];
    let result = client
        .auth()
        .batch_short_lived_tokens(&input, &mutation)
        .await
        .expect("failed batch rolled back its request key too");
    assert_eq!(result.items.len(), 2);
    assert_eq!(
        result
            .items
            .iter()
            .map(|item| &item.app_id)
            .collect::<Vec<_>>(),
        ids.iter().collect::<Vec<_>>()
    );
    assert_ne!(result.items[0].slt, result.items[1].slt);
    let replay = client
        .auth()
        .batch_short_lived_tokens(&input, &mutation)
        .await
        .expect("exact batch replay");
    assert_eq!(
        serde_json::to_value(&result).expect("serialize batch"),
        serde_json::to_value(&replay).expect("serialize replay")
    );
    let mut changed = input.clone();
    changed.applications.reverse();
    assert!(
        client
            .auth()
            .batch_short_lived_tokens(&changed, &mutation)
            .await
            .is_err(),
        "different request must not replay"
    );
    let choices = client
        .auth()
        .batch_login_organizations(&ids)
        .await
        .expect("selected choices");
    for (index, app) in choices.items.iter().enumerate() {
        assert_eq!(
            app.items
                .iter()
                .filter(|org| org.authorized)
                .map(|org| org.org_id.clone())
                .collect::<Vec<_>>(),
            vec![orgs[index].clone()]
        );
    }
    let app_a = anonymous.with_credential(Credential::application(
        apps[0].0.clone(),
        apps[0].1.clone(),
    ));
    let app_b = anonymous.with_credential(Credential::application(
        apps[1].0.clone(),
        apps[1].1.clone(),
    ));
    assert!(
        app_b
            .oauth()
            .login(&ids[1], &result.items[0].slt, &Mutation::new())
            .await
            .is_err(),
        "app B must not consume app A's SLT"
    );
    assert!(app_a.auth().batch_login_organizations(&ids).await.is_err());
    for (index, app) in [&app_a, &app_b].iter().enumerate() {
        let tokens = app
            .oauth()
            .login(&ids[index], &result.items[index].slt, &Mutation::new())
            .await
            .expect("each app exchanges only its own SLT");
        assert!(
            app.oauth()
                .login(&ids[index], &result.items[index].slt, &Mutation::new())
                .await
                .is_err(),
            "SLT must be single use"
        );
        let bearer = anonymous.with_credential(Credential::bearer(tokens.access_token.clone()));
        assert!(
            bearer
                .auth()
                .batch_short_lived_tokens(&input, &Mutation::new())
                .await
                .is_err(),
            "an app bearer cannot authorize more apps"
        );
        let authorized = bearer
            .application_reads()
            .organizations(&Paging::new())
            .await
            .expect("selected organization directory");
        assert_eq!(
            authorized["items"]
                .as_array()
                .expect("organization items")
                .len(),
            1
        );
        assert_eq!(authorized["items"][0]["org_id"], orgs[index]);
        assert!(
            bearer
                .application_reads()
                .organization(&orgs[1 - index])
                .await
                .is_err(),
            "an unselected organization must stay inaccessible"
        );
    }
    // A subsequent batch adds an organization without revoking the earlier grant.
    input.applications[0].org_ids = vec![orgs[1].clone()];
    client
        .auth()
        .batch_short_lived_tokens(&input, &Mutation::new())
        .await
        .expect("additive batch");
    let choices = client
        .auth()
        .batch_login_organizations(&ids)
        .await
        .expect("additive choices");
    assert_eq!(
        choices.items[0]
            .items
            .iter()
            .filter(|org| org.authorized)
            .count(),
        2
    );
}

#[tokio::test]
#[ignore = "needs a running disposable Silicon IAM with a testing database"]
async fn application_testing_imports_cycles_and_preserves_obo_authority() {
    let Some(anonymous) = service() else { return };
    let owner = enrol(&anonymous, &unique("apptest"), None).await;
    let mut orgs = Vec::new();
    for prefix in ["testsource", "testtarget"] {
        let org_id = unique(prefix);
        owner
            .organizations()
            .create(
                &models::OrganizationCreate {
                    org_id: org_id.clone(),
                    name: "Application testing owner".to_owned(),
                    logo: None,
                    description: None,
                },
                &Mutation::new(),
            )
            .await
            .expect("application owner organization");
        orgs.push(org_id);
    }
    let mut apps = Vec::new();
    for (index, org) in orgs.iter().enumerate() {
        let app = owner
            .applications()
            .create(
                &models::ApplicationCreate {
                    app_scope: Some(models::ApplicationScope {
                        iam: vec!["self.identity.read".to_owned()],
                        external: vec![],
                    }),
                    webhook_scope: None,
                    obo_review_message: None,
                    testing_idle_days: Some(60),
                    app_id: format!("service{index}"),
                    org_id: org.clone(),
                    app_name: Some("Imported application".to_owned()),
                    app_logo: None,
                    webhook_url: "https://testing.example.test/hooks".to_owned(),
                    webhook_secret: "testing-webhook-secret-at-least-32-characters".to_owned(),
                    base_url: "https://testing.example.test".to_owned(),
                    obo_endpoints: Some(vec![models::ApplicationOboEndpoint {
                        ttl_seconds: Some(900),
                        critical: true,
                        endpoint_id: "operation".to_owned(),
                        path: "/operation".to_owned(),
                        metadata: serde_json::json!({}),
                    }]),
                },
                &Mutation::new(),
            )
            .await
            .expect("application with critical endpoint");
        apps.push(app);
    }
    // Both edges are declared, producing a cycle. Approve one review and leave
    // the other pending: test imports must use the desired configuration while
    // production preserves the previous approved version.
    for index in 0..2 {
        let request = owner
            .application_scopes()
            .request(
                &apps[index].application.app_id,
                apps[index].application.version,
                &models::ApplicationScopeRequestCreate {
                    app_scope: models::ApplicationScope {
                        iam: vec!["self.identity.read".to_owned()],
                        external: vec![models::ApplicationExternalScope {
                            app_id: apps[1 - index].application.app_id.clone(),
                            endpoint_id: "operation".to_owned(),
                        }],
                    },
                    message: "Call the related service for this user.".to_owned(),
                },
                &Mutation::new(),
            )
            .await
            .expect("critical scope review request");
        assert_eq!(request.items.len(), 1);
        let discussion = owner
            .application_scopes()
            .get(request.items[0].id)
            .await
            .expect("scope discussion");
        assert!(discussion.can_decide);
        if index == 1 {
            let replied = owner
                .application_scopes()
                .reply(
                    discussion.id,
                    discussion.version,
                    &models::ApplicationScopeMessageCreate {
                        message: "Reviewed exact endpoint.".to_owned(),
                    },
                    &Mutation::new(),
                )
                .await
                .expect("scope discussion reply");
            let decision = owner
                .application_scopes()
                .decide(
                    replied.id,
                    replied.version,
                    &models::ApplicationScopeDecision {
                        decision: models::ApplicationScopeDecisionDecision::Approve,
                        reason: None,
                    },
                    &Mutation::new(),
                )
                .await
                .expect("audience owner approves critical scope");
            assert_eq!(
                decision.status,
                models::ApplicationScopeRequestStatus::Approved
            );
        }
    }
    let app = anonymous.with_credential(Credential::application(
        apps[0].application.app_id.clone(),
        apps[0].app_secret.clone(),
    ));
    let input = models::ApplicationTestingEnvironmentCreate {
        name: "Recursive integration".to_owned(),
        description: None,
        iam_test_key: None,
    };
    let mutation = Mutation::new();
    let created = app
        .applications()
        .create_testing_environment(&input, &mutation)
        .await
        .expect("application creates recursive environment");
    assert_eq!(
        created.dependencies,
        vec![apps[1].application.app_id.clone()]
    );
    assert_ne!(created.app_secret, apps[0].app_secret);
    let replay = app
        .applications()
        .create_testing_environment(&input, &mutation)
        .await
        .expect("exact environment creation replay");
    assert_eq!(
        serde_json::to_value(&created).expect("created environment"),
        serde_json::to_value(replay).expect("replayed environment")
    );
    let reused = app
        .applications()
        .create_testing_environment(
            &models::ApplicationTestingEnvironmentCreate {
                name: input.name,
                description: None,
                iam_test_key: Some(created.iam_test_key.clone()),
            },
            &Mutation::new(),
        )
        .await
        .expect("cyclic import reuses existing environment");
    assert_eq!(created.environment_id, reused.environment_id);
    assert_eq!(created.app_secret, reused.app_secret);
    let listed = app
        .applications()
        .testing_environments(None, &Paging::new().limit(1))
        .await
        .expect("application environment list");
    assert_eq!(listed.items[0].environment_id, created.environment_id);
    assert_eq!(listed.items[0].retention_days, 60);
    let wrong_owner = anonymous.with_credential(Credential::application(
        apps[1].application.app_id.clone(),
        apps[1].app_secret.clone(),
    ));
    assert!(
        wrong_owner
            .applications()
            .create_testing_environment(
                &models::ApplicationTestingEnvironmentCreate {
                    name: "Wrong owner".to_owned(),
                    description: None,
                    iam_test_key: Some(created.iam_test_key.clone()),
                },
                &Mutation::new()
            )
            .await
            .is_err()
    );

    let key = silicon_iam_client::EnvironmentKey::new(created.iam_test_key.clone())
        .expect("environment key");
    let sandbox = anonymous.with_environment(key.clone());
    let caller = sandbox.with_credential(Credential::application(
        created.app_id.clone(),
        created.app_secret.clone(),
    ));
    let viewed = caller
        .applications()
        .testing_context()
        .await
        .expect("test credential selects own context");
    assert_eq!(viewed.environment_id, created.environment_id);
    assert_eq!(viewed.application.app_id, created.app_id);
    let discovered = caller
        .without_environment()
        .with_testing_application(&created.app_id, &created.app_secret)
        .expect("test selector");
    let context = discovered
        .applications()
        .testing_context()
        .await
        .expect("secret-only discovery");
    assert_eq!(context.environment_id, created.environment_id);
    assert_eq!(
        context.environment.as_ref().expect("IAM metadata").org_id,
        orgs[0]
    );
    assert!(
        discovered.signup().start(&Mutation::new()).await.is_err(),
        "test app secret is not signup/root authority"
    );
    assert!(
        app.with_testing_application(&created.app_id, &apps[0].app_secret)
            .expect("shape")
            .applications()
            .testing_context()
            .await
            .is_err(),
        "production secrets never select testing"
    );
    assert!(
        discovered
            .with_credential(Credential::application(
                &apps[1].application.app_id,
                &created.app_secret
            ))
            .applications()
            .testing_context()
            .await
            .is_err(),
        "different application rejected"
    );

    assert!(
        app.applications().testing_context().await.is_err(),
        "no test view in production"
    );
    assert!(
        app.with_environment(key.clone())
            .applications()
            .testing_context()
            .await
            .is_err(),
        "production secret cannot enter testing"
    );
    assert!(
        wrong_owner
            .environments()
            .key(&orgs[0], created.environment_id)
            .await
            .is_err()
    );
    assert!(
        caller
            .environments()
            .delete(&orgs[0], created.environment_id, &Mutation::new())
            .await
            .is_err(),
        "test credentials cannot delete a production control record"
    );
    let catalog = caller
        .obo()
        .endpoints(&apps[1].application.app_id)
        .await
        .expect("imported cross-organization audience");
    assert!(catalog.endpoints[0].critical);
    let handle = unique("testsubject");
    let user = enrol(&sandbox, &handle, Some("000000")).await;
    let user_org = unique("testsubjectorg");
    user.organizations()
        .create(
            &models::OrganizationCreate {
                org_id: user_org.clone(),
                name: "Subject organization".to_owned(),
                logo: None,
                description: None,
            },
            &Mutation::new(),
        )
        .await
        .expect("subject organization differs from both applications");
    let choices = user
        .auth()
        .login_organizations(&created.app_id)
        .await
        .expect("test scopes");
    assert!(
        choices
            .scopes
            .iter()
            .any(|scope| scope.scope.starts_with("obo:"))
    );
    let slt = user
        .auth()
        .short_lived_token_for_organizations(
            &created.app_id,
            std::slice::from_ref(&user_org),
            choices.scope_version,
            &choices
                .scopes
                .iter()
                .map(|scope| scope.scope.clone())
                .collect::<Vec<_>>(),
            &Mutation::new(),
        )
        .await
        .expect("test subject grants exact scopes");
    let tokens = discovered
        .oauth()
        .login(&created.app_id, &slt.slt, &Mutation::new())
        .await
        .expect("imported app exchanges subject SLT");
    let bearer = discovered.with_credential(Credential::bearer(tokens.access_token.clone()));
    let profile = bearer
        .application_reads()
        .me()
        .await
        .expect("identity-only self read");
    assert_eq!(profile["carbon_id"], handle);
    for field in ["email", "phone_number", "display_name", "profile_photo"] {
        assert!(profile.get(field).is_none(), "undeclared field {field}");
    }
    assert!(
        bearer
            .application_reads()
            .organizations(&Paging::new())
            .await
            .is_err()
    );
    let denied_directory = bearer
        .application_reads()
        .members(&user_org, &Paging::new())
        .await
        .expect_err("unapproved directory scope is forbidden");
    assert!(
        matches!(denied_directory, silicon_iam_client::Error::Api(ref error) if error.status == 403),
        "parameterized scoped routes must reject missing scopes, not fail path extraction: {denied_directory:?}"
    );
    assert!(
        bearer
            .application_reads()
            .organization(&orgs[0])
            .await
            .is_err()
    );
    let digest = silicon_iam_client::api::obo::body_sha256(b"{\"operation\":1}");
    let exchange = models::OboExchangeRequest {
        org_id: None,
        subject_token: tokens.access_token,
        audience: apps[1].application.app_id.clone(),
        endpoint_id: "operation".to_owned(),
        metadata: serde_json::json!({}),
        request: models::OboExchangeRequestBinding {
            method: "POST".to_owned(),
            body_sha256: digest.clone(),
        },
    };
    let proof = caller
        .obo()
        .exchange_signed(&exchange, &catalog, &Mutation::new())
        .await
        .expect("declared and consented cross-org test OBO");
    assert_eq!(
        proof.expires_in, 900,
        "test imports preserve provider lifetime"
    );
    let context = proof.testing_context.expect("audience test credentials");
    assert_eq!(context.app_id, apps[1].application.app_id);
    assert_ne!(context.app_secret, apps[1].app_secret);
    assert_eq!(context.iam_test_key, created.iam_test_key);
    let audience =
        sandbox.with_credential(Credential::application(context.app_id, context.app_secret));
    let verification = models::OboVerifyRequest {
        access_proof: proof.access_proof,
        request: models::OboVerifyRequestBinding {
            method: "POST".to_owned(),
            path: "/operation".to_owned(),
            body_sha256: digest,
        },
    };
    let mut wrong_request = verification.clone();
    wrong_request.request.path = "/different".to_owned();
    assert!(audience.obo().verify(&wrong_request).await.is_err());
    let verified = audience
        .obo()
        .verify(&verification)
        .await
        .expect("audience validates exact request");
    assert_eq!(verified.org_id, user_org);
    assert_eq!(verified.issuer_app_id, created.app_id);
    assert_eq!(
        verified.authorization.scopes,
        vec![
            format!("obo:{}:operation", apps[1].application.app_id),
            "self.identity.read".to_owned(),
        ]
    );
    assert!(matches!(
        verified.authorization.actor_type,
        Some(models::ApplicationAuthorizationActorType::Carbon)
    ));
    assert_eq!(
        verified.authorization.public_id.as_deref(),
        Some(handle.as_str())
    );
    assert!(verified.authorization.org_role.is_none());
    assert!(verified.authorization.tags.is_none());
    assert!(
        audience.obo().verify(&verification).await.is_err(),
        "proof is single use"
    );

    // Human imports reuse the same graph implementation. Re-importing after
    // a credential rotation must report and cache the current secret/version.
    let human_environment = owner
        .environments()
        .create(
            &orgs[0],
            &models::TestingEnvironmentCreate {
                name: "Human import".to_owned(),
                description: None,
            },
            &Mutation::new(),
        )
        .await
        .expect("human-owned environment");
    let human_key =
        silicon_iam_client::EnvironmentKey::new(human_environment.key.clone()).expect("human key");
    let human = enrol(
        &anonymous.with_environment(human_key),
        &unique("humanimport"),
        Some("000000"),
    )
    .await;
    let imported = human
        .applications()
        .import_from_production(&created.app_id, &Mutation::new())
        .await
        .expect("human imports dependency cycle");
    let challenge = human
        .auth()
        .start_step_up(
            &models::StepUpChallengeCreate {
                channel: models::StepUpChallengeCreateChannel::Email,
                action: models::StepUpAction::ApplicationClientSecretRotate,
                resource_id: imported.application.id.clone(),
            },
            &Mutation::new(),
        )
        .await
        .expect("rotation step-up challenge");
    let step_up = human
        .auth()
        .verify_step_up(challenge.session_id, "000000", &Mutation::new())
        .await
        .expect("test rotation step-up");
    let rotated = human
        .applications()
        .rotate_secret(
            &created.app_id,
            imported.application.version,
            &Mutation::new().step_up(step_up.step_up_token),
        )
        .await
        .expect("rotate imported app secret");
    let reimport_key = Mutation::new();
    let reimported = human
        .applications()
        .import_from_production(&created.app_id, &reimport_key)
        .await
        .expect("human reuses import after secret rotation");
    assert_eq!(reimported.application.id, imported.application.id);
    assert_eq!(reimported.application.version, rotated.application_version);
    assert_eq!(reimported.app_secret_version, rotated.app_secret_version);
    assert_eq!(reimported.app_secret, rotated.app_secret);
    let replayed = human
        .applications()
        .import_from_production(&created.app_id, &reimport_key)
        .await
        .expect("re-import replay");
    assert_eq!(replayed.app_secret, rotated.app_secret);
    assert_eq!(replayed.application.version, rotated.application_version);

    app.applications()
        .create_testing_environment(
            &models::ApplicationTestingEnvironmentCreate {
                name: "Attach human environment".to_owned(),
                description: None,
                iam_test_key: Some(human_environment.key.clone()),
            },
            &Mutation::new(),
        )
        .await
        .expect("app attaches using explicit human environment key");
    let attached = app
        .applications()
        .testing_environments(Some("all"), &Paging::new())
        .await
        .expect("human-owned links remain readable");
    assert!(
        !attached
            .items
            .iter()
            .find(|item| item.environment_id == human_environment.id)
            .expect("human link")
            .can_manage
    );

    // Application control uses production credentials and the shared lifecycle.
    assert!(listed.items[0].can_manage);
    assert!(
        app.environments()
            .key(&orgs[0], human_environment.id)
            .await
            .is_err(),
        "application cannot take over a human-created environment"
    );
    let environment_record = app
        .environments()
        .get(&orgs[0], created.environment_id)
        .await
        .expect("owner app reads environment");
    let patch = models::TestingEnvironmentPatch {
        name: Some("Renamed app environment".to_owned()),
        description: Some(Some("SDK lifecycle".to_owned())),
    };
    let edited = app
        .environments()
        .update(
            &orgs[0],
            created.environment_id,
            environment_record.version,
            &patch,
            &Mutation::new(),
        )
        .await
        .expect("owner app edits environment");
    assert_eq!(edited.name, "Renamed app environment");
    assert_eq!(
        discovered
            .applications()
            .testing_context()
            .await
            .expect("metadata refresh")
            .environment
            .expect("metadata")
            .name,
        edited.name
    );

    assert!(
        app.environments()
            .update(
                &orgs[0],
                created.environment_id,
                environment_record.version,
                &patch,
                &Mutation::new()
            )
            .await
            .is_err(),
        "stale version rejected"
    );
    let revealed = app
        .environments()
        .key(&orgs[0], created.environment_id)
        .await
        .expect("owner retrieves key");
    assert_eq!(revealed.key, created.iam_test_key);
    let rotate = Mutation::new();
    let new_key = app
        .environments()
        .rotate_key(&orgs[0], created.environment_id, &rotate)
        .await
        .expect("owner rotates key");
    let replay = app
        .environments()
        .rotate_key(&orgs[0], created.environment_id, &rotate)
        .await
        .expect("rotation replay");
    assert_eq!(new_key.key, replay.key);
    assert_ne!(new_key.key, created.iam_test_key);
    assert!(
        caller.applications().testing_context().await.is_err(),
        "old root key invalidated"
    );
    discovered
        .applications()
        .testing_context()
        .await
        .expect("application discovery survives root rotation");
    let next = caller.with_environment(
        silicon_iam_client::EnvironmentKey::new(new_key.key.clone()).expect("new key"),
    );
    next.applications()
        .testing_context()
        .await
        .expect("rotated key retains test data");
    let deleted = app
        .environments()
        .delete(&orgs[0], created.environment_id, &Mutation::new())
        .await
        .expect("owner deletes environment");
    assert!(deleted.purge_after.is_some());
    assert!(
        discovered.applications().testing_context().await.is_err(),
        "IAM retirement disables discovery"
    );

    assert!(
        next.applications().testing_context().await.is_err(),
        "deleted environment unavailable"
    );
    let retired = app
        .applications()
        .testing_environments(Some("deleted"), &Paging::new().limit(1))
        .await
        .expect("deleted environments discoverable");
    assert_eq!(retired.items[0].environment_id, created.environment_id);
    app.environments()
        .restore(&orgs[0], created.environment_id, &Mutation::new())
        .await
        .expect("owner restores environment");
    next.applications()
        .testing_context()
        .await
        .expect("restore preserves test application");
    discovered
        .applications()
        .testing_context()
        .await
        .expect("IAM restore enables discovery");
    let clean = Mutation::new();
    let cleaned = app
        .environments()
        .clean(&orgs[0], created.environment_id, &clean)
        .await
        .expect("owner cleans entire environment");
    assert!(cleaned.erased_rows > 0);
    assert!(
        discovered.applications().testing_context().await.is_err(),
        "IAM clean invalidates old app selector"
    );
    let replay = app
        .environments()
        .clean(&orgs[0], created.environment_id, &clean)
        .await
        .expect("clean replay");
    assert_eq!(cleaned.erased_rows, replay.erased_rows);
    assert!(
        next.applications().testing_context().await.is_err(),
        "clean removed imported app"
    );
    assert_eq!(
        app.environments()
            .key(&orgs[0], created.environment_id)
            .await
            .expect("clean retains key")
            .key,
        new_key.key
    );
    human
        .applications()
        .get(&created.app_id)
        .await
        .expect("other environment untouched");
    owner
        .applications()
        .get(&created.app_id)
        .await
        .expect("production application untouched");
}
