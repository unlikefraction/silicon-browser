use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use serde_json::{Value, json};

#[test]
fn packaged_controller_is_found_beside_the_cli_and_keeps_the_version_pin() {
    let package = tempfile::tempdir().unwrap();
    let executable = package.path().join(if cfg!(windows) { "browser.exe" } else { "browser" });
    let controller = package.path().join(silicon_browser::setup::runner_file_name());
    // A browser copy is a real native executable with the wrong controller version on every OS.
    fs::copy(env!("CARGO_BIN_EXE_browser"), &executable).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_browser"), &controller).unwrap();
    let output = Command::new(&executable)
        .env_remove("SB_CONTROLLER_BIN")
        .env("SB_HOME", package.path().join("state"))
        .args(["run", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(&format!("found {}", env!("CARGO_PKG_VERSION"))));
    #[cfg(unix)]
    {
        let aliases = tempfile::tempdir().unwrap();
        let alias = aliases.path().join("browser");
        std::os::unix::fs::symlink(&executable, &alias).unwrap();
        let linked = Command::new(alias)
            .env_remove("SB_CONTROLLER_BIN")
            .env("SB_HOME", package.path().join("state"))
            .args(["run", "--help"])
            .output()
            .unwrap();
        assert!(linked.status.success());
        assert!(String::from_utf8_lossy(&linked.stderr).contains(&format!("found {}", env!("CARGO_PKG_VERSION"))));
    }
    let overridden = Command::new(executable)
        .env("SB_CONTROLLER_BIN", package.path().join("missing-controller"))
        .args(["run", "--help"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&overridden.stderr).contains("no runnable browser controller"));
}

#[derive(Debug)]
struct CapturedRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl CapturedRequest {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("request body must be JSON")
    }
}

struct StubResponse {
    status: u16,
    content_type: &'static str,
    auth_rejected: bool,
    body: Vec<u8>,
}

impl StubResponse {
    fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            auth_rejected: false,
            body: serde_json::to_vec(&body).unwrap(),
        }
    }
}

struct StubServer {
    base_url: String,
    worker: thread::JoinHandle<Vec<CapturedRequest>>,
}

impl StubServer {
    fn start(responses: Vec<StubResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let mut requests = Vec::with_capacity(responses.len());
            for mut response in responses {
                let (mut stream, _) = listener.accept().expect("CLI must connect to the local stub");
                let request = read_request(&mut stream);
                if response.body == b"REPORT_RECEIPT" {
                    response.body =
                        serde_json::to_vec(&json!({"data":{"command_id":request.json()["command_id"],"sequence":1}}))
                            .unwrap();
                }
                requests.push(request);
                write_response(&mut stream, response);
            }
            requests
        });
        Self { base_url, worker }
    }

    fn finish(self) -> Vec<CapturedRequest> {
        self.worker.join().expect("local HTTP stub must finish cleanly")
    }
}

fn read_request(stream: &mut TcpStream) -> CapturedRequest {
    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).expect("stub must read request bytes");
        assert_ne!(count, 0, "connection closed before complete HTTP headers");
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        assert!(bytes.len() <= 64 * 1024, "request headers unexpectedly large");
    };

    let header_text = std::str::from_utf8(&bytes[..header_end - 4]).expect("request headers must be UTF-8");
    let mut lines = header_text.split("\r\n");
    let mut request_line = lines.next().unwrap().split_whitespace();
    let method = request_line.next().unwrap().to_owned();
    let path = request_line.next().unwrap().to_owned();
    let headers = lines
        .map(|line| {
            let (name, value) = line.split_once(':').expect("well-formed request header");
            (name.to_ascii_lowercase(), value.trim().to_owned())
        })
        .collect::<BTreeMap<_, _>>();
    let content_length = headers.get("content-length").and_then(|value| value.parse().ok()).unwrap_or(0);
    while bytes.len() < header_end + content_length {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).expect("stub must read request body");
        assert_ne!(count, 0, "connection closed before complete HTTP body");
        bytes.extend_from_slice(&chunk[..count]);
    }
    CapturedRequest { method, path, headers, body: bytes[header_end..header_end + content_length].to_vec() }
}

fn write_response(stream: &mut TcpStream, response: StubResponse) {
    let reason = match response.status {
        200 => "OK",
        401 => "Unauthorized",
        409 => "Conflict",
        503 => "Service Unavailable",
        status => panic!("unsupported stub status {status}"),
    };
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",
        response.status,
        reason,
        response.content_type,
        response.body.len(),
        if response.auth_rejected { "X-SB-Auth-Rejected: 1\r\n" } else { "" }
    )
    .unwrap();
    stream.write_all(&response.body).unwrap();
    stream.flush().unwrap();
}

