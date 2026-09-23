//! Optional, default-on diagnostics in the dedicated `tos.siliconiam` Space Station table.
//!
//! The recording key stays in deployment configuration or a private local file.
//! Only typed operational context is sent: no bodies, credentials, contacts,
//! error messages, argument values, query strings, cookies or authorization headers.
use serde_json::{Map, Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::Duration,
};

/// Dedicated IAM event table, shared by backend, CLI, daemon and web.
pub const TABLE: &str = "siliconiam";
/// Organization owning the dedicated telemetry table.
pub const ORGANIZATION: &str = "tos";
/// Production Space Station ingest service.
pub const URL: &str = "https://backend.spacestation.teamofsilicons.com";

/// A bounded, nonblocking recorder. Clones share one sender; Debug reveals no key.
#[derive(Clone)]
pub struct Telemetry {
    #[cfg(unix)]
    client: Arc<space_station::SpaceClient>,
    source: &'static str,
    environment: String,
    instance_id: String,
    isi: Option<String>,
}
impl std::fmt::Debug for Telemetry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Telemetry")
            .field("source", &self.source)
            .field("table", &TABLE)
            .finish_non_exhaustive()
    }
}

/// Environment-wide kill switch. A caller's explicit opt-out always wins.
#[must_use]
pub fn enabled(preference: bool) -> bool {
    preference
        && !std::env::var("IAM_TELEMETRY").is_ok_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "no"
            )
        })
}

/// Standard private IAM directory for telemetry credentials and durable spool.
#[must_use]
pub fn home() -> Option<PathBuf> {
    std::env::var_os("SILICON_IAM_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("SILICON_HOME")
                .or_else(|| std::env::var_os("HOME"))
                .map(|p| {
                    let root = PathBuf::from(p).join(".silicon-iam");
                    std::fs::read_to_string(root.join(".silicon-iam-home"))
                        .ok()
                        .filter(|value| !value.trim().is_empty())
                        .map_or(root, |value| PathBuf::from(value.trim()))
                })
        })
}

impl Telemetry {
    /// Load the opt-out and table key without failing an IAM operation.
    ///
    /// # Errors
    /// Returns a nonsecret diagnostic for invalid configuration. Missing keys
    /// mean unconfigured, and explicit opt-out never reads credentials or starts a daemon.
    pub fn from_env(source: &'static str, preference: bool) -> Result<Option<Self>, String> {
        if !enabled(preference) {
            return Ok(None);
        }
        let key = match std::env::var("IAM_TELEMETRY_KEY") {
            Ok(key) if !key.trim().is_empty() => Some(key),
            _ => {
                let path = std::env::var_os("IAM_TELEMETRY_KEY_FILE")
                    .map(PathBuf::from)
                    .or_else(|| home().map(|p| p.join("telemetry.key")));
                path.map(|p| read_key(&p)).transpose()?.flatten()
            }
        };
        let Some(key) = key else {
            return Ok(None);
        };
        let spool = std::env::var_os("IAM_TELEMETRY_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|p| p.join("telemetry-spool")))
            .ok_or("Cannot locate telemetry spool; set IAM_TELEMETRY_HOME")?;
        let url = std::env::var("IAM_TELEMETRY_URL").unwrap_or_else(|_| URL.into());
        Self::new(key.trim(), &url, &spool, source).map(Some)
    }

    /// Configure a recorder explicitly, including for local protocol verification.
    ///
    /// # Errors
    /// Rejects a key for another table or an unsafe service URL. The Space Station
    /// Rust recorder supports Unix (macOS/Linux); other targets return a clear error.
    pub fn new(key: &str, url: &str, spool: &Path, source: &'static str) -> Result<Self, String> {
        let suffix = key
            .strip_prefix("table-siliconiam-")
            .ok_or("Telemetry requires a siliconiam table key")?;
        if suffix.len() != 32 || !suffix.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid siliconiam telemetry key".into());
        }
        let parsed = url::Url::parse(url).map_err(|_| "Invalid telemetry service URL")?;
        let loopback = match parsed.host() {
            Some(url::Host::Domain("localhost")) => true,
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        };
        if !(parsed.scheme() == "https" || parsed.scheme() == "http" && loopback)
            || parsed.host_str().is_none()
            || parsed.port() == Some(0)
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || parsed.path() != "/"
        {
            return Err(
                "Telemetry URL must be an HTTPS origin (HTTP is allowed only on loopback)".into(),
            );
        }
        #[cfg(unix)]
        {
            // Reqwest and Space Station can enable different Rustls providers.
            // Respect a host-installed provider; otherwise select ring explicitly.
            let _ = rustls::crypto::ring::default_provider().install_default();
            let client = space_station::SpaceClient::builder(key)
                .url(url)
                .home(spool)
                .flush_timeout(Duration::from_millis(100))
                .on_error(|_| {})
                .build()
                .map_err(|_| "Cannot initialize Space Station telemetry recorder")?;
            let environment = std::env::var("IAM_ENVIRONMENT")
                .ok()
                .filter(|v| matches!(v.as_str(), "production" | "development" | "test"))
                .unwrap_or_else(|| "client".into());
            Ok(Self {
                client: Arc::new(client),
                source,
                environment,
                instance_id: uuid::Uuid::now_v7().to_string(),
                isi: std::env::var("ISI").ok(),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = (spool, source);
            Err("The Space Station Rust recorder requires macOS or Linux".into())
        }
    }

    /// Queue an operational event. Names must be static; context is allowlisted.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "owned JSON context is consumed conceptually by the nonblocking recorder API"
    )]
    pub fn record(&self, step: &'static str, event: &'static str, context: Value) {
        let mut context = safe_context(&context);
        if let Some(isi) = &self.isi
            && let Some(value) = safe_context(&json!({"isi":isi})).get("isi")
        {
            context["isi"] = value.clone();
        }
        #[cfg(unix)]
        self.client.record(json!({
            "schema_version": 1, "service": "silicon-iam", "source": self.source,
            "step": step, "event": event, "version": env!("CARGO_PKG_VERSION"),
            "environment": self.environment, "instance_id": self.instance_id,
            "progress": u8::from(!event.ends_with("started")),
            "context": context,
        }));
    }

