use std::sync::Arc;
use std::time::Duration;

use silicon_browser_backend::auth::SiliconIamIdentityProvider;
use silicon_browser_backend::config::Config;
use silicon_browser_backend::crypto::SecretBox;
use silicon_browser_backend::providers::{BriefcaseClient, BrowserUseV3, FairSearchPool};
use silicon_browser_backend::store::{PublicIdentifierMapping, Store};
use silicon_browser_backend::{AppState, TestingRegistry, production_router_with_testing, spawn_ttl_reaper};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "silicon_browser_backend=info,tower_http=info".into()),
        )
        .with_target(false)
        .compact()
        .init();

    if let Err(error) = run().await {
        tracing::error!(%error, "server stopped");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() && !(args.len() == 2 || args.len() == 4)
        || args.first().is_some_and(|arg| arg != "--migrate-public-identifiers")
        || args.len() == 4 && args[2] != "--scope-key"
    {
        return Err("usage: silicon-browser-backend [--migrate-public-identifiers mapping.json [--scope-key <IAM-testing-UUID>]]".into());
    }
    let config = Config::from_env()?;
    if !args.is_empty() {
        use std::str::FromStr;
        let options =
            sqlx::sqlite::SqliteConnectOptions::from_str(&config.database_url).map_err(|error| error.to_string())?;
        let filename = options.get_filename();
        if !filename.is_file() {
            return Err("migration requires an existing SQLite database; check SB_DATABASE_URL".into());
        }
        let scope_key = args.get(3).map(String::as_str).unwrap_or("");
        if let Some(stem) = filename.file_stem().and_then(|name| name.to_str())
            && stem.len() == 101
            && stem.as_bytes()[36] == b'-'
            && uuid::Uuid::parse_str(&stem[..36]).is_ok()
            && &stem[..36] != scope_key
        {
            return Err(
                "test database filename does not match --scope-key; never use a production mapping for testing".into(),
            );
        }
        if config.iam_test_environment_key.is_some() && scope_key.is_empty() {
            return Err("testing configuration requires an explicit --scope-key for offline migration".into());
        }
        let mapping: Vec<PublicIdentifierMapping> = serde_json::from_slice(
            &std::fs::read(&args[1]).map_err(|error| format!("could not read IAM mapping: {error}"))?,
        )
        .map_err(|error| format!("invalid IAM mapping: {error}"))?;
        let store = Store::connect(&config.database_url).await.map_err(|error| error.to_string())?;
        store
            .migrate_public_identifiers(scope_key, &mapping, &SecretBox::new(&config.encryption_key))
            .await
            .map_err(|error| error.to_string())?;
        tracing::info!(
            identities = mapping.len(),
            "public identifier migration completed; no listeners or workers started"
        );
        return Ok(());
    }
    let store = Store::connect(&config.database_url).await.map_err(|error| error.to_string())?;
    let cache = Arc::new(silicon_browser_backend::auth_cache::AuthorizationCache::default());
    let webhook = config
        .iam_webhook_secret
        .as_deref()
        .map(|secret| {
            silicon_browser_backend::webhook::router(
                store.clone(),
                cache.clone(),
                secret,
                config.iam_webhook_key_version,
                config.iam_test_environment_key.as_deref(),
            )
        })
        .transpose()?;
    let identity = SiliconIamIdentityProvider::connect_with_environment(
        &config.iam_url,
        config.iam_app_id.clone(),
        config.iam_app_secret.clone(),
        config.iam_test_environment_key.as_deref(),
    )
    .await
    .map_err(|error| error.to_string())?
    .with_authorization_cache(cache);
    let scope_key = identity.testing_environment_id().map(|id| id.to_string()).unwrap_or_default();
    store.ensure_public_identifiers_migrated(&scope_key).await.map_err(|error| error.to_string())?;
    let browser = BrowserUseV3::new(&config.browser_use_api_key).map_err(|error| error.to_string())?;
    let search = if config.tinyfish_api_keys.is_empty() {
        tracing::warn!("TinyFish is not configured; search and fetch endpoints will return 503");
        None
    } else {
        Some(Arc::new(
            FairSearchPool::from_tinyfish_api_keys(&config.tinyfish_api_keys).map_err(|error| error.to_string())?,
        ))
    };
    let mut state = AppState::new(
        &config.origin,
        store,
        SecretBox::new(&config.encryption_key),
        Arc::new(identity),
        Arc::new(browser),
        search,
    )?
    .require_recording_delivery();
    if let (Some(url), Some(audience)) = (&config.briefcase_url, &config.briefcase_app_id) {
        let client = BriefcaseClient::with_upload_limit(
            url,
            config.briefcase_test_environment_key.as_deref(),
            config.recording_max_bytes,
        )
        .map_err(|error| error.to_string())?;
        state = state.with_recording_delivery(client, config.iam_app_id.clone(), audience.clone())?;
        tracing::info!("automatic recording delivery is enabled");
    } else {
        tracing::warn!("Briefcase is not configured; new browser sessions are disabled");
    }

    let testing = TestingRegistry::new(config.clone(), state.clone())?;
    let mut app = production_router_with_testing(state.clone(), testing.clone());
    if let Some(webhook) = webhook {
        app = app.merge(webhook);
    }
    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .map_err(|error| format!("could not bind {}: {error}", config.bind))?;
    tracing::info!(address = %config.bind, "Silicon Browser backend listening");
    let reaper = spawn_ttl_reaper(state, Duration::from_secs(15));
    let test_maintenance = testing.spawn_maintenance(Duration::from_secs(15));
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|error| format!("HTTP server failed: {error}"));
    reaper.abort();
    test_maintenance.abort();
    result
}

async fn shutdown_signal() {
    let control_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "could not install Ctrl-C handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => tracing::error!(%error, "could not install termination handler"),
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = control_c => {},
        () = terminate => {},
    }
}
