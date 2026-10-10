use std::{
    net::TcpListener,
    path::Path,
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};
fn launch(directory: &Path, database: &Path, upstream: &TcpListener, args: &[&str]) -> Output {
    let origin = format!("http://{}", upstream.local_addr().unwrap());
    let mut child = Command::new(env!("CARGO_BIN_EXE_silicon-browser-backend"))
        .current_dir(directory)
        .env_clear()
        .env("SB_DATABASE_URL", format!("sqlite://{}?mode=rwc", database.display()))
        .env("SB_ORIGIN", &origin)
        .env("SB_ENCRYPTION_KEY", "17".repeat(32))
        .env("ACCOUNTS_URL", &origin)
        .env("ACCOUNTS_APP_ID", "browser")
        .env("ACCOUNTS_APP_SECRET", "sa_app_test_secret")
        .env("BROWSER_USE_API_KEY", "offline-provider-secret")
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
            panic!("backend must reject legacy state before network or workers");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(matches!(upstream.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock));
    output
}
#[tokio::test]
async fn retired_migration_commands_never_create_or_mutate_databases() {
    let directory = tempfile::tempdir().unwrap();
    let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
    upstream.set_nonblocking(true).unwrap();
    let missing = directory.path().join("missing.db");
    for args in [
        &["--migrate-public-identifiers", "mapping.json"][..],
        &["--verify-public-identifiers-only"][..],
        &["--unknown"][..],
    ] {
        let output = launch(directory.path(), &missing, &upstream, args);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("usage:"));
        assert!(!missing.exists());
    }
}
#[tokio::test]
async fn legacy_database_startup_rejects_ownership_without_schema_changes_or_network() {
    let directory = tempfile::tempdir().unwrap();
    let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
    upstream.set_nonblocking(true).unwrap();
    let database = directory.path().join("legacy.db");
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=rwc", database.display())).await.unwrap();
    sqlx::query("CREATE TABLE identity_projection(principal_id TEXT)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO identity_projection VALUES ('old-owner')").execute(&pool).await.unwrap();
    pool.close().await;
    let original = std::fs::read(&database).unwrap();
    let output = launch(directory.path(), &database, &upstream, &[]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("IAM-owned"));
    assert_eq!(std::fs::read(&database).unwrap(), original);
}
