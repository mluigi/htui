//! Infisical's JSON shapes (MOD-10 D6), only the fields htui reads (H-10).
//!
//! **No `Debug` on any of them** (H-2): the login request holds the client secret, the login
//! answer the token, a listed secret its value. No `deny_unknown_fields` either: a server upgrade
//! adds fields.

use serde::{Deserialize, Serialize};

/// `POST /api/v1/auth/universal-auth/login` body.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoginRequest<'a> {
    /// `clientId`.
    pub(crate) client_id: &'a str,
    /// `clientSecret`.
    pub(crate) client_secret: &'a str,
}

/// Its 200 answer. Only the read fields are declared (H-10): serde ignores `accessTokenMaxTTL`,
/// `tokenType` and anything new. `zeroize` 1.9 has no `serde` feature, so the token is a `String`
/// moved into `Zeroizing` right after decoding.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoginResponse {
    /// `accessToken`.
    pub(crate) access_token: String,
    /// `expiresIn`, seconds.
    pub(crate) expires_in: u64,
}

/// `GET /api/v4/secrets` 200 answer.
#[derive(Deserialize)]
pub(crate) struct ListResponse {
    /// The folder's own secrets.
    pub(crate) secrets: Vec<WireSecret>,
    /// Absent or null → no imports.
    #[serde(default)]
    pub(crate) imports: Option<Vec<WireImport>>,
}

/// One listed secret.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireSecret {
    /// `secretKey`.
    pub(crate) secret_key: String,
    /// `secretValue`.
    #[serde(default)]
    pub(crate) secret_value: String,
    /// `secretValueHidden`.
    #[serde(default)]
    pub(crate) secret_value_hidden: bool,
    /// `type`: `"shared"` | `"personal"`; absent → shared.
    #[serde(rename = "type", default)]
    pub(crate) kind: Option<String>,
}

/// One import. `secretPath` and `environment` are not read, so not declared (H-10).
#[derive(Deserialize)]
pub(crate) struct WireImport {
    /// The imported secrets.
    pub(crate) secrets: Vec<WireSecret>,
}

/// `{reqId, statusCode, message, error}`. Only `message` and `error` are read. `message` is a
/// `Value` because validation errors send an array.
#[derive(Deserialize, Default)]
pub(crate) struct ErrorBody {
    #[serde(default)]
    message: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<String>,
}

impl ErrorBody {
    /// Parsed leniently: a body that is not this JSON is `ErrorBody::default()`.
    pub(crate) fn from_bytes(body: &[u8]) -> Self {
        serde_json::from_slice(body).unwrap_or_default()
    }

    /// `message` when it is a JSON string, else `""`.
    pub(crate) fn message(&self) -> &str {
        self.message
            .as_ref()
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
    }

    /// `error`, or `""`.
    pub(crate) fn error(&self) -> &str {
        self.error.as_deref().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;
    use crate::infisical::{Merged, merge};

    fn list(value: serde_json::Value) -> ListResponse {
        serde_json::from_value(value).expect("a list body")
    }

    fn shared(key: &str, value: &str) -> serde_json::Value {
        json!({"secretKey": key, "secretValue": value, "type": "shared"})
    }

    fn personal(key: &str, value: &str) -> serde_json::Value {
        json!({"secretKey": key, "secretValue": value, "type": "personal"})
    }

    fn values(merged: &BTreeMap<String, Merged>) -> Vec<(&str, &str)> {
        merged
            .iter()
            .map(|(k, m)| (k.as_str(), m.value.as_str()))
            .collect()
    }

    #[test]
    fn merge_worked_example() {
        let merged = merge(list(json!({
            "secrets": [shared("A", "f1"), shared("B", "f2"), personal("P", "p1")],
            "imports": [
                {"secretPath": "/a", "environment": "dev",
                 "secrets": [shared("B", "i0b"), shared("C", "i0c"), shared("D", "i0d")]},
                {"secretPath": "/b", "environment": "dev",
                 "secrets": [shared("C", "i1c"), personal("E", "e1"), shared("D", "i1d")]},
            ],
        })));
        assert_eq!(
            values(&merged),
            [("A", "f1"), ("B", "f2"), ("C", "i1c"), ("D", "i1d")]
        );
    }

    #[test]
    fn merge_keeps_the_first_duplicate_within_one_list() {
        let merged = merge(list(json!({
            "secrets": [shared("A", "first"), shared("A", "second")],
            "imports": [{"secrets": [shared("B", "one"), shared("B", "two")]}],
        })));
        assert_eq!(values(&merged), [("A", "first"), ("B", "one")]);
    }

    #[test]
    fn the_login_request_serialises_camel_case() {
        let body = serde_json::to_value(LoginRequest {
            client_id: "cid",
            client_secret: "csecret",
        })
        .expect("serialises");
        assert_eq!(body, json!({"clientId": "cid", "clientSecret": "csecret"}));
    }

    #[test]
    fn a_list_body_with_unknown_fields_and_null_imports_parses() {
        let parsed = list(json!({
            "secrets": [{"id": "1", "version": 3, "secretKey": "K", "secretValue": "v",
                         "secretValueHidden": false, "type": "shared", "tags": []}],
            "imports": null,
            "something": "new",
        }));
        assert_eq!(parsed.secrets.len(), 1);
        assert!(parsed.imports.is_none());
        let absent = list(json!({"secrets": [{"secretKey": "K"}]}));
        assert!(absent.imports.is_none());
        assert_eq!(absent.secrets[0].secret_value, "");
        assert!(!absent.secrets[0].secret_value_hidden);
        assert!(absent.secrets[0].kind.is_none());
    }

    #[test]
    fn an_error_body_that_is_not_json_is_empty() {
        let body = ErrorBody::from_bytes(b"<html>");
        assert_eq!((body.message(), body.error()), ("", ""));
        let array = ErrorBody::from_bytes(br#"{"message":["a","b"],"error":"ValidationError"}"#);
        assert_eq!((array.message(), array.error()), ("", "ValidationError"));
        let plain = ErrorBody::from_bytes(br#"{"message":"m","error":"NotFound","reqId":"r"}"#);
        assert_eq!((plain.message(), plain.error()), ("m", "NotFound"));
    }
}
