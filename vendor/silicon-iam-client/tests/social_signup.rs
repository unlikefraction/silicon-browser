//! Provider signup keeps polling authority in request bodies and redacts diagnostics.

#![allow(clippy::expect_used)]

use std::{
    io::{Read as _, Write as _},
    net::TcpListener,
    sync::mpsc,
    thread,
    time::Duration,
};

use serde_json::{Value, json};
use silicon_iam_client::{Client, Mutation, models};
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
async fn social_start_and_status_use_fixed_signup_routes_and_redact_poll_tokens() {
    let id = Uuid::new_v4();
    let (client, capture, server) = service(
        json!({"request_id":id,"authorization_url":"https://accounts.google.com/o/oauth2/v2/auth?state=state","poll_token":"private-poll-capability","expires_at":"2030-01-01T00:00:00Z"}),
    );
    let start = client
        .signup()
        .social_start("google", &Mutation::new())
        .await
        .expect("start");
    let (headers, body) = capture.recv().expect("captured start");
    assert!(headers.starts_with("POST /api/v1/signup/social/google/start HTTP/1.1"));
    assert!(headers.to_ascii_lowercase().contains("idempotency-key:"));
    assert_eq!(body, json!({}));
    assert!(!format!("{start:?}").contains("private-poll-capability"));
    server.join().expect("mock completed");
    let (client, capture, server) =
        service(json!({"status":"verified","signup_session_id":id,"email":"person@example.test"}));
    let input = models::SocialSignupStatusInput {
        request_id: id,
        poll_token: start.poll_token,
    };
    assert!(!format!("{input:?}").contains("private-poll-capability"));
    let status = client
        .signup()
        .social_status("google", &input)
        .await
        .expect("status");
    assert_eq!(status.status, models::SocialSignupStatusStatus::Verified);
    let (headers, body) = capture.recv().expect("captured status");
    assert!(headers.starts_with("POST /api/v1/signup/social/google/status HTTP/1.1"));
    assert!(!headers.contains("private-poll-capability"));
    assert_eq!(
        body,
        json!({"request_id":id,"poll_token":"private-poll-capability"})
    );
    server.join().expect("mock completed");
}

#[tokio::test]
async fn provider_discovery_reports_configuration_and_unknown_providers_fail_before_network() {
    let (client, capture, server) = service(
        json!({"providers":[{"id":"google","enabled":true},{"id":"apple","enabled":false}]}),
    );
    let providers = client.signup().social_providers().await.expect("catalog");
    assert_eq!(providers.providers.len(), 2);
    assert!(!providers.providers[1].enabled);
    assert!(
        capture
            .recv()
            .expect("captured discovery")
            .0
            .starts_with("GET /api/v1/signup/social/providers HTTP/1.1")
    );
    server.join().expect("mock completed");
    assert!(
        client
            .signup()
            .social_start("https://attacker.example", &Mutation::new())
            .await
            .is_err()
    );
}
