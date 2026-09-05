//! Environment is read once at boot. Secrets stay in this value and are never logged.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;

use url::Url;

use crate::url_policy::is_https_or_loopback_http;

#[derive(Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub origin: String,
    pub database_url: String,
    pub encryption_key: [u8; 32],
    pub iam_url: String,
    pub iam_app_id: String,
    pub iam_app_secret: String,
    pub iam_test_environment_key: Option<String>,
    pub iam_webhook_secret: Option<String>,
    pub iam_webhook_key_version: i64,
    pub browser_use_api_key: String,
    pub tinyfish_api_keys: Vec<String>,
    pub briefcase_url: Option<String>,
    pub briefcase_app_id: Option<String>,
    pub briefcase_test_environment_key: Option<String>,
    pub recording_max_bytes: usize,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Self::from_vars(dotenv(Path::new(".env"), |name| std::env::var(name).ok()))
    }

    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let need = |name: &str| var(name).filter(|v| !v.trim().is_empty()).ok_or_else(|| format!("{name} is not set"));
        let port =
            var("PORT").unwrap_or_else(|| "8080".into()).parse::<u16>().map_err(|_| "PORT must be a port number")?;
        let origin = http_url(&need("SB_ORIGIN")?, "SB_ORIGIN")?;
        let iam_url = http_url(
            &var("SILICON_IAM_URL").unwrap_or_else(|| "https://backend.iam.teamofsilicons.com".into()),
            "SILICON_IAM_URL",
        )?;
        let key = need("SB_ENCRYPTION_KEY")?;
        if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("SB_ENCRYPTION_KEY must be 64 hexadecimal characters".into());
        }
        let mut encryption_key = [0_u8; 32];
        hex::decode_to_slice(&key, &mut encryption_key).map_err(|_| "SB_ENCRYPTION_KEY is not valid hexadecimal")?;
        let tinyfish_api_keys = var("TINYFISH_API_KEYS")
            .or_else(|| var("TINYFISH_API_KEY"))
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_owned)
            .collect();
        let briefcase_url = var("BRIEFCASE_URL")
            .filter(|value| !value.trim().is_empty())
            .map(|value| http_url(&value, "BRIEFCASE_URL"))
            .transpose()?;
        let briefcase_app_id = var("BRIEFCASE_APP_ID").filter(|value| !value.trim().is_empty());
        if briefcase_url.is_some() != briefcase_app_id.is_some() {
            return Err("BRIEFCASE_URL and BRIEFCASE_APP_ID must be configured together".into());
        }
        if briefcase_url.is_some()
            && var("IAM_TEST_ENVIRONMENT_KEY").is_some() != var("BRIEFCASE_TEST_ENVIRONMENT_KEY").is_some()
        {
            return Err("recording delivery requires both paired test keys or neither".into());
        }
        let recording_max_bytes = var("SB_RECORDING_MAX_BYTES")
            .unwrap_or_else(|| (512 * 1024 * 1024).to_string())
            .parse::<usize>()
            .map_err(|_| "SB_RECORDING_MAX_BYTES must be a positive byte count")?;
        if recording_max_bytes == 0 || recording_max_bytes as u64 > 5 * 1024_u64.pow(4) {
            return Err("SB_RECORDING_MAX_BYTES must be between 1 byte and 5 TiB".into());
        }
        Ok(Self {
            bind: SocketAddr::from(([0, 0, 0, 0], port)),
            origin,
            database_url: var("SB_DATABASE_URL").unwrap_or_else(|| "sqlite://silicon-browser.db?mode=rwc".into()),
            encryption_key,
            iam_url,
            iam_app_id: need("IAM_APP_ID")?,
            iam_app_secret: need("IAM_APP_SECRET")?,
            iam_test_environment_key: var("IAM_TEST_ENVIRONMENT_KEY"),
            iam_webhook_secret: var("IAM_WEBHOOK_SECRET").filter(|value| !value.is_empty()),
            iam_webhook_key_version: var("IAM_WEBHOOK_KEY_VERSION")
                .unwrap_or_else(|| "1".into())
                .parse()
                .map_err(|_| "IAM_WEBHOOK_KEY_VERSION must be an integer")?,
            browser_use_api_key: need("BROWSER_USE_API_KEY")?,
            tinyfish_api_keys,
            briefcase_url,
            briefcase_app_id,
            briefcase_test_environment_key: var("BRIEFCASE_TEST_ENVIRONMENT_KEY"),
            recording_max_bytes,
        })
    }
}

