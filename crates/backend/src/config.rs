//! Environment is read once at boot. Secrets stay in this value and are never logged.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;

use silicon_browser_shared::app_id;
use url::Url;

use crate::url_policy::is_https_or_loopback_http;

#[derive(Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub origin: String,
    pub database_url: String,
    pub encryption_key: [u8; 32],
    pub accounts_url: String,
    pub accounts_app_id: String,
    pub accounts_app_secret: String,
    pub accounts_webhook_secret: Option<String>,
    pub browser_use_api_key: String,
    pub tinyfish_api_keys: Vec<String>,
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
        let accounts_url = http_url(
            &var("ACCOUNTS_URL").unwrap_or_else(|| "https://accounts.teamofsilicons.com".into()),
            "ACCOUNTS_URL",
        )?;
        let accounts_app_id = var("ACCOUNTS_APP_ID").unwrap_or_else(|| "browser".into());
        app_id(&accounts_app_id, "ACCOUNTS_APP_ID").map_err(|_| "ACCOUNTS_APP_ID must be a bare application ID")?;
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
        Ok(Self {
            bind: SocketAddr::from(([0, 0, 0, 0], port)),
            origin,
            database_url: var("SB_DATABASE_URL")
                .unwrap_or_else(|| "sqlite://silicon-browser-accounts.db?mode=rwc".into()),
            encryption_key,
            accounts_url,
            accounts_app_id,
            accounts_app_secret: need("ACCOUNTS_APP_SECRET")?,
            accounts_webhook_secret: var("ACCOUNTS_WEBHOOK_SECRET").filter(|value| !value.is_empty()),
            browser_use_api_key: need("BROWSER_USE_API_KEY")?,
            tinyfish_api_keys,
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
    #[test]
    fn accounts_configuration_needs_only_current_auth_and_provider_secrets() {
        let values = HashMap::from([
            ("SB_ORIGIN", "https://browser.example"),
            ("SB_ENCRYPTION_KEY", "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"),
            ("ACCOUNTS_APP_SECRET", "sa_app_private"),
            ("BROWSER_USE_API_KEY", "bu_private"),
        ]);
        let config = Config::from_vars(|name| values.get(name).map(ToString::to_string)).unwrap();
        assert_eq!(config.accounts_url, "https://accounts.teamofsilicons.com");
        assert_eq!(config.accounts_app_id, "browser");
        assert_eq!(config.database_url, "sqlite://silicon-browser-accounts.db?mode=rwc");
        assert!(!config.database_url.contains("silicon-browser.db"));
    }
    #[test]
    fn dotenv_never_overrides_process_environment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(&path, "PORT=9000\nSB_ORIGIN='https://file.example/'\n").unwrap();
        let read = dotenv(&path, |name| (name == "PORT").then(|| "8000".into()));
        assert_eq!(read("PORT").as_deref(), Some("8000"));
        assert_eq!(read("SB_ORIGIN").as_deref(), Some("https://file.example/"));
    }
    #[test]
    fn service_urls_refuse_plaintext_and_embedded_secrets() {
        for value in
            ["http://accounts.example", "https://user:secret@accounts.example", "https://accounts.example?secret=x"]
        {
            assert!(http_url(value, "ACCOUNTS_URL").is_err());
        }
        assert!(http_url("http://127.0.0.1:8081", "ACCOUNTS_URL").is_ok());
    }
}
