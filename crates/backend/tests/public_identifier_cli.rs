use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn migrate(directory: &Path, database: &Path, upstream: &TcpListener, args: &[&str]) -> Output {
    let origin = format!("http://{}", upstream.local_addr().unwrap());
    let mut child = Command::new(env!("CARGO_BIN_EXE_silicon-browser-backend"))
        .current_dir(directory)
        .env_clear()
        .env("SB_DATABASE_URL", format!("sqlite://{}?mode=rwc", database.display()))
        .env("SB_ORIGIN", &origin)
        .env("SB_ENCRYPTION_KEY", "17".repeat(32))
        .env("SILICON_IAM_URL", &origin)
        .env("IAM_APP_ID", "browser")
        .env("IAM_APP_SECRET", "offline-test-secret")
        .env("BROWSER_USE_API_KEY", "offline-provider-secret")
        .env("BRIEFCASE_URL", &origin)
        .env("BRIEFCASE_APP_ID", "briefcase")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("offline migration did not exit; it must not start network requests or workers");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(matches!(upstream.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock));
    output
}

#[tokio::test]
async fn migration_cli_is_offline_world_scoped_and_refuses_accidental_new_databases() {
    let directory = tempfile::tempdir().unwrap();
    let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
    upstream.set_nonblocking(true).unwrap();
    std::fs::write(directory.path().join("mapping.json"), "[]").unwrap();
    let args = ["--migrate-public-identifiers", "mapping.json"];
    let missing = directory.path().join("missing.db");
    for arguments in [args.as_slice(), &["--unknown", "mapping.json"][..]] {
        let output = migrate(directory.path(), &missing, &upstream, arguments);
        assert!(!output.status.success());
        let message = String::from_utf8_lossy(&output.stdout);
        assert!(message.contains(if arguments == args { "existing SQLite database" } else { "usage:" }), "{message}");
        assert!(!missing.exists());
    }
    let world = "00000000-0000-0000-0000-000000000001";
    let testing = directory.path().join(format!("{world}-{}.db", "a".repeat(64)));
    std::fs::write(&testing, []).unwrap();
    for arguments in [
        args.as_slice(),
        &["--migrate-public-identifiers", "mapping.json", "--scope-key", "00000000-0000-0000-0000-000000000002"][..],
        &["--migrate-public-identifiers", "mapping.json", "--unknown", world][..],
    ] {
        let output = migrate(directory.path(), &testing, &upstream, arguments);
        assert!(!output.status.success());
        assert_eq!(std::fs::metadata(&testing).unwrap().len(), 0);
    }
    let production = directory.path().join("production.db");
    std::fs::write(&production, []).unwrap();
    for (database, scope, arguments) in [
        (&production, "", args.as_slice()),
        (&testing, world, &["--migrate-public-identifiers", "mapping.json", "--scope-key", world][..]),
    ] {
        let output = migrate(directory.path(), database, &upstream, arguments);
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=ro", database.display())).await.unwrap();
        let persisted: (String, String) =
            sqlx::query_as("SELECT scope_key, mapping_json FROM public_identifier_schema")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(persisted, (scope.into(), "[]".into()));
        pool.close().await;
    }
}