fn isolated_sb(home: &Path, backend: &str) -> assert_cmd::Command {
    let mut command = cargo_bin_cmd!("browser");
    command
        .env_clear()
        .env("SB_HOME", home)
        .env("SB_BACKEND_URL", backend)
        .env("SB_AUTHTOKEN", "jwt.binary_contract.signature");
    command
}

#[test]
fn silicon_home_contains_state_unless_sb_home_overrides_it() {
    for override_home in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let silicon_home = directory.path().join("silicon home");
        fs::create_dir(&silicon_home).unwrap();
        let sb_home = directory.path().join("override");
        let mut command = isolated_sb(&sb_home, "http://127.0.0.1:9");
        command.env("SILICON_HOME", &silicon_home).env("SB_AUTHTOKEN", "invalid");
        if !override_home {
            command.env_remove("SB_HOME");
        }
        command
            .arg("setup")
            .assert()
            .failure()
            .stderr(predicate::str::contains("SB_AUTHTOKEN must use Silicon Accounts"));
        assert_eq!(silicon_home.join(".silicon-browser/backends").is_dir(), !override_home);
        assert_eq!(sb_home.join("backends").is_dir(), override_home);
    }
}

#[cfg(unix)]
fn install_fake_runner(directory: &Path) {
    use std::os::unix::fs::PermissionsExt as _;

    fs::create_dir_all(directory).unwrap();
    let runner = directory.join("agent-browser");
    fs::write(
        &runner,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'agent-browser 0.36.0'; exit 0; fi\nif [ \"$1\" = \"install\" ]; then exit 0; fi\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&runner, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn bare_command_uses_environment_auth_and_live_service_discovery() {
    let server = StubServer::start(vec![
        StubResponse::json(
            200,
            json!({
                "data": {
                    "id": "11111111-1111-4111-8111-111111111117",
                    "name": "Ada Browser",
                    "kind": "silicon",
                    "tags": ["engineering"]
                }
            }),
        ),
        StubResponse::json(200, json!({"data": ["remote-browser", "search-and-fetch"]})),
    ]);
    let state_root = tempfile::tempdir().unwrap();

    isolated_sb(&state_root.path().join("state"), &server.base_url)
        .assert()
        .success()
        .stdout("Ada Browser (Silicon)\nservices: remote-browser, search-and-fetch\n")
        .stderr("");

    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!((&requests[0].method, &requests[0].path), (&"GET".into(), &"/api/v1/me".into()));
    assert_eq!((&requests[1].method, &requests[1].path), (&"GET".into(), &"/api/v1/services".into()));
    for request in requests {
        assert_eq!(
            request.headers.get("authorization").map(String::as_str),
            Some("Bearer jwt.binary_contract.signature")
        );
        assert!(!request.headers.contains_key("x-org-id"));
        assert!(request.body.is_empty());
    }
}

#[test]
fn profile_busy_renders_structured_owner_session_and_expiry_details() {
    let server = StubServer::start(vec![StubResponse::json(
        409,
        json!({
            "error": {
                "code": "profile_busy",
                "message": "profile already has a live session",
                "details": {
                    "actor_id": "11111111-1111-4111-8111-111111111117",
                    "session_id": "session-active",
                    "expires_at": "2026-09-04T12:00:00Z"
                },
                "request_id": "request-busy"
            }
        }),
    )]);
    let state_root = tempfile::tempdir().unwrap();

    isolated_sb(&state_root.path().join("state"), &server.base_url)
        .args(["session", "new", "profile-1", "--name", "busy test", "--description", "contract test", "--ttl", "30m"])
        .assert()
        .failure()
        .code(1)
        .stdout("")
        .stderr(
            predicate::str::contains("profile_busy: profile already has a live session")
                .and(predicate::str::contains("actor_id: 11111111-1111-4111-8111-111111111117"))
                .and(predicate::str::contains("session_id: session-active"))
                .and(predicate::str::contains("expires_at: 2026-09-04T12:00:00Z")),
        );

    let _backend_url = server.base_url.clone();
    let requests = server.finish();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/api/v1/sessions");
    assert_eq!(request.headers.get("authorization").map(String::as_str), Some("Bearer jwt.binary_contract.signature"));
    assert!(!request.headers.contains_key("x-org-id"));
    assert_eq!(
        request.json(),
        json!({
            "profile_id": "profile-1",
            "incognito": false,
            "name": "busy test",
            "description": "contract test",
            "ttl": "30m"
        })
    );
}

fn connection_response() -> StubResponse {
    StubResponse::json(
        200,
        json!({"data":{"session_id":"session-1","principal_id":"11111111-1111-4111-8111-111111111119","cdp_url":"wss://direct.example/secret-cdp","expires_at":"2099-01-01T00:00:00Z"}}),
    )
}
fn report_response() -> StubResponse {
    StubResponse {
        status: 200,
        content_type: "application/json",
        auth_rejected: false,
        body: b"REPORT_RECEIPT".to_vec(),
    }
}
fn partition_state(home: &Path, backend: &str) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let normalized = silicon_browser::normalize_backend_url(backend).unwrap();
    home.join("backends").join(format!("{:x}", Sha256::digest(normalized.as_bytes()))).join("state.json")
}
#[cfg(unix)]
#[test]
fn run_executes_locally_and_sends_only_completed_telemetry() {
    use std::os::unix::fs::PermissionsExt;
    let server = StubServer::start(vec![connection_response(), report_response(), report_response()]);
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("state");
    let runner = root.path().join("controller");
    fs::write(&runner,"#!/bin/sh\n[ \"$1\" = screenshot ] || exit 91\n[ \"$2\" = './my capture.png' ] || exit 92\n[ \"$AGENT_BROWSER_CDP\" = 'wss://direct.example/secret-cdp' ] || exit 93\n[ -z \"$SB_AUTHTOKEN\" ] || exit 94\nprintf 'local output'\nprintf 'local error' >&2\nexit 23\n").unwrap();
    fs::set_permissions(&runner, fs::Permissions::from_mode(0o700)).unwrap();
    for _ in 0..2 {
        isolated_sb(&home, &server.base_url)
            .env("SB_CONTROLLER_BIN", &runner)
            .args(["--json", "run", "session-1", "screenshot './my capture.png'"])
            .assert()
            .code(23)
            .stdout(predicate::str::contains("local output"))
            .stderr("browser: browser command exited with 23\n");
    }
    let requests = server.finish();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/api/v1/sessions/session-1/connection");
    assert!(requests[0].body.is_empty());
    for request in &requests[1..] {
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/api/v1/sessions/session-1/commands");
        assert_eq!(request.json()["command"], "screenshot './my capture.png'");
        assert_eq!(request.json()["exit_code"], 23);
        assert!(request.json().get("stdout").is_none());
        assert!(request.json().get("stderr").is_none());
        assert!(!String::from_utf8_lossy(&request.body).contains("secret-cdp"));
    }
    assert_ne!(requests[1].json()["command_id"], requests[2].json()["command_id"]);
}

#[test]
fn search_uses_the_account_directly_without_discovery() {
    let server =
        StubServer::start(vec![StubResponse::json(200, json!({"data": {"results": [], "page": 0, "queued_ms": 0}}))]);
    let root = tempfile::tempdir().unwrap();
    isolated_sb(&root.path().join("state"), &server.base_url)
        .args(["search", "query", "--purpose", "contract test"])
        .assert()
        .success();
    let requests = server.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/api/v1/search");
    assert!(!requests[0].headers.contains_key("x-org-id"));
}

#[cfg(unix)]
#[test]
fn logout_removes_stored_credentials_without_network_access() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("state");
    let backend = "http://127.0.0.1:9";
    stored_recovery_home(&home, backend);
    isolated_sb(&home, backend).env_remove("SB_AUTHTOKEN").arg("logout").assert().success().stdout("signed out\n");
    let state: Value = serde_json::from_slice(&fs::read(partition_state(&home, backend)).unwrap()).unwrap();
    for key in ["access_token", "refresh_token", "identity_id", "token_expires_at", "credential_generation", "org_id"] {
        assert!(state.get(key).is_none(), "logout retained {key}");
    }
    isolated_sb(&home, backend)
        .env_remove("SB_AUTHTOKEN")
        .args(["login", "status"])
        .assert()
        .success()
        .stdout("not authenticated\n");
}

