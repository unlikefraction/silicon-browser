use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::RequestId;

/// Successful API response envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub data: T,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<RequestId>,
}

impl<T> Envelope<T> {
    pub fn new(data: T) -> Self {
        Self { data, request_id: None }
    }

    pub fn with_request_id(data: T, request_id: impl Into<RequestId>) -> Self {
        Self { data, request_id: Some(request_id.into()) }
    }

    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> Envelope<U> {
        Envelope { data: map(self.data), request_id: self.request_id }
    }
}

/// Machine-readable field error embedded in an API error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

/// Stable error payload. `code` intentionally remains open for future services.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldError>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub details: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<RequestId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

/// Failed API response envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiErrorEnvelope {
    pub error: ApiError,
}

impl ApiErrorEnvelope {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: ApiError {
                code: code.into(),
                message: message.into(),
                fields: Vec::new(),
                details: BTreeMap::new(),
                request_id: None,
                retry_after_ms: None,
            },
        }
    }
}

#[cfg(test)]
mod api_envelope_tests {
    use super::*;

    /// Test group: wire envelopes keep request correlation while mapping data.
    #[test]
    fn envelope_maps_without_losing_request_id() {
        let response = Envelope::with_request_id(4, "request-1").map(|value| value * 2);
        assert_eq!(response.data, 8);
        assert_eq!(response.request_id.as_deref(), Some("request-1"));
    }

    /// Test group: error wire format has one predictable top-level key.
    #[test]
    fn error_envelope_serializes_under_error_key() {
        let json = serde_json::to_value(ApiErrorEnvelope::new("not_found", "missing")).unwrap();
        assert_eq!(json["error"]["code"], "not_found");
        assert!(json.get("data").is_none());
    }
}