    /// Bounded spool/ack handoff; a true result alone does not prove remote acceptance.
    #[must_use]
    pub fn flush(&self) -> bool {
        #[cfg(unix)]
        {
            self.client.flush()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }
}

fn read_key(path: &Path) -> Result<Option<String>, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Cannot read telemetry key file".into()),
    };
    if !metadata.is_file() || metadata.len() > 256 {
        return Err("Telemetry key must be a small regular private file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("Telemetry key file must have mode 0600".into());
        }
    }
    std::fs::read_to_string(path)
        .map(Some)
        .map_err(|_| "Cannot read telemetry key file".into())
}

/// Map a request path to its public contract template, dropping all identifiers.
#[must_use]
pub fn route(path: &str) -> String {
    static ROUTES: OnceLock<Vec<String>> = OnceLock::new();
    let routes = ROUTES.get_or_init(|| {
        serde_json::from_str(include_str!("telemetry_routes.json")).unwrap_or_default()
    });
    let path = path.split(['?', '#']).next().unwrap_or("");
    let parts: Vec<_> = path.split('/').collect();
    routes
        .iter()
        .find(|template| {
            let pattern: Vec<_> = template.split('/').collect();
            pattern.len() == parts.len()
                && pattern.iter().zip(&parts).all(|(expected, value)| {
                    !value.is_empty() && expected.starts_with('{') && expected.ends_with('}')
                        || expected == value
                })
        })
        .cloned()
        .unwrap_or_else(|| "<unmatched>".into())
}

/// Drop free-form data; retain only bounded operational measurements and labels.
#[must_use]
pub fn safe_context(value: &Value) -> Value {
    let mut safe = Map::new();
    let Some(values) = value.as_object() else {
        return Value::Object(safe);
    };
    for (key, value) in values {
        let allowed = match key.as_str() {
            "duration_ms" | "latency_ms" | "status" | "exit_code" | "count" | "attempt"
            | "line" | "progress" => value.is_number(),
            "testing" | "success" => value.is_boolean(),
            "request_id" | "invocation_id" | "testing_environment_id" => value
                .as_str()
                .is_some_and(|v| uuid::Uuid::parse_str(v).is_ok()),
            "route" => {
                if let Some(path) = value.as_str() {
                    safe.insert(key.clone(), Value::String(route(path)));
                }
                false
            }
            "method" => value.as_str().is_some_and(|v| {
                matches!(
                    v,
                    "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
                )
            }),
            "command" | "stage" | "code" | "target" | "outcome" | "level" | "isi" => {
                value.as_str().is_some_and(|v| {
                    v.len() <= 160
                        && v.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"._: -/".contains(&b))
                        && !contains_secret(v)
                })
            }
            _ => false,
        };
        if allowed {
            safe.insert(key.clone(), value.clone());
        }
    }
    Value::Object(safe)
}
fn contains_secret(value: &str) -> bool {
    [
        "stk-", "slt_", "rft_", "sat_", "aak_", "sscli-", "table-", "Bearer ", "apikey-",
    ]
    .iter()
    .any(|prefix| value.contains(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn records_cannot_contain_credentials_contacts_or_unbounded_error_text() {
        let value = safe_context(
            &json!({"route":"/api/v1/carbon-ids/alice@example.com/availability?otp=123456", "method":"GET", "status":401, "error":"secret", "token":"slt_secret", "body":{"phone":"secret"}, "command":"sat_secret", "request_id":uuid::Uuid::from_u128(4)}),
        );
        assert_eq!(
            value["route"],
            "/api/v1/carbon-ids/{carbon_id}/availability"
        );
        assert_eq!(value["status"], 401);
        assert!(value.get("error").is_none());
        assert!(value.get("body").is_none());
        assert!(value.get("command").is_none());
        assert!(!value.to_string().contains("alice"));
        assert_eq!(route("/unknown/slt_secret"), "<unmatched>");
    }

    #[test]
    fn app_identity_keys_are_dropped_even_from_allowlisted_labels() {
        let value = safe_context(&json!({
            "command":"iam app verification issue", "outcome":"aak_secret",
            "code":"received_aak_secret", "app_access_key":"aak_secret"
        }));
        assert_eq!(value, json!({"command":"iam app verification issue"}));
    }
    #[test]
    fn wrong_table_and_insecure_destinations_are_rejected_before_any_sender_starts() {
        assert!(
            Telemetry::new(
                "table-other-0123456789abcdef0123456789abcdef",
                URL,
                Path::new("/unused"),
                "test"
            )
            .is_err()
        );
        assert!(
            Telemetry::new(
                "table-siliconiam-0123456789abcdef0123456789abcdef",
                "http://example.com",
                Path::new("/unused"),
                "test"
            )
            .is_err()
        );
    }
}
