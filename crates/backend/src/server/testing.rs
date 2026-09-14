//! IAM authenticates the selector before any test database is opened. The normal
//! API and providers then run against an isolated environment/clean generation.
use super::*;
use crate::auth::SiliconIamIdentityProvider;
use crate::config::Config;
use crate::providers::BriefcaseClient;
use axum::extract::Request;
use axum::routing::any;
use silicon_browser_shared::{TestingCredentials, TestingEnvironment};
use silicon_iam_client::models::ApplicationTestingContext;
use std::path::PathBuf;
use std::str::FromStr;
use tower::ServiceExt;

const APP_SECRET: &str = "x-sb-test-app-secret";
const IAM_KEY: &str = "x-testing-environment-key";
const BRIEFCASE_KEY: &str = "x-sb-test-briefcase-key";

#[derive(Clone)]
pub struct TestingRegistry(Arc<Registry>);

struct Registry {
    config: Config,
    template: AppState,
    directory: PathBuf,
    // ponytail: one initialization lock; split by environment if enrollment
    // throughput warrants it. Ordinary IAM verification runs outside this lock.
    states: Mutex<HashMap<String, AppState>>,
}

impl TestingRegistry {
    pub fn new(config: Config, template: AppState) -> Result<Self, String> {
        let options = sqlx::sqlite::SqliteConnectOptions::from_str(&config.database_url)
            .map_err(|_| "invalid SQLite database URL")?;
        let mut directory = options.get_filename().as_os_str().to_os_string();
        directory.push(".testing");
        let directory = PathBuf::from(directory);
        private_directory(&directory)?;
        Ok(Self(Arc::new(Registry { config, template, directory, states: Mutex::new(HashMap::new()) })))
    }