fn http_url(value: &str, name: &str) -> Result<String, String> {
    let parsed = Url::parse(value).ok().filter(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    });
    let parsed = parsed.ok_or_else(|| format!("{name} must be an HTTP(S) URL"))?;
    if !is_https_or_loopback_http(&parsed) {
        return Err(format!("{name} must use HTTPS unless its host is loopback"));
    }
    Ok(parsed.as_str().trim_end_matches('/').to_owned())
}

/// Real environment variables take precedence over a local dotenv file.
pub fn dotenv<E: Fn(&str) -> Option<String>>(path: &Path, env: E) -> impl Fn(&str) -> Option<String> + use<E> {
    let file: HashMap<String, String> = std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| {
            let value = value.trim();
            let unquoted =
                ['"', '\''].iter().find_map(|quote| value.strip_prefix(*quote)?.strip_suffix(*quote)).unwrap_or(value);
            (name.trim().to_owned(), unquoted.to_owned())
        })
        .collect();
    move |name| env(name).or_else(|| file.get(name).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> HashMap<&'static str, &'static str> {
        HashMap::from([
            ("PORT", "8081"),
            ("SB_ORIGIN", "https://browser.example/"),
            ("SB_ENCRYPTION_KEY", "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"),
            ("IAM_APP_ID", "tos>browser"),
            ("IAM_APP_SECRET", "ask_secret"),
            ("BROWSER_USE_API_KEY", "bu_secret"),
            ("TINYFISH_API_KEYS", "first, second"),
        ])
    }

    #[test]
    fn configuration_is_validated_and_normalized() {
        let values = values();
        let config = Config::from_vars(|name| values.get(name).map(ToString::to_string)).unwrap();
        assert_eq!(config.bind.port(), 8081);
        assert_eq!(config.origin, "https://browser.example");
        assert_eq!(config.tinyfish_api_keys, ["first", "second"]);
        assert_eq!(config.encryption_key[..3], [0x00, 0x11, 0x22]);
        assert!(config.iam_test_environment_key.is_none());
    }

    #[test]
    fn test_environment_key_is_forwarded_without_silently_falling_back_to_production() {
        for key in ["test-environment-key", ""] {
            let mut values = values();
            values.insert("IAM_TEST_ENVIRONMENT_KEY", key);
            let config = Config::from_vars(|name| values.get(name).map(ToString::to_string)).unwrap();
            assert_eq!(config.iam_test_environment_key.as_deref(), Some(key));
        }
    }

    #[test]
    fn configuration_names_the_first_bad_secret_without_echoing_it() {
        let mut values = values();
        values.insert("SB_ENCRYPTION_KEY", "not-a-key");
        let error = Config::from_vars(|name| values.get(name).map(ToString::to_string)).err().unwrap();
        assert_eq!(error, "SB_ENCRYPTION_KEY must be 64 hexadecimal characters");
        assert!(!error.contains("not-a-key"));
    }

    #[test]
    fn dotenv_never_overrides_the_process_environment() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join(".env");
        std::fs::write(&file, "PORT=9000\nSB_ORIGIN='https://file.example/'\n").unwrap();
        let read = dotenv(&file, |name| (name == "PORT").then(|| "8000".into()));
        assert_eq!(read("PORT").as_deref(), Some("8000"));
        assert_eq!(read("SB_ORIGIN").as_deref(), Some("https://file.example/"));
    }

    #[test]
    fn service_urls_cannot_embed_credentials_or_query_secrets() {
        for origin in ["https://user:secret@browser.example", "https://browser.example?token=secret"] {
            let mut values = values();
            values.insert("SB_ORIGIN", origin);
            let error = Config::from_vars(|name| values.get(name).map(ToString::to_string)).err().unwrap();
            assert_eq!(error, "SB_ORIGIN must be an HTTP(S) URL");
            assert!(!error.contains("secret"));
        }
    }

    #[test]
    fn service_urls_require_tls_except_for_loopback_development() {
        for (name, value) in [
            ("SB_ORIGIN", "http://browser.example"),
            ("SILICON_IAM_URL", "http://iam.example"),
            ("BRIEFCASE_URL", "http://briefcase.example"),
        ] {
            let mut values = values();
            values.insert(name, value);
            let error = Config::from_vars(|key| values.get(key).map(ToString::to_string)).err().unwrap();
            assert_eq!(error, format!("{name} must use HTTPS unless its host is loopback"));
        }

        let mut values = values();
        values.insert("SB_ORIGIN", "http://localhost:3000");
        values.insert("SILICON_IAM_URL", "http://127.0.0.1:8081");
        values.insert("BRIEFCASE_URL", "http://[::1]:8082");
        values.insert("BRIEFCASE_APP_ID", "tos>briefcase");
        assert!(Config::from_vars(|key| values.get(key).map(ToString::to_string)).is_ok());
    }
}
