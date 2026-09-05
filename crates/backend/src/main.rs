use std::sync::Arc;
use std::time::Duration;

use silicon_browser_backend::auth::SiliconIamIdentityProvider;
use silicon_browser_backend::config::Config;
use silicon_browser_backend::crypto::SecretBox;
use silicon_browser_backend::providers::{BriefcaseClient, BrowserUseV3, FairSearchPool};
use silicon_browser_backend::store::Store;
use silicon_browser_backend::{AppState, production_router, spawn_ttl_reaper};

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
    let config = Config::from_env()?;
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

    let mut app = production_router(state.clone());
    if let Some(webhook) = webhook {
        app = app.merge(webhook);
    }
    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .map_err(|error| format!("could not bind {}: {error}", config.bind))?;
    tracing::info!(address = %config.bind, "Silicon Browser backend listening");
    let reaper = spawn_ttl_reaper(state, Duration::from_secs(15));
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|error| format!("HTTP server failed: {error}"));
    reaper.abort();
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