    pub(super) fn router(&self) -> Router {
        Router::new()
            .route("/api/v1/testing/context", post(context))
            .route("/testing/{environment_id}/{*path}", any(dispatch))
            .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY))
            .with_state(self.clone())
    }

    async fn verify(
        &self,
        credentials: &TestingCredentials,
    ) -> Result<(SiliconIamIdentityProvider, ApplicationTestingContext), ApiFailure> {
        credentials.validate().map_err(ApiFailure::validation)?;
        let (identity, context) = SiliconIamIdentityProvider::connect_testing_application(
            &self.0.config.iam_url,
            self.0.config.iam_app_id.clone(),
            credentials.app_secret.clone(),
        )
        .await
        .map_err(ApiFailure::from)?;
        if let Some(key) = &credentials.iam_test_key {
            let selected = SiliconIamIdentityProvider::connect_with_environment(
                &self.0.config.iam_url,
                self.0.config.iam_app_id.clone(),
                credentials.app_secret.clone(),
                Some(key),
            )
            .await?;
            if selected.testing_environment_id() != Some(context.environment_id) {
                return Err(ApiFailure::forbidden("IAM root key and app secret select different test environments"));
            }
        }
        Ok((identity, context))
    }

    async fn isolated_state(&self, namespace: &str) -> Result<AppState, ApiFailure> {
        // Names come from a verified IAM UUID and a local digest, never a path
        // supplied by a caller (also check persisted registry rows on restore).
        if !namespace.is_ascii()
            || namespace.len() != 101
            || Uuid::parse_str(&namespace[..36]).is_err()
            || namespace.as_bytes()[36] != b'-'
            || !namespace[37..].bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err(internal("stored test namespace is invalid"));
        }
        let mut states = self.0.states.lock().await;
        if let Some(state) = states.get(namespace) {
            return Ok(state.clone());
        }
        let path = self.0.directory.join(format!("{namespace}.db"));
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
                return Err(internal("test database must be a regular file"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                if let Err(error) = options.open(&path)
                    && error.kind() != std::io::ErrorKind::AlreadyExists
                {
                    return Err(internal("could not create test database"));
                }
            }
            Err(_) => return Err(internal("could not inspect test database")),
        }
        let url = Url::from_file_path(std::path::absolute(path).map_err(|_| internal("invalid test database path"))?)
            .map_err(|_| internal("invalid test database path"))?;
        let store = Store::connect(&format!("sqlite://{}?mode=rwc", url.path())).await?;
        let template = &self.0.template;
        let state = AppState::new(
            &*template.public_origin,
            store,
            template.secrets.clone(),
            Arc::new(ClosedIdentity),
            template.browser.clone(),
            template.search.clone(),
        )
        .map_err(|_| internal("could not initialize test application"))?
        .with_proxy_locations(template.proxy_locations.as_ref().clone())
        .require_recording_delivery();
        states.insert(namespace.to_owned(), state.clone());
        Ok(state)
    }

    fn authorize_state(
        &self,
        mut state: AppState,
        identity: SiliconIamIdentityProvider,
        credentials: &TestingCredentials,
    ) -> Result<AppState, ApiFailure> {
        state.testing_environment = identity.testing_environment_id();
        state.identity = Arc::new(identity);
        // Never send a test recording to production Briefcase storage. The
        // paired Briefcase environment is independently selected by its key.
        if let (Some(url), Some(audience), Some(key)) =
            (&self.0.config.briefcase_url, &self.0.config.briefcase_app_id, &credentials.briefcase_test_environment_key)
        {
            let client = BriefcaseClient::with_upload_limit(url, Some(key), self.0.config.recording_max_bytes)?;
            state = state
                .with_recording_delivery(client, self.0.config.iam_app_id.clone(), audience.clone())
                .map_err(|_| internal("could not configure test recording delivery"))?;
        }
        Ok(state)
    }

    async fn remember(
        &self,
        namespace: &str,
        environment_id: Uuid,
        credentials: &TestingCredentials,
    ) -> Result<TestingCredentials, ApiFailure> {
        let mut transaction = self
            .0
            .template
            .store
            .pool()
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| internal("could not lock test configuration"))?;
        let stored: Option<String> =
            sqlx::query_scalar("SELECT credentials FROM testing_environments WHERE namespace = ?")
                .bind(namespace)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|_| internal("could not read test configuration"))?;
        let context = format!("testing:{namespace}");
        let mut credentials = credentials.clone();
        let stored = stored
            .map(|value| self.0.template.secrets.open_for(&context, &value))
            .transpose()
            .map_err(|_| internal("could not decrypt existing test configuration"))?;
        if let Some(stored) = &stored {
            let previous: TestingCredentials =
                serde_json::from_str(stored).map_err(|_| internal("stored test configuration is invalid"))?;
            // A reader using only an app secret must not remove the environment's
            // recording configuration and strand already queued deliveries.
            if credentials.briefcase_test_environment_key.is_none() {
                credentials.briefcase_test_environment_key = previous.briefcase_test_environment_key;
            }
        }
        let plaintext =
            serde_json::to_string(&credentials).map_err(|_| internal("could not encode test configuration"))?;
        if stored.as_deref() == Some(&plaintext) {
            transaction.commit().await.map_err(|_| internal("could not unlock test configuration"))?;
            return Ok(credentials);
        }
        let encrypted = self
            .0
            .template
            .secrets
            .seal_for(&context, &plaintext)
            .map_err(|_| internal("could not encrypt test configuration"))?;
        sqlx::query("INSERT INTO testing_environments (namespace, environment_id, credentials) VALUES (?, ?, ?) ON CONFLICT(namespace) DO UPDATE SET credentials = excluded.credentials")
            .bind(namespace).bind(environment_id.to_string()).bind(encrypted).execute(&mut *transaction).await
            .map_err(|_| internal("could not persist test configuration"))?;
        transaction.commit().await.map_err(|_| internal("could not persist test configuration"))?;
        Ok(credentials)
    }

    /// Restores durable test cleanup after restart, even when IAM is down or a
    /// test secret was revoked. Delivery additionally requires fresh IAM proof.
    pub fn spawn_maintenance(&self, interval: Duration) -> tokio::task::JoinHandle<()> {
        let registry = self.clone();
        tokio::spawn(async move {
            let mut timer = tokio::time::interval(interval);
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                timer.tick().await;
                if registry.maintain_once().await.is_err() {
                    tracing::error!("test environment maintenance failed; will retry");
                }
            }
        })
    }

    async fn maintain_once(&self) -> Result<(), ApiFailure> {
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT namespace, credentials FROM testing_environments")
            .fetch_all(self.0.template.store.pool())
            .await
            .map_err(|_| internal("could not load test configuration"))?;
        for batch in rows.chunks(8) {
            let mut tasks = tokio::task::JoinSet::new();
            for (namespace, encrypted) in batch.iter().cloned() {
                let registry = self.clone();
                tasks.spawn(async move {
                    let state = registry.isolated_state(&namespace).await?;
                    let delivery = async {
                        let plaintext = registry
                            .0
                            .template
                            .secrets
                            .open_for(&format!("testing:{namespace}"), &encrypted)
                            .map_err(|_| internal("could not decrypt test configuration"))?;
                        let credentials: TestingCredentials = serde_json::from_str(&plaintext)
                            .map_err(|_| internal("stored test configuration is invalid"))?;
                        let (identity, context) = registry.verify(&credentials).await?;
                        if namespace_for(&context)? != namespace {
                            return Ok(()); // IAM clean retired this generation.
                        }
                        registry
                            .authorize_state(state.clone(), identity, &credentials)?
                            .deliver_recordings_once()
                            .await?;
                        Ok::<_, ApiFailure>(())
                    };
                    let (ttl, profiles, sources, delivery) = tokio::join!(
                        state.reap_expired_sessions_once(),
                        state.reconcile_profiles_once(),
                        state.reconcile_recording_sources_once(Utc::now()),
                        delivery,
                    );
                    ttl?;
                    profiles?;
                    sources?;
                    delivery?;
                    Ok::<_, ApiFailure>(())
                });
            }
            while let Some(result) = tasks.join_next().await {
                if !matches!(result, Ok(Ok(()))) {
                    tracing::warn!("test environment cleanup or delivery needs retry; check IAM test credentials");
                }
            }
        }
        Ok(())
    }
}

