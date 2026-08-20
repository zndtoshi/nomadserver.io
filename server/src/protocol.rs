//! Protocol envelope types — the decrypted content of gift-wrap rumors.
//! Canonical shapes live in `shared/schemas/v1/`; these structs mirror
//! them and PROTOCOL.md §3. Anything protocol-visible defined here must
//! stay in sync with the schemas.

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u8 = 1;

/// Rumor kinds (never published bare; only inside NIP-59 seals).
pub const KIND_REQUEST: u16 = 25078;
pub const KIND_RESPONSE: u16 = 25079;
#[allow(dead_code)] // used by the watcher phase (notify/new_tx)
pub const KIND_NOTIFY: u16 = 25080;

/// Known v1 request types (PROTOCOL.md §5).
pub const KNOWN_TYPES: &[&str] = &[
    "pair",
    "unpair",
    "ping",
    "get_balance",
    "get_utxos",
    "get_history",
    "get_transactions",
    "get_fee_estimates",
    "broadcast_tx",
    "watch_addresses",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestEnvelope {
    pub v: u8,
    pub id: String,
    pub ts: u64,
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default)]
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    UnsupportedVersion,
    UnknownType,
    NotPaired,
    PairingFailed,
    InvalidRequest,
    InvalidTx,
    BroadcastFailed,
    BackendUnavailable,
    RateLimited,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseEnvelope {
    pub v: u8,
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

impl ResponseEnvelope {
    pub fn ok(id: &str, result: serde_json::Value) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id: id.to_string(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: &str, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id: id.to_string(),
            ok: false,
            result: None,
            error: Some(ErrorBody {
                code,
                message: message.into(),
            }),
        }
    }
}

/// Parse and minimally validate a request envelope (PROTOCOL.md §3, §7).
/// Returns the envelope, or the error response to send back (the id is
/// echoed when it could be read).
pub fn parse_request(content: &str) -> Result<RequestEnvelope, Option<ResponseEnvelope>> {
    if content.len() > 256 * 1024 {
        return Err(None); // oversized: no id trusted, drop
    }
    let env: RequestEnvelope = match serde_json::from_str(content) {
        Ok(e) => e,
        Err(_) => return Err(None), // unreadable: nothing to correlate, drop
    };
    if env.v != PROTOCOL_VERSION {
        return Err(Some(ResponseEnvelope::err(
            &env.id,
            ErrorCode::UnsupportedVersion,
            "unsupported protocol version",
        )));
    }
    if env.id.is_empty() || env.id.len() > 64 || uuid::Uuid::parse_str(&env.id).is_err() {
        return Err(Some(ResponseEnvelope::err(
            "00000000-0000-0000-0000-000000000000",
            ErrorCode::InvalidRequest,
            "id must be a UUID",
        )));
    }
    Ok(env)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_request() {
        let env = parse_request(
            r#"{"v":1,"id":"7c9e6679-7425-40de-944b-e07fc1f90ae7","ts":1,"type":"ping","payload":{}}"#,
        )
        .unwrap();
        assert_eq!(env.msg_type, "ping");
    }

    #[test]
    fn rejects_bad_version_with_echoed_id() {
        let err = parse_request(
            r#"{"v":2,"id":"7c9e6679-7425-40de-944b-e07fc1f90ae7","ts":1,"type":"ping","payload":{}}"#,
        )
        .unwrap_err()
        .unwrap();
        assert_eq!(err.error.unwrap().code, ErrorCode::UnsupportedVersion);
    }

    #[test]
    fn drops_unparseable_and_bad_id() {
        assert!(parse_request("not json").unwrap_err().is_none());
        let err = parse_request(
            r#"{"v":1,"id":"nope","ts":1,"type":"ping","payload":{}}"#,
        )
        .unwrap_err()
        .unwrap();
        assert_eq!(err.error.unwrap().code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn response_roundtrip_ok_and_err() {
        let ok = ResponseEnvelope::ok("id1", serde_json::json!({"watching": 3}));
        let s = serde_json::to_string(&ok).unwrap();
        assert!(s.contains("\"ok\":true"));
        assert!(!s.contains("error"));
        let err = ResponseEnvelope::err("id1", ErrorCode::NotPaired, "no");
        let s = serde_json::to_string(&err).unwrap();
        assert!(s.contains("\"not_paired\""));
        assert!(!s.contains("result"));
    }
}
