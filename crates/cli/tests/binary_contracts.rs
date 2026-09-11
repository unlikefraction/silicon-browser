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
    let mut command = cargo_bin_cmd!("sb");
    command
        .env_clear()
        .env("SB_HOME", home)
        .env("SB_BACKEND_URL", backend)
        .env("SB_AUTHTOKEN", "oat_binary_contract")
        .env("SB_ORG_ID", "org-contract");
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
        command.arg("setup").assert().failure().stderr(predicate::str::contains("SB_AUTHTOKEN must use IAM"));
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
                    "id": "silicon-7",
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
        .stdout("Ada Browser (Silicon)\norg: org-contract\nservices: remote-browser, search-and-fetch\n")
        .stderr("");

    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!((&requests[0].method, &requests[0].path), (&"GET".into(), &"/api/v1/me".into()));
    assert_eq!((&requests[1].method, &requests[1].path), (&"GET".into(), &"/api/v1/services".into()));
    for request in requests {
        assert_eq!(request.headers.get("authorization").map(String::as_str), Some("Bearer oat_binary_contract"));
        assert_eq!(request.headers.get("x-org-id").map(String::as_str), Some("org-contract"));
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
                    "actor_id": "silicon-7",
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
                .and(predicate::str::contains("actor_id: silicon-7"))
                .and(predicate::str::contains("session_id: session-active"))
                .and(predicate::str::contains("expires_at: 2026-09-04T12:00:00Z")),
        );

    let _backend_url = server.base_url.clone();
    let requests = server.finish();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/api/v1/sessions");
    assert_eq!(request.headers.get("authorization").map(String::as_str), Some("Bearer oat_binary_contract"));
    assert_eq!(request.headers.get("x-org-id").map(String::as_str), Some("org-contract"));
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
        json!({"data":{"session_id":"session-1","principal_id":"immutable-actor","cdp_url":"wss://direct.example/secret-cdp","expires_at":"2099-01-01T00:00:00Z"}}),
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
            .stderr("sb: browser command exited with 23\n");
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
fn search_with_one_bound_org_needs_no_setup_or_org_flag() {
    let server = StubServer::start(vec![
        StubResponse::json(200, json!({"data": [{"id": "org-only", "name": "Only Org"}]})),
        StubResponse::json(200, json!({"data": {"results": [], "page": 0, "queued_ms": 0}})),
    ]);
    let state_root = tempfile::tempdir().unwrap();
    let state_home = state_root.path().join("state");

    isolated_sb(&state_home, &server.base_url)
        .env_remove("SB_ORG_ID")
        .args(["search", "query", "--purpose", "contract test"])
        .assert()
        .success()
        .stdout("")
        .stderr("");

    let _backend_url = server.base_url.clone();
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, "/api/v1/orgs");
    assert_eq!(requests[0].headers.get("authorization").map(String::as_str), Some("Bearer oat_binary_contract"));
    assert!(!requests[0].headers.contains_key("x-org-id"));
    assert_eq!(requests[1].path, "/api/v1/search");
    assert_eq!(requests[1].headers.get("x-org-id").map(String::as_str), Some("org-only"));

    let persisted: Value =
        serde_json::from_slice(&fs::read(partition_state(&state_home, &_backend_url)).unwrap()).unwrap();
    assert!(persisted.get("org_id").is_none(), "an environment identity must not change the saved org");
}

#[cfg(unix)]
#[test]
fn a_sole_org_derived_from_stored_auth_is_persisted_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;

    let server = StubServer::start(vec![
        StubResponse::json(200, json!({"data": [{"id": "org-only", "name": "Only Org"}]})),
        StubResponse::json(200, json!({"data": {"results": [], "page": 0, "queued_ms": 0}})),
    ]);
    let state_root = tempfile::tempdir().unwrap();
    let state_home = state_root.path().join("state");
    fs::create_dir(&state_home).unwrap();
    fs::set_permissions(&state_home, fs::Permissions::from_mode(0o700)).unwrap();
    let state_file = state_home.join("state.json");
    fs::write(
        &state_file,
        serde_json::to_vec(&json!({
            "backend_url": server.base_url,
            "access_token": "oat_stored_contract",
            "refresh_token": "ort_stored_contract",
            "token_expires_at": "2030-03-17T17:46:40Z"
        }))
        .unwrap(),
    )
    .unwrap();
    fs::set_permissions(&state_file, fs::Permissions::from_mode(0o600)).unwrap();

    isolated_sb(&state_home, &server.base_url)
        .env_remove("SB_AUTHTOKEN")
        .env_remove("SB_ORG_ID")
        .args(["search", "query", "--purpose", "contract test"])
        .assert()
        .success();

    let _backend_url = server.base_url.clone();
    let requests = server.finish();
    assert_eq!(requests[0].path, "/api/v1/orgs");
    assert_eq!(requests[1].headers.get("x-org-id").map(String::as_str), Some("org-only"));
    let state_file = partition_state(&state_home, &_backend_url);
    let persisted: Value = serde_json::from_slice(&fs::read(&state_file).unwrap()).unwrap();
    assert_eq!(persisted["org_id"], "org-only");
    assert_eq!(fs::metadata(state_file).unwrap().permissions().mode() & 0o777, 0o600);
}