fn private_directory(path: &std::path::Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(path)
            .map_err(|_| "could not create test data directory")?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(path).map_err(|_| "could not create test data directory")?;
    let metadata = std::fs::symlink_metadata(path).map_err(|_| "could not inspect test data directory")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("test data directory must be a real directory".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "could not secure test data directory")?;
    }
    Ok(())
}

fn namespace_for(context: &ApplicationTestingContext) -> Result<String, ApiFailure> {
    let metadata = context.environment.as_ref().ok_or_else(|| {
        ApiFailure::bad_gateway(
            "iam_contract",
            "IAM omitted test lifecycle metadata; refusing to reuse potentially cleaned data",
        )
    })?;
    if metadata.environment_id != context.environment_id || context.environment_id.is_nil() {
        return Err(ApiFailure::bad_gateway("iam_contract", "IAM returned a different test environment"));
    }
    let generation =
        metadata.cleaned_at.map(|time| time.unix_timestamp_nanos().to_string()).unwrap_or_else(|| "initial".into());
    Ok(format!("{}-{}", context.environment_id, hex::encode(Sha256::digest(generation.as_bytes()))))
}

async fn context(
    State(registry): State<TestingRegistry>,
    payload: Result<Json<TestingCredentials>, JsonRejection>,
) -> Result<Response, ApiFailure> {
    let credentials = json_payload(payload)?;
    let (_, context) = registry.verify(&credentials).await?;
    namespace_for(&context)?;
    let environment = TestingEnvironment {
        environment_id: context.environment_id,
        app_id: registry.0.config.iam_app_id.clone(),
        name: context.environment.as_ref().expect("namespace validates metadata").name.clone(),
    };
    let mut response = success(environment).into_response();
    response.headers_mut().insert("cache-control", HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn dispatch(
    State(registry): State<TestingRegistry>,
    Path((environment_id, _path)): Path<(String, String)>,
    request: Request,
) -> Result<Response, ApiFailure> {
    let environment_id = Uuid::parse_str(&environment_id)
        .map_err(|_| ApiFailure::bad_request("invalid_test_environment", "test environment must be a UUID"))?;
    let credentials = credentials(request.headers())?;
    let (identity, context) = registry.verify(&credentials).await?;
    if context.environment_id != environment_id {
        return Err(ApiFailure::forbidden("test app secret belongs to a different IAM environment"));
    }
    let namespace = namespace_for(&context)?;
    let state = registry.isolated_state(&namespace).await?;
    let credentials = registry.remember(&namespace, environment_id, &credentials).await?;
    let state = registry.authorize_state(state, identity, &credentials)?;
    // Rebuild the request so the outer route's Path captures cannot leak into
    // handlers such as /sessions/{session_id}. Keep the original escaped path.
    let path = request.uri().path().splitn(4, '/').nth(3).unwrap_or("");
    let uri = format!("/{path}{}", request.uri().query().map(|q| format!("?{q}")).unwrap_or_default());
    let (parts, body) = request.into_parts();
    let mut inner = Request::new(body);
    *inner.method_mut() = parts.method;
    *inner.version_mut() = parts.version;
    *inner.headers_mut() = parts.headers;
    *inner.uri_mut() = uri.parse().map_err(|_| ApiFailure::bad_request("invalid_path", "test API path is invalid"))?;
    let mut response = router(state).oneshot(inner).await.expect("Router is infallible");
    response.headers_mut().insert("cache-control", HeaderValue::from_static("no-store"));
    Ok(response)
}

fn credentials(headers: &HeaderMap) -> Result<TestingCredentials, ApiFailure> {
    let header = |name: &'static str| -> Result<Option<String>, ApiFailure> {
        if headers.get_all(name).iter().count() > 1 {
            return Err(ApiFailure::bad_request("invalid_test_credentials", "test credential headers must occur once"));
        }
        headers
            .get(name)
            .map(|value| {
                value
                    .to_str()
                    .map(str::to_owned)
                    .map_err(|_| ApiFailure::bad_request("invalid_test_credentials", "test credentials must be ASCII"))
            })
            .transpose()
    };
    let credentials = TestingCredentials {
        app_secret: header(APP_SECRET)?.ok_or_else(|| {
            ApiFailure::new(
                StatusCode::UNAUTHORIZED,
                "test_credentials_required",
                "test route requires x-sb-test-app-secret; run sb testing login",
            )
        })?,
        iam_test_key: header(IAM_KEY)?,
        briefcase_test_environment_key: header(BRIEFCASE_KEY)?,
    };
    credentials.validate().map_err(ApiFailure::validation)?;
    Ok(credentials)
}

pub(super) async fn reject_misdirected_credentials(
    request: Request,
    next: axum::middleware::Next,
) -> Result<Response, ApiFailure> {
    if [APP_SECRET, IAM_KEY, BRIEFCASE_KEY].iter().any(|name| request.headers().contains_key(*name)) {
        return Err(ApiFailure::bad_request(
            "test_route_required",
            "test credentials require /testing/<environment-id>/api/v1; refusing production routing",
        ));
    }
    Ok(next.run(request).await)
}

fn internal(message: &'static str) -> ApiFailure {
    ApiFailure::new(StatusCode::INTERNAL_SERVER_ERROR, "testing_storage", message)
}

// Cleanup-only states restored from disk must never borrow production identity.
struct ClosedIdentity;
#[async_trait::async_trait]
impl IdentityProvider for ClosedIdentity {
    async fn identify(&self, _: &str, _: &str) -> Result<PrincipalIdentity, IdentityError> {
        Err(IdentityError::Unauthenticated)
    }
    async fn orgs(&self, _: &str) -> Result<Vec<OrganizationAccess>, IdentityError> {
        Err(IdentityError::Unauthenticated)
    }
    async fn exchange_short_lived_token(&self, _: ExchangeRequest) -> Result<ExchangedAuth, IdentityError> {
        Err(IdentityError::Unauthenticated)
    }
    async fn refresh(&self, _: RefreshRequest) -> Result<ExchangedAuth, IdentityError> {
        Err(IdentityError::Unauthenticated)
    }
}
