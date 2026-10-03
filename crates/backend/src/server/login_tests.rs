use super::*;

async fn start_login(fixture: &Fixture, kind: &str) -> Value {
    let (status, headers, body) =
        request(&fixture.app, "POST", "/api/v1/auth/login-attempts", None, Some(json!({"identity_kind":kind}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["cache-control"], "no-store");
    let attempt = data(&body);
    assert_eq!(attempt["identity_kind"], kind);
    assert_eq!(attempt["state"].as_str().unwrap().len(), 64);
    attempt
}

#[tokio::test]
async fn login_attempt_binds_state_kind_and_exact_retry_across_router_restart() {
    for (name, kind, actor) in
        [("carbon", IdentityKind::Carbon, "c:viewer-1"), ("silicon", IdentityKind::Silicon, "si:owner-1")]
    {
        let fixture = fixture().await;
        let attempt = start_login(&fixture, name).await;
        let id = attempt["attempt_id"].as_str().unwrap();
        let route = format!("/api/v1/auth/login-attempts/{id}/complete");
        let body = json!({"short_lived_token":"oac_login_first", "state":attempt["state"]});
        let (status, _, _) = request(
            &fixture.app,
            "POST",
            &route,
            None,
            Some(json!({"short_lived_token":"oac_login_first","state":"f".repeat(64)})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let bound: Option<String> = sqlx::query_scalar("SELECT token_digest FROM login_attempts WHERE id=?")
            .bind(id)
            .fetch_one(fixture.store.pool())
            .await
            .unwrap();
        assert!(bound.is_none(), "invalid state must not consume the attempt");

        // An uncertain/failed exchange binds the SLT before the network call.
        let (status, _, _) = request(&fixture.app, "POST", &route, None, Some(body.clone())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        fixture.identity.allow_exchange(
            "oac_login_first",
            "",
            ExchangedAuth {
                access_token: "oat_bound".into(),
                refresh_token: "ort_bound".into(),
                identity: principal(actor, kind, true),
                scope: "app".into(),
            },
        );
        let restarted = router(fixture.state.clone());
        let retries = tokio::join!(
            request(&restarted, "POST", &route, None, Some(body.clone())),
            request(&restarted, "POST", &route, None, Some(body.clone()))
        );
        for (status, headers, response) in [retries.0, retries.1] {
            assert_eq!(status, StatusCode::OK);
            assert_eq!(headers["cache-control"], "no-store");
            assert_eq!(data(&response)["identity"]["kind"], name);
        }
        let (status, _, _) = request(
            &restarted,
            "POST",
            &route,
            None,
            Some(json!({"short_lived_token":"oac_login_changed","state":attempt["state"]})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(fixture.identity.exchange_keys(), vec![format!("browser-login-{id}"); 3]);
        let (stored_state, stored_token): (String, String) =
            sqlx::query_as("SELECT state_digest,token_digest FROM login_attempts WHERE id=?")
                .bind(id)
                .fetch_one(fixture.store.pool())
                .await
                .unwrap();
        assert_ne!(stored_state, attempt["state"].as_str().unwrap());
        assert_ne!(stored_token, "oac_login_first");
        sqlx::query("UPDATE login_attempts SET expires_at=0 WHERE id=?")
            .bind(id)
            .execute(fixture.store.pool())
            .await
            .unwrap();
        assert_eq!(request(&restarted, "POST", &route, None, Some(body)).await.0, StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn login_attempt_rejects_wrong_principal_kind_before_returning_tokens() {
    let fixture = fixture().await;
    let attempt = start_login(&fixture, "carbon").await;
    fixture.identity.allow_exchange(
        "oac_wrong_kind",
        "",
        ExchangedAuth {
            access_token: "oat_must_not_escape".into(),
            refresh_token: "ort_must_not_escape".into(),
            identity: principal("si:owner-1", IdentityKind::Silicon, true),
            scope: "app".into(),
        },
    );
    let route = format!("/api/v1/auth/login-attempts/{}/complete", attempt["attempt_id"].as_str().unwrap());
    let body = json!({"short_lived_token":"oac_wrong_kind","state":attempt["state"]});
    let (status, _, response) = request(&fixture.app, "POST", &route, None, Some(body.clone())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let response = String::from_utf8(response).unwrap();
    assert!(response.contains("login_identity_mismatch"));
    assert!(!response.contains("must_not_escape"));
    assert_eq!(request(&fixture.app, "POST", &route, None, Some(body)).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn login_attempt_storage_is_bounded_and_expired_rows_are_pruned() {
    let fixture = fixture().await;
    sqlx::query("WITH RECURSIVE ids(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM ids WHERE n<10000) INSERT INTO login_attempts(id,identity_kind,state_digest,expires_at) SELECT CAST(n AS TEXT),'carbon','digest',? FROM ids")
        .bind(Utc::now().timestamp()+600).execute(fixture.store.pool()).await.unwrap();
    let body = Some(json!({"identity_kind":"carbon"}));
    assert_eq!(
        request(&fixture.app, "POST", "/api/v1/auth/login-attempts", None, body.clone()).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    sqlx::query("UPDATE login_attempts SET expires_at=0").execute(fixture.store.pool()).await.unwrap();
    assert_eq!(request(&fixture.app, "POST", "/api/v1/auth/login-attempts", None, body).await.0, StatusCode::OK);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM login_attempts")
            .fetch_one(fixture.store.pool())
            .await
            .unwrap(),
        1
    );
}