#[cfg(unix)]
#[test]
fn setup_exchanges_an_environment_slt_without_letting_it_shadow_the_oat() {
    let auth = json!({
        "data": {
            "access_token": "jwt.from_exchange.signature",
            "refresh_token": "sar_from_exchange",
            "expires_at": "2030-03-17T17:46:40Z",
            "identity": {"id": "11111111-1111-4111-8111-111111111117", "name": "Ada Browser", "kind": "silicon"},
            "services": ["remote-browser", "search-and-fetch"]
        }
    });
    let setup_server = StubServer::start(vec![
        StubResponse::json(200, auth),
        StubResponse::json(
            200,
            json!({"data": {"id": "11111111-1111-4111-8111-111111111117", "name": "Ada Browser", "kind": "silicon"}}),
        ),
        StubResponse::json(200, json!({"data": ["remote-browser", "search-and-fetch"]})),
        StubResponse::json(200, json!({"data": []})),
    ]);
    let state_root = tempfile::tempdir().unwrap();
    let state_home = state_root.path().join("state");
    let runner_directory = state_root.path().join("bin");
    install_fake_runner(&runner_directory);
    let short_lived = "slt_single_use_contract";
    let exchanged_backend = setup_server.base_url.clone();

    isolated_sb(&state_home, &setup_server.base_url)
        .env("SB_AUTHTOKEN", short_lived)
        .env("PATH", &runner_directory)
        .args(["setup"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ready: 11111111-1111-4111-8111-111111111117"));

    let state_bytes = fs::read(partition_state(&state_home, &exchanged_backend)).unwrap();
    let state_text = String::from_utf8(state_bytes).unwrap();
    assert!(!state_text.contains(short_lived));
    assert!(state_text.contains("jwt.from_exchange.signature"));
    assert!(state_text.contains("sar_from_exchange"));
    let saved: Value = serde_json::from_str(&state_text).unwrap();
    assert_eq!(saved["backend_url"], exchanged_backend);
    assert!(saved.get("org_id").is_none());

    // Keeping the one-shot SLT exported must fall back to the exchanged OAT on
    // later commands instead of shadowing it.
    isolated_sb(&state_home, &exchanged_backend)
        .env("SB_AUTHTOKEN", short_lived)
        .args(["profile", "ls"])
        .assert()
        .success();
    let requests = setup_server.finish();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].path, "/api/v1/auth/exchange");
    assert!(!requests[0].headers.contains_key("authorization"));
    assert_eq!(requests[0].json(), json!({"short_lived_token": short_lived, "contract_version":2}));
    for request in &requests[1..] {
        assert_eq!(
            request.headers.get("authorization").map(String::as_str),
            Some("Bearer jwt.from_exchange.signature")
        );
        assert!(!request.headers.contains_key("x-org-id"));
    }
    assert_eq!(
        requests[3].headers.get("authorization").map(String::as_str),
        Some("Bearer jwt.from_exchange.signature")
    );
}

