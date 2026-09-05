use super::*;
use crate::providers::{BrowserUseV3, test_http::spawn_json_server};
use silicon_browser_shared::UsageLimits;

/// Exercise the real adapter and authenticated route against fake account HTTP.
/// No provider account, session, upgrade, or payment is touched.
#[tokio::test]
async fn usage_limits_contract_coalesces_checks_and_observes_upgraded_capacity_after_refresh() {
    let account = |limit| {
        json!({
            "concurrentSessionLimit": limit,
            "rateLimit": 3,
            "activeSessionCount": 42,
            "balance": "private-balance",
            "projectId": "other-organizations-account",
            "planInfo": {"planName": "private-plan"}
        })
        .to_string()
    };
    let (base, mut captured, server) = spawn_json_server(vec![(200, account(3)), (200, account(50))]).await;
    let mut state = fixture().await.state;
    state.browser = Arc::new(BrowserUseV3::with_base_url("private-test-key", &base).unwrap());
    let app = router(state);
    let uri = "/api/v1/usage/limits";
    assert_eq!(request(&app, "GET", uri, None, None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(request(&app, "GET", uri, Some(("oat_owner", "wrong-org")), None).await.0, StatusCode::UNAUTHORIZED);
    assert!(captured.try_recv().is_err(), "unauthorized callers must never check the account");

    let mut callers = tokio::task::JoinSet::new();
    for index in 0..32 {
        let app = app.clone();
        callers.spawn(async move {
            let token = if index % 2 == 0 { "oat_owner" } else { "oat_viewer" };
            request(&app, "GET", uri, Some((token, "org-1")), None).await
        });
    }
    let mut first = None;
    while let Some(response) = callers.join_next().await {
        let (status, headers, body) = response.unwrap();
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        assert_eq!(headers["cache-control"], "no-store");
        let value = data(&body);
        let limits: UsageLimits = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(limits.concurrent_browser_limit, 3);
        assert_eq!(limits.rate_limit, Some(3));
        assert!(limits.checked_at <= Utc::now());
        assert_eq!(value.as_object().unwrap().len(), 3, "no private account fields may escape");
        if let Some(first) = &first {
            assert_eq!(first, &limits, "cache hits preserve the original checked_at");
        } else {
            first = Some(limits);
        }
        let wire = String::from_utf8(body).unwrap();
        for forbidden in ["private-", "activeSessionCount", "projectId", "planInfo", "browser-use"] {
            assert!(!wire.contains(forbidden), "unexpected account disclosure: {forbidden}");
        }
    }
    let check = captured.recv().await.unwrap();
    assert_eq!(check.method, "GET");
    assert_eq!(check.target, "/api/v3/billing/account");
    assert!(check.headers.to_ascii_lowercase().contains("x-browser-use-api-key: private-test-key"));
    assert!(check.body.is_empty());
    assert!(captured.try_recv().is_err(), "concurrent callers must share one account check");

    // Advance only the in-process cache clock; the HTTP fixture remains local.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::time::resume();
    let (status, _, body) = request(&app, "GET", uri, Some(("oat_owner", "org-1")), None).await;
    assert_eq!(status, StatusCode::OK);
    let upgraded: UsageLimits = serde_json::from_value(data(&body)).unwrap();
    assert_eq!(upgraded.concurrent_browser_limit, 50);
    assert!(upgraded.checked_at >= first.unwrap().checked_at);
    assert_eq!(captured.recv().await.unwrap().target, "/api/v3/billing/account");
    server.await.unwrap();
}

#[tokio::test]
async fn usage_limits_failures_are_suppressed_briefly_and_do_not_return_stale_capacity() {
    let (base, mut captured, server) = spawn_json_server(vec![
        (200, json!({"concurrentSessionLimit":3}).to_string()),
        (503, json!({"balance":"private-balance", "apiKey":"private-key"}).to_string()),
        (200, json!({"concurrentSessionLimit":0,"rateLimit":null}).to_string()),
    ])
    .await;
    let mut state = fixture().await.state;
    state.browser = Arc::new(BrowserUseV3::with_base_url("test-key", &base).unwrap());
    let app = router(state);
    let uri = "/api/v1/usage/limits";
    let auth = Some(("oat_owner", "org-1"));
    let (status, _, body) = request(&app, "GET", uri, auth, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(data(&body)["rate_limit"].is_null(), "missing rate interval must not be invented");
    captured.recv().await.unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::time::resume();
    for _ in 0..8 {
        let (status, headers, body) = request(&app, "GET", uri, auth, None).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(headers["cache-control"], "no-store");
        assert_eq!(headers["retry-after"], "5");
        let error: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["error"]["code"], "usage_limits_unavailable");
        assert!(error.get("data").is_none(), "expired limits must not masquerade as current");
        assert!(!String::from_utf8(body).unwrap().contains("private-"));
    }
    captured.recv().await.unwrap();
    assert!(captured.try_recv().is_err());
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(6)).await;
    tokio::time::resume();
    let (status, _, body) = request(&app, "GET", uri, auth, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(data(&body)["concurrent_browser_limit"], 0);
    assert!(data(&body)["rate_limit"].is_null());
    captured.recv().await.unwrap();
    server.await.unwrap();
}
