use thiserror::Error;

/// A provider response may legitimately contain several fetched documents, but it must never be
/// allowed to grow without bound before JSON decoding. This limit applies to the decoded response
/// stream (including responses whose server omitted `Content-Length`).
const DEFAULT_MAX_JSON_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// Failure at an external-service boundary.
///
/// This type deliberately stores transport failures as already-sanitised text. `reqwest`
/// errors normally include the request URL, and Browser Use's CDP URLs are credentials.
#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("invalid provider input: {0}")]
    InvalidInput(String),

    #[error("{provider} does not support {feature}")]
    Unsupported { provider: &'static str, feature: &'static str },

    #[error("{provider} transport failed: {message}")]
    Transport { provider: &'static str, message: String },

    #[error("{provider} returned HTTP {status}: {message}")]
    Http { provider: &'static str, status: u16, message: String, retry_after: Option<std::time::Duration> },

    #[error("{provider} returned an invalid response: {message}")]
    InvalidResponse { provider: &'static str, message: String },

    #[error("{provider} scheduler queue is full (capacity {capacity})")]
    Overloaded { provider: &'static str, capacity: usize },
}

pub type ProviderResult<T> = Result<T, ProviderError>;

pub(crate) fn transport(provider: &'static str, error: reqwest::Error) -> ProviderError {
    ProviderError::Transport { provider, message: error.without_url().to_string() }
}

pub(crate) async fn json_response<T: serde::de::DeserializeOwned>(
    provider: &'static str,
    response: reqwest::Response,
) -> ProviderResult<T> {
    json_response_with_limit(provider, response, DEFAULT_MAX_JSON_RESPONSE_BYTES).await
}

pub(crate) async fn json_response_with_limit<T: serde::de::DeserializeOwned>(
    provider: &'static str,
    mut response: reqwest::Response,
    max_bytes: usize,
) -> ProviderResult<T> {
    let status = response.status();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after);
    if response.content_length().is_some_and(|length| length > max_bytes as u64) {
        return Err(response_too_large(provider, max_bytes));
    }

    let mut body = Vec::with_capacity(response.content_length().unwrap_or_default().min(max_bytes as u64) as usize);
    while let Some(chunk) = response.chunk().await.map_err(|error| transport(provider, error))? {
        append_limited(&mut body, &chunk, max_bytes).map_err(|()| response_too_large(provider, max_bytes))?;
    }
    if !status.is_success() {
        // Provider error bodies are untrusted and can echo request URLs, bearer query
        // parameters, API keys, or submitted form values. Keep the status for operational
        // handling, but never carry the raw body across the provider boundary.
        return Err(ProviderError::Http {
            provider,
            status: status.as_u16(),
            message: "provider response body redacted".into(),
            retry_after,
        });
    }

    // serde's diagnostics can quote an unrecognized enum value from the body. Treat the whole
    // upstream body as untrusted and keep it out of logs and public errors.
    serde_json::from_slice(&body).map_err(|_| ProviderError::InvalidResponse {
        provider,
        message: "provider response did not match the expected JSON contract".into(),
    })
}

fn append_limited(destination: &mut Vec<u8>, chunk: &[u8], limit: usize) -> Result<(), ()> {
    if chunk.len() > limit.saturating_sub(destination.len()) {
        return Err(());
    }
    destination.extend_from_slice(chunk);
    Ok(())
}

fn response_too_large(provider: &'static str, max_bytes: usize) -> ProviderError {
    ProviderError::InvalidResponse {
        provider,
        message: format!("provider response exceeded the {max_bytes}-byte safety limit"),
    }
}

fn parse_retry_after(value: &str) -> Option<std::time::Duration> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(std::time::Duration::from_secs(seconds));
    }
    let deadline = chrono::DateTime::parse_from_rfc2822(value).ok()?.with_timezone(&chrono::Utc);
    let seconds = deadline.signed_duration_since(chrono::Utc::now()).num_seconds().max(0) as u64;
    Some(std::time::Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_response_limit_is_checked_before_appending() {
        let mut body = b"1234".to_vec();
        assert_eq!(append_limited(&mut body, b"56", 6), Ok(()));
        assert_eq!(append_limited(&mut body, b"7", 6), Err(()));
        assert_eq!(body, b"123456");
    }

    #[tokio::test]
    async fn oversized_provider_body_is_a_typed_redacted_failure() {
        let (base, server) = super::super::test_http::spawn_header_only_server(
            200,
            vec![("Content-Length".into(), (DEFAULT_MAX_JSON_RESPONSE_BYTES + 1).to_string())],
        )
        .await;
        let response = reqwest::Client::new().get(base).send().await.unwrap();
        let error = json_response::<serde_json::Value>("test-provider", response).await.unwrap_err();
        server.await.unwrap();

        assert!(matches!(error, ProviderError::InvalidResponse { provider: "test-provider", .. }));
        assert!(error.to_string().contains("safety limit"));
        assert!(!error.to_string().contains("response body"));
    }

    #[tokio::test]
    async fn invalid_json_does_not_echo_untrusted_response_values() {
        let secret = "credential-shaped-secret";
        let body = format!(r#"{{"status":"{secret}"}}"#);
        let (base, _captured, server) = super::super::test_http::spawn_json_server(vec![(200, body)]).await;
        let response = reqwest::Client::new().get(base).send().await.unwrap();

        #[allow(dead_code)]
        #[derive(Debug, serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum ExpectedStatus {
            Ready,
        }
        #[derive(Debug, serde::Deserialize)]
        struct ExpectedBody {
            #[allow(dead_code)]
            status: ExpectedStatus,
        }

        let error = json_response::<ExpectedBody>("test-provider", response).await.unwrap_err();
        server.await.unwrap();
        assert!(matches!(error, ProviderError::InvalidResponse { .. }));
        assert!(!error.to_string().contains(secret));
    }
}