/// Test group: setup validates an invocation-only environment identity without rewriting
/// the issuer or metadata bound to a separately stored rotating credential.
#[cfg(unix)]
#[test]
fn setup_with_environment_jwt_preserves_the_stored_identity_binding() {
    use std::os::unix::fs::PermissionsExt as _;

    let server = StubServer::start(vec![
        StubResponse::json(
            200,
            json!({"data": {
                "id": "11111111-1111-4111-8111-111111111118", "name": "Temporary", "kind": "silicon"
            }}),
        ),
        StubResponse::json(200, json!({"data": ["search-and-fetch"]})),
    ]);
    let state_root = tempfile::tempdir().unwrap();
    let state_home = state_root.path().join("state");
    fs::create_dir(&state_home).unwrap();
    fs::set_permissions(&state_home, fs::Permissions::from_mode(0o700)).unwrap();
    let saved = json!({
        "backend_url": "https://stored.example.test",
        "access_token": "jwt.stored.signature", "refresh_token": "sar_stored",
        "token_expires_at": "2030-03-17T17:46:40Z",
        "org_id": "org-stored", "identity_id": "silicon-stored",
        "services": ["remote-browser"], "last_session_id": "session-stored"
    });
    let state_file = state_home.join("state.json");
    fs::write(&state_file, serde_json::to_vec(&saved).unwrap()).unwrap();
    fs::set_permissions(&state_file, fs::Permissions::from_mode(0o600)).unwrap();
    let runner_directory = state_root.path().join("bin");
    install_fake_runner(&runner_directory);

    isolated_sb(&state_home, &server.base_url)
        .env("SB_AUTHTOKEN", "jwt.temporary.signature")
        .env("PATH", &runner_directory)
        .args(["setup"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ready: 11111111-1111-4111-8111-111111111118"));

    let _backend_url = server.base_url.clone();
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, "/api/v1/me");
    assert_eq!(requests[1].path, "/api/v1/services");
    for request in requests {
        assert_eq!(request.headers.get("authorization").map(String::as_str), Some("Bearer jwt.temporary.signature"));
        assert!(!request.headers.contains_key("x-org-id"));
    }
    let after: Value = serde_json::from_slice(&fs::read(state_file).unwrap()).unwrap();
    assert_eq!(after, saved);
}

#[test]
fn setup_rejects_an_unknown_environment_token_family_before_network_access() {
    let state_root = tempfile::tempdir().unwrap();
    isolated_sb(&state_root.path().join("state"), "http://127.0.0.1:9")
        .env("SB_AUTHTOKEN", "mystery_secret")
        .args(["setup"])
        .assert()
        .failure()
        .stdout("")
        .stderr(
            predicate::str::contains(
                "SB_AUTHTOKEN must use Silicon Accounts slt_ short-lived-token or JWT access-token form",
            )
            .and(predicate::str::contains("could not be reached").not()),
        );
}

#[test]
fn setup_slt_exchange_reports_connection_failure() {
    let state_root = tempfile::tempdir().unwrap();
    isolated_sb(&state_root.path().join("state"), "http://127.0.0.1:9")
        .env("SB_AUTHTOKEN", "slt_single_use")
        .env_remove("SB_ORG_ID")
        .arg("setup")
        .assert()
        .failure()
        .stdout("")
        .stderr(predicate::str::contains("could not be reached"));
}

#[cfg(unix)]
#[test]
fn run_help_falls_back_offline_and_still_succeeds() {
    let state_root = tempfile::tempdir().unwrap();
    let empty_path = state_root.path().join("empty-bin");
    fs::create_dir(&empty_path).unwrap();

    let mut command = cargo_bin_cmd!("browser");
    command
        .env_clear()
        .env("SB_HOME", state_root.path())
        .env("PATH", empty_path)
        .args(["run", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Useful categories:").and(predicate::str::contains("browser run {sessionid}")))
        .stderr(predicate::str::contains("warning:").and(predicate::str::contains("Run `browser setup` first")));
}

#[test]
fn session_logs_emit_shell_replayable_commands_and_keep_json_metadata() {
    let home = tempfile::tempdir().unwrap();
    let command = "fill @e1 'hello; $(false)'";
    let logs = json!([{"sequence":1,"at":"2026-09-05T00:00:00Z","actor_id":"11111111-1111-4111-8111-111111111110","command":command,"exit_code":0}]);
    let server = StubServer::start(vec![
        StubResponse::json(200, json!({"data":logs.clone()})),
        StubResponse::json(200, json!({"data":logs})),
    ]);
    let output = isolated_sb(&home.path().join("state"), &server.base_url)
        .args(["session", "logs", "session-one", "--date", "05-09-2026"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let script =
        format!("browser() {{ printf '%s\\n' \"$#\" \"$1\" \"$2\" \"$3\"; }}\n{}", String::from_utf8(output).unwrap());
    let replay = Command::new("sh").args(["-c", &script]).output().unwrap();
    assert!(replay.status.success());
    assert_eq!(String::from_utf8(replay.stdout).unwrap(), format!("3\nrun\nsession-one\n{command}\n"));
    isolated_sb(&home.path().join("state"), &server.base_url)
        .args(["--json", "session", "logs", "session-one", "--date", "05-09-2026"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"sequence\": 1"));
    assert_eq!(server.finish().len(), 2);
}

#[test]
fn usage_session_filter_is_rejected_instead_of_silently_ignored() {
    let home = tempfile::tempdir().unwrap();
    isolated_sb(&home.path().join("state"), "http://127.0.0.1:1")
        .args(["usage", "show", "session-one", "--filter", "between:01-09-2026=05-09-2026"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unexpected argument '--filter'"));
}

#[cfg(unix)]
#[test]
fn setup_remains_usable_without_recording_consent_and_never_exchanges_a_second_login() {
    let root = tempfile::tempdir().unwrap();
    let runner = root.path().join("bin");
    install_fake_runner(&runner);
    let server = StubServer::start(vec![
        StubResponse::json(
            200,
            json!({"data":{"id":"11111111-1111-4111-8111-111111111110","name":"Actor","kind":"silicon"}}),
        ),
        StubResponse::json(200, json!({"data":[]})),
    ]);
    isolated_sb(&root.path().join("state"), &server.base_url)
        .env("PATH", &runner)
        .env("SB_RECORDING_SLT", "slt_retired_delivery_flow")
        .args(["--json", "setup"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"ready\": true"));
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| request.method == "GET"));
}

#[test]
fn failed_recording_does_not_claim_pending_delivery() {
    let root = tempfile::tempdir().unwrap();
    let server = StubServer::start(vec![StubResponse::json(
        200,
        json!({"data":{
            "session_id":"s1","incognito":true,"session_name":"Failed recording","session_description":"test",
            "owner_id":"11111111-1111-4111-8111-111111111110","briefcase_path":"private/actor/apps/browser/file.mp4",
            "duration_seconds":1,"size_bytes":0,"status":"failed","created_at":"2026-09-05T00:00:00Z",
        "delivery_error":"authorization_required","command_log_link":"https://briefcase.example/files/log-1"
        }}),
    )]);
    isolated_sb(&root.path().join("state"), &server.base_url)
        .args(["recording", "show", "s1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("delivery failed"))
        .stdout(predicate::str::contains("delivery error: authorization_required"))
        .stdout(predicate::str::contains("command log: https://briefcase.example/files/log-1"))
        .stdout(predicate::str::contains("pending OBO storage").not());
    assert_eq!(server.finish().len(), 1);
}

#[cfg(unix)]
fn stored_recovery_home(home: &Path, backend: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir(home).unwrap();
    fs::set_permissions(home, fs::Permissions::from_mode(0o700)).unwrap();
    let state = json!({"backend_url":backend,"access_token":"jwt.rejected.signature","refresh_token":"sar_owned",
        "token_expires_at":"2099-01-01T00:00:00Z","identity_id":"11111111-1111-4111-8111-111111111110"});
    fs::write(home.join("state.json"), serde_json::to_vec(&state).unwrap()).unwrap();
    fs::set_permissions(home.join("state.json"), fs::Permissions::from_mode(0o600)).unwrap();
}
fn rejected_response(marked: bool) -> StubResponse {
    let mut response = StubResponse::json(401, json!({"error":{"code":"unauthenticated","message":"Access rejected"}}));
    response.auth_rejected = marked;
    response
}
fn refreshed_session_response() -> StubResponse {
    StubResponse::json(
        200,
        json!({"data":{"access_token":"jwt.recovered.signature","refresh_token":"sar_rotated",
        "expires_at":"2099-01-01T00:00:00Z","identity":{"id":"11111111-1111-4111-8111-111111111110","name":"Actor","kind":"silicon"},
        "services":[]}}),
    )
}

#[cfg(unix)]
#[test]
fn login_status_refreshes_expired_credentials_and_preserves_service_failures() {
    let server = StubServer::start(vec![
        refreshed_session_response(),
        StubResponse::json(
            200,
            json!({"data":{"id":"11111111-1111-4111-8111-111111111110","name":"Actor","kind":"silicon"}}),
        ),
        StubResponse::json(503, json!({"error":{"code":"unavailable","message":"temporary failure"}})),
    ]);
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("state");
    stored_recovery_home(&home, &server.base_url);
    let mut state: Value = serde_json::from_slice(&fs::read(home.join("state.json")).unwrap()).unwrap();
    state["token_expires_at"] = json!("2000-01-01T00:00:00Z");
    fs::write(home.join("state.json"), state.to_string()).unwrap();
    isolated_sb(&home, &server.base_url)
        .env_remove("SB_AUTHTOKEN")
        .args(["login", "status", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"authenticated\": true"));
    let saved = fs::read(partition_state(&home, &server.base_url)).unwrap();
    isolated_sb(&home, &server.base_url)
        .env_remove("SB_AUTHTOKEN")
        .args(["login", "status", "--json"])
        .assert()
        .failure()
        .stdout("")
        .stderr(predicate::str::contains("unavailable"));
    assert_eq!(fs::read(partition_state(&home, &server.base_url)).unwrap(), saved);
    let requests = server.finish();
    assert_eq!(requests[0].path, "/api/v1/auth/refresh");
    assert_eq!(requests[1].headers["authorization"], "Bearer jwt.recovered.signature");
}

#[cfg(unix)]
#[test]
fn marked_pre_handler_rejection_recovers_owned_auth_once_for_control_plane_request() {
    let server = StubServer::start(vec![
        rejected_response(true),
        refreshed_session_response(),
        StubResponse::json(200, json!({"data":[]})),
    ]);
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("state");
    stored_recovery_home(&home, &server.base_url);
    isolated_sb(&home, &server.base_url).env_remove("SB_AUTHTOKEN").args(["profile", "ls"]).assert().success();
    let _backend_url = server.base_url.clone();
    let requests = server.finish();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].path, "/api/v1/profiles");
    assert_eq!(requests[1].path, "/api/v1/auth/refresh");
    assert_eq!(requests[1].json(), json!({"refresh_token":"sar_owned","contract_version":2}));
    assert_eq!(requests[2].path, requests[0].path);
    assert_eq!(requests[2].body, requests[0].body);
    assert_eq!(requests[2].headers.get("authorization").unwrap(), "Bearer jwt.recovered.signature");
    let saved: Value = serde_json::from_slice(&fs::read(partition_state(&home, &_backend_url)).unwrap()).unwrap();
    assert_eq!(saved["refresh_token"], "sar_rotated");
    assert_eq!(saved["identity_id"], "11111111-1111-4111-8111-111111111110");
}

#[cfg(unix)]
#[test]
fn ordinary_401_or_environment_jwt_never_consumes_stored_refresh_tokens() {
    for (marked, environment) in [(false, false), (true, true)] {
        let server = StubServer::start(vec![rejected_response(marked)]);
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("state");
        stored_recovery_home(&home, &server.base_url);
        let before = fs::read(home.join("state.json")).unwrap();
        let mut cmd = isolated_sb(&home, &server.base_url);
        if !environment {
            cmd.env_remove("SB_AUTHTOKEN");
        }
        cmd.args(["profile", "ls"]).assert().failure();
        let backend = server.base_url.clone();
        assert_eq!(server.finish().len(), 1);
        let saved: Value = serde_json::from_slice(&fs::read(partition_state(&home, &backend)).unwrap()).unwrap();
        let old: Value = serde_json::from_slice(&before).unwrap();
        assert_eq!(saved["access_token"], old["access_token"]);
        assert_eq!(saved["refresh_token"], old["refresh_token"]);
    }
}

#[cfg(unix)]
#[test]
fn repeated_marked_rejection_stops_after_one_recovery() {
    let server =
        StubServer::start(vec![rejected_response(true), refreshed_session_response(), rejected_response(true)]);
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("state");
    stored_recovery_home(&home, &server.base_url);
    isolated_sb(&home, &server.base_url).env_remove("SB_AUTHTOKEN").args(["profile", "ls"]).assert().failure();
    assert_eq!(server.finish().len(), 3);
}

#[cfg(unix)]
#[test]
fn one_cli_invocation_reuses_its_own_recovery_for_later_metadata_requests() {
    let server = StubServer::start(vec![
        rejected_response(true),
        refreshed_session_response(),
        StubResponse::json(
            200,
            json!({"data":{"id":"11111111-1111-4111-8111-111111111110","name":"Actor","kind":"silicon"}}),
        ),
        StubResponse::json(200, json!({"data":["remote-browser"]})),
    ]);
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("state");
    stored_recovery_home(&home, &server.base_url);
    isolated_sb(&home, &server.base_url).env_remove("SB_AUTHTOKEN").assert().success();
    let requests = server.finish();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[3].path, "/api/v1/services");
    assert_eq!(requests[3].headers.get("authorization").unwrap(), "Bearer jwt.recovered.signature");
}

#[cfg(unix)]
#[test]
fn failed_telemetry_is_retried_without_repeating_the_browser_action() {
    use std::os::unix::fs::PermissionsExt;
    let server = StubServer::start(vec![connection_response(), rejected_response(false), report_response()]);
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("state");
    let runner = root.path().join("controller");
    let calls = root.path().join("calls");
    fs::write(&runner, "#!/bin/sh\nprintf 'action\\n' >> \"$TEST_ACTION_COUNT\"\nprintf 'completed locally'\n")
        .unwrap();
    fs::set_permissions(&runner, fs::Permissions::from_mode(0o700)).unwrap();
    isolated_sb(&home, &server.base_url)
        .env("SB_CONTROLLER_BIN", &runner)
        .env("TEST_ACTION_COUNT", &calls)
        .args(["run", "session-1", "snapshot"])
        .assert()
        .success()
        .stdout("completed locally")
        .stderr(predicate::str::contains("command logs are queued locally"));
    isolated_sb(&home, &server.base_url)
        .args(["session", "sync", "session-1"])
        .assert()
        .success()
        .stdout("delivered 1 command logs\n");
    let requests = server.finish();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[1].body, requests[2].body);
    assert_eq!(fs::read_to_string(calls).unwrap(), "action\n");
}

#[cfg(unix)]
#[test]
fn backend_override_never_reuses_another_issuers_stored_credentials() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("state");
    stored_recovery_home(&home, "https://original.example");
    isolated_sb(&home, "http://127.0.0.1:9")
        .env_remove("SB_AUTHTOKEN")
        .args(["profile", "ls"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not signed in").and(predicate::str::contains("could not be reached").not()));
    let original: Value = serde_json::from_slice(&fs::read(home.join("state.json")).unwrap()).unwrap();
    let other: Value =
        serde_json::from_slice(&fs::read(partition_state(&home, "http://127.0.0.1:9")).unwrap()).unwrap();
    assert_eq!(original["access_token"], "jwt.rejected.signature");
    assert!(other.get("access_token").is_none());
}

#[test]
fn apps_discovery_is_available_offline_without_a_session() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("state");
    for args in [vec!["accounts", "--json"], vec!["login", "status", "--json"]] {
        let output = isolated_sb(&home, "http://127.0.0.1:1")
            .env_remove("SB_AUTHTOKEN")
            .args(args.clone())
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let result: Value = serde_json::from_slice(&output).unwrap();
        if args[0] == "accounts" {
            assert_eq!(result["app_id"], "browser");
        } else {
            assert_eq!(result["authenticated"], false);
        }
    }
}

#[cfg(unix)]
#[test]
fn a_refresh_only_login_saves_the_rotated_pair_before_continuing() {
    let server = StubServer::start(vec![
        StubResponse::json(
            200,
            json!({"data":{
                "access_token":"jwt.recovered.signature","refresh_token":"sar_rotated",
                "expires_at":"2099-01-01T00:00:00Z",
                "identity":{"id":"11111111-1111-4111-8111-111111111119","name":"si:actor","kind":"silicon"},
                "services":[]
            }}),
        ),
        StubResponse::json(
            200,
            json!({"data":{"id":"11111111-1111-4111-8111-111111111119","name":"si:actor","kind":"silicon"}}),
        ),
    ]);
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("state");
    stored_recovery_home(&home, &server.base_url);
    let mut state: Value = serde_json::from_slice(&fs::read(home.join("state.json")).unwrap()).unwrap();
    state.as_object_mut().unwrap().remove("access_token");
    state["identity_id"] = json!("11111111-1111-4111-8111-111111111119");
    fs::write(home.join("state.json"), state.to_string()).unwrap();
    isolated_sb(&home, &server.base_url)
        .env_remove("SB_AUTHTOKEN")
        .args(["login", "status", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"authenticated\": true"));
    let saved: Value = serde_json::from_slice(&fs::read(partition_state(&home, &server.base_url)).unwrap()).unwrap();
    assert_eq!(saved["access_token"], "jwt.recovered.signature");
    assert_eq!(saved["refresh_token"], "sar_rotated");
    assert_eq!(saved["identity_id"], "11111111-1111-4111-8111-111111111119");
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, "/api/v1/auth/refresh");
    assert_eq!(requests[1].headers["authorization"], "Bearer jwt.recovered.signature");
}

#[cfg(unix)]
#[test]
fn failed_refresh_is_never_replayed_by_a_later_invocation() {
    let server = StubServer::start(vec![StubResponse::json(
        503,
        json!({"error":{"code":"accounts_unavailable","message":"Token outcome unknown; sign in again"}}),
    )]);
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("state");
    stored_recovery_home(&home, &server.base_url);
    let mut state: Value = serde_json::from_slice(&fs::read(home.join("state.json")).unwrap()).unwrap();
    state["token_expires_at"] = json!("2000-01-01T00:00:00Z");
    fs::write(home.join("state.json"), state.to_string()).unwrap();
    for _ in 0..2 {
        isolated_sb(&home, &server.base_url)
            .env_remove("SB_AUTHTOKEN")
            .args(["login", "status", "--json"])
            .assert()
            .failure();
    }
    let backend = server.base_url.clone();
    let requests = server.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/api/v1/auth/refresh");
    let saved: Value = serde_json::from_slice(&fs::read(partition_state(&home, &backend)).unwrap()).unwrap();
    assert!(saved.get("refresh_token").is_none());
}

#[cfg(unix)]
#[test]
fn a_concurrent_login_during_refresh_never_rebinds_the_pending_command() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let backend = format!("http://{}", listener.local_addr().unwrap());
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("state");
    stored_recovery_home(&home, &backend);
    let mut original: Value = serde_json::from_slice(&fs::read(home.join("state.json")).unwrap()).unwrap();
    original["token_expires_at"] = json!("2000-01-01T00:00:00Z");
    original["credential_generation"] = json!("login-a");
    fs::write(home.join("state.json"), original.to_string()).unwrap();
    let worker_home = home.clone();
    let worker_backend = backend.clone();
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        assert_eq!(request.path, "/api/v1/auth/refresh");
        let path = partition_state(&worker_home, &worker_backend);
        let mut newer: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        newer["access_token"] = json!("jwt.accountB.signature");
        newer["refresh_token"] = json!("sar_accountB");
        newer["identity_id"] = json!("22222222-2222-4222-8222-222222222222");
        newer["credential_generation"] = json!("login-b");
        newer["token_expires_at"] = json!("2099-01-01T00:00:00Z");
        fs::write(&path, newer.to_string()).unwrap();
        write_response(&mut stream, refreshed_session_response());
        listener
    });
    isolated_sb(&home, &backend)
        .env_remove("SB_AUTHTOKEN")
        .args(["profile", "ls"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("sign-in changed while refreshing"));
    let listener = worker.join().unwrap();
    listener.set_nonblocking(true).unwrap();
    assert!(matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock));
    let newer: Value = serde_json::from_slice(&fs::read(partition_state(&home, &backend)).unwrap()).unwrap();
    assert_eq!(newer["identity_id"], "22222222-2222-4222-8222-222222222222");
    assert_eq!(newer["refresh_token"], "sar_accountB");
}