#[cfg(unix)]
#[test]
fn setup_exchanges_an_environment_slt_without_letting_it_shadow_the_oat() {
    let auth = json!({
        "data": {
            "access_token": "oat_from_exchange",
            "refresh_token": "ort_from_exchange",
            "expires_at": "2030-03-17T17:46:40Z",
            "identity": {"id": "silicon-7", "name": "Ada Browser", "kind": "silicon"},
            "org": {"id": "org-contract", "name": "Contract Org"},
            "services": ["remote-browser", "search-and-fetch"]
        }
    });
    let setup_server = StubServer::start(vec![
        StubResponse::json(200, auth),
        StubResponse::json(200, json!({"data": {"id": "silicon-7", "name": "Ada Browser", "kind": "silicon"}})),
        StubResponse::json(200, json!({"data": ["remote-browser", "search-and-fetch"]})),
        StubResponse::json(200, json!({"data": []})),
    ]);
    let state_root = tempfile::tempdir().unwrap();
    let state_home = state_root.path().join("state");
    let runner_directory = state_root.path().join("bin");
    install_fake_runner(&runner_directory);
    let short_lived = "oac_single_use_contract";
    let exchanged_backend = setup_server.base_url.clone();

    isolated_sb(&state_home, &setup_server.base_url)
        .env("SB_AUTHTOKEN", short_lived)
        .env("PATH", &runner_directory)
        .args(["setup", "--org-id", "org-contract"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ready: @silicon-7 in org-contract"));

    let state_bytes = fs::read(partition_state(&state_home, &exchanged_backend)).unwrap();
    let state_text = String::from_utf8(state_bytes).unwrap();
    assert!(!state_text.contains(short_lived));
    assert!(state_text.contains("oat_from_exchange"));
    assert!(state_text.contains("ort_from_exchange"));
    let saved: Value = serde_json::from_str(&state_text).unwrap();
    assert_eq!(saved["backend_url"], exchanged_backend);
    assert_eq!(saved["org_id"], "org-contract");

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
    assert_eq!(requests[0].json(), json!({"short_lived_token": short_lived, "org_id": "org-contract"}));
    for request in &requests[1..] {
        assert_eq!(request.headers.get("authorization").map(String::as_str), Some("Bearer oat_from_exchange"));
        assert_eq!(request.headers.get("x-org-id").map(String::as_str), Some("org-contract"));
    }
    assert_eq!(requests[3].headers.get("authorization").map(String::as_str), Some("Bearer oat_from_exchange"));
}

/// Test group: setup validates an invocation-only environment identity without rewriting
/// the issuer, org, or metadata bound to a separately stored rotating credential.
#[cfg(unix)]
#[test]
fn setup_with_environment_oat_preserves_the_stored_identity_binding() {
    use std::os::unix::fs::PermissionsExt as _;

    let server = StubServer::start(vec![
        StubResponse::json(
            200,
            json!({"data": {
                "id": "silicon-temporary", "name": "Temporary", "kind": "silicon"
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
        "access_token": "oat_stored", "refresh_token": "ort_stored",
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
        .env("SB_AUTHTOKEN", "oat_temporary")
        .env("PATH", &runner_directory)
        .args(["setup", "--org", "org-temporary"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ready: @silicon-temporary in org-temporary"));

    let _backend_url = server.base_url.clone();
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, "/api/v1/me");
    assert_eq!(requests[1].path, "/api/v1/services");
    for request in requests {
        assert_eq!(request.headers.get("authorization").map(String::as_str), Some("Bearer oat_temporary"));
        assert_eq!(request.headers.get("x-org-id").map(String::as_str), Some("org-temporary"));
    }
    let after: Value = serde_json::from_slice(&fs::read(state_file).unwrap()).unwrap();
    assert_eq!(after, saved);
}

#[test]
fn setup_rejects_an_unknown_environment_token_family_before_network_access() {
    let state_root = tempfile::tempdir().unwrap();
    isolated_sb(&state_root.path().join("state"), "http://127.0.0.1:9")
        .env("SB_AUTHTOKEN", "mystery_secret")
        .args(["setup", "--org", "org-contract"])
        .assert()
        .failure()
        .stdout("")
        .stderr(
            predicate::str::contains("SB_AUTHTOKEN must use IAM's oac_ short-lived-token or oat_ access-token form")
                .and(predicate::str::contains("could not be reached").not()),
        );
}

#[test]
fn setup_exchanges_an_environment_slt_without_an_org() {
    let state_root = tempfile::tempdir().unwrap();
    isolated_sb(&state_root.path().join("state"), "http://127.0.0.1:9")
        .env("SB_AUTHTOKEN", "oac_single_use")
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

    let mut command = cargo_bin_cmd!("sb");
    command
        .env_clear()
        .env("SB_HOME", state_root.path())
        .env("PATH", empty_path)
        .args(["run", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Useful categories:").and(predicate::str::contains("sb run {sessionid}")))
        .stderr(predicate::str::contains("warning:").and(predicate::str::contains("Run `sb setup` first")));
}

#[test]
fn session_logs_emit_shell_replayable_commands_and_keep_json_metadata() {
    let home = tempfile::tempdir().unwrap();
    let command = "fill @e1 'hello; $(false)'";
    let logs = json!([{"sequence":1,"at":"2026-09-05T00:00:00Z","actor_id":"actor","command":command,"exit_code":0}]);
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
        format!("sb() {{ printf '%s\\n' \"$#\" \"$1\" \"$2\" \"$3\"; }}\n{}", String::from_utf8(output).unwrap());
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
        .stderr(predicate::str::contains("cannot be used"));
}

#[cfg(unix)]
#[test]
fn setup_enrolls_a_separate_delivery_token_and_reuses_active_authorization() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("state");
    let runner = root.path().join("bin");
    install_fake_runner(&runner);
    let identity = json!({"data":{"id":"actor","name":"Actor","kind":"silicon"}});
    let services = json!({"data":["recording_delivery"]});
    let status = |state: &str, enabled: bool| json!({"data":{"configured":true,"enabled":enabled,"state":state,"actor_id":"actor"}});
    let server = StubServer::start(vec![
        StubResponse::json(200, identity.clone()),
        StubResponse::json(200, services.clone()),
        StubResponse::json(200, status("needs_auth", false)),
        StubResponse::json(200, status("active", true)),
        StubResponse::json(200, identity),
        StubResponse::json(200, services),
        StubResponse::json(200, status("active", true)),
    ]);
    let separate = "oac_delivery_single_use";
    isolated_sb(&home, &server.base_url)
        .env("PATH", &runner)
        .env("SB_RECORDING_SLT", separate)
        .args(["--json", "setup"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"state\": \"active\""))
        .stdout(predicate::str::contains(separate).not());
    // A stale exported token must not be consumed again when authorization is already active.
    isolated_sb(&home, &server.base_url)
        .env("PATH", &runner)
        .env("SB_RECORDING_SLT", separate)
        .args(["setup"])
        .assert()
        .success();
    let requests = server.finish();
    assert_eq!(requests.len(), 7);
    assert_eq!(requests[3].method, "POST");
    assert_eq!(requests[3].path, "/api/v1/auth/delivery");
    assert_eq!(requests[3].json(), json!({"short_lived_token":separate}));
    assert_eq!(requests[3].headers.get("authorization").unwrap(), "Bearer oat_binary_contract");
    assert_eq!(requests[3].headers.get("x-org-id").unwrap(), "org-contract");
    if let Ok(state) = fs::read_to_string(home.join("state.json")) {
        assert!(!state.contains(separate));
    }
}

#[test]
fn setup_reports_missing_delivery_authorization_without_claiming_ready() {
    let root = tempfile::tempdir().unwrap();
    let server = StubServer::start(vec![
        StubResponse::json(200, json!({"data":{"id":"actor","name":"Actor","kind":"silicon"}})),
        StubResponse::json(200, json!({"data":["recording_delivery"]})),
        StubResponse::json(
            200,
            json!({"data":{"configured":true,"enabled":false,"state":"needs_auth","actor_id":"actor"}}),
        ),
    ]);
    isolated_sb(&root.path().join("state"), &server.base_url)
        .args(["setup"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("SB_RECORDING_SLT"))
        .stdout(predicate::str::contains("ready:").not());
    assert_eq!(server.finish().len(), 3);
}

#[test]
fn failed_recording_does_not_claim_pending_delivery() {
    let root = tempfile::tempdir().unwrap();
    let server = StubServer::start(vec![StubResponse::json(
        200,
        json!({"data":{
            "session_id":"s1","incognito":true,"session_name":"Failed recording","session_description":"test",
            "owner_id":"actor","briefcase_path":"private/actor/apps/tos>browser/file.mp4",
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

#[test]
fn recording_send_queues_delivery_and_surfaces_non_retryable_errors() {
    let root = tempfile::tempdir().unwrap();
    let server = StubServer::start(vec![
        StubResponse::json(
            200,
            json!({"data":{
                "session_id":"s1","incognito":true,"session_name":"Retry recording","session_description":"test",
                "owner_id":"actor","briefcase_path":"private/actor/file.mp4","duration_seconds":30,
                "size_bytes":0,"status":"pending","created_at":"2026-09-05T00:00:00Z",
                "command_log_link":"https://briefcase.example/existing-log"
            }}),
        ),
        StubResponse::json(
            409,
            json!({"error":{"code":"recording_not_retryable","message":"This failure cannot be retried"}}),
        ),
    ]);
    let home = root.path().join("state");
    isolated_sb(&home, &server.base_url)
        .args(["recording", "send", "s1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("status: Pending"))
        .stdout(predicate::str::contains("https://briefcase.example/existing-log"));
    isolated_sb(&home, &server.base_url)
        .args(["recording", "send", "s2"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("recording_not_retryable"));
    let _backend_url = server.base_url.clone();
    let requests = server.finish();
    assert_eq!(requests.len(), 2);
    for (request, id) in requests.iter().zip(["s1", "s2"]) {
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, format!("/api/v1/recordings/{id}/retry"));
        assert_eq!(request.headers.get("authorization").unwrap(), "Bearer oat_binary_contract");
        assert_eq!(request.headers.get("x-org-id").unwrap(), "org-contract");
    }
}

#[cfg(unix)]
fn stored_recovery_home(home: &Path, backend: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir(home).unwrap();
    fs::set_permissions(home, fs::Permissions::from_mode(0o700)).unwrap();
    let state = json!({"backend_url":backend,"access_token":"oat_rejected","refresh_token":"ort_owned",
        "token_expires_at":"2099-01-01T00:00:00Z","org_id":"org-contract","identity_id":"actor"});
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
        json!({"data":{"access_token":"oat_recovered","refresh_token":"ort_rotated",
        "expires_at":"2099-01-01T00:00:00Z","identity":{"id":"actor","name":"Actor","kind":"silicon"},
        "org":{"id":"org-contract","name":"Org"},"services":[]}}),
    )
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
    assert_eq!(requests[1].json(), json!({"refresh_token":"ort_owned","org_id":"org-contract"}));
    assert_eq!(requests[2].path, requests[0].path);
    assert_eq!(requests[2].body, requests[0].body);
    assert_eq!(requests[2].headers.get("authorization").unwrap(), "Bearer oat_recovered");
    let saved: Value = serde_json::from_slice(&fs::read(partition_state(&home, &_backend_url)).unwrap()).unwrap();
    assert_eq!(saved["refresh_token"], "ort_rotated");
    assert_eq!(saved["identity_id"], "actor");
}

#[cfg(unix)]
#[test]
fn ordinary_401_or_environment_oat_never_consumes_stored_refresh_tokens() {
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
        StubResponse::json(200, json!({"data":{"id":"actor","name":"Actor","kind":"silicon"}})),
        StubResponse::json(200, json!({"data":["remote-browser"]})),
    ]);
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("state");
    stored_recovery_home(&home, &server.base_url);
    isolated_sb(&home, &server.base_url).env_remove("SB_AUTHTOKEN").assert().success();
    let requests = server.finish();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[3].path, "/api/v1/services");
    assert_eq!(requests[3].headers.get("authorization").unwrap(), "Bearer oat_recovered");
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
    assert_eq!(original["access_token"], "oat_rejected");
    assert!(other.get("access_token").is_none());
}
