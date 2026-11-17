//! The privileged-operation protocol between `serve` (and later `proxy`) and `priv`.
//!
//! `PrivOp` is a closed enum. `priv` rejects anything that does not deserialize
//! into it; there is no "run this shell string" operation and there never will be.

use serde::{Deserialize, Serialize};

/// A request frame. `id` correlates the response and the logs on both sides; `serve`
/// reuses the HTTP request id so one request can be followed across processes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivRequest {
    pub id: String,
    /// Who the operation is performed for, recorded by `priv` for audit.
    #[serde(default)]
    pub actor: Option<String>,
    pub op: PrivOp,
}

/// Every operation `priv` can perform. Add a variant here, then a module in
/// `opsbox-priv`, and nothing else becomes possible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "args", rename_all = "snake_case")]
pub enum PrivOp {
    /// Liveness check. Answered with [`PrivReply::Pong`].
    Ping,
    /// Facts about the machine. Answered with [`PrivReply::HostInfo`].
    HostInfo,
}

impl PrivOp {
    /// Stable name for logs.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ping => "ping",
            Self::HostInfo => "host_info",
        }
    }
}

/// A response frame. `id` echoes the request id (empty when the request was too
/// broken to carry one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivResponse {
    pub id: String,
    pub result: PrivResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PrivResult {
    #[serde(rename = "ok")]
    Success { reply: PrivReply },
    #[serde(rename = "error")]
    Failure { error: PrivError },
}

impl PrivResponse {
    pub fn success(id: impl Into<String>, reply: PrivReply) -> Self {
        Self {
            id: id.into(),
            result: PrivResult::Success { reply },
        }
    }

    pub fn failure(id: impl Into<String>, code: PrivErrorCode, message: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            result: PrivResult::Failure {
                error: PrivError {
                    code,
                    message: message.into(),
                },
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum PrivReply {
    Pong {
        /// Version of the `priv` process, so `status` can spot a stale helper.
        version: String,
    },
    HostInfo(HostInfo),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivError {
    pub code: PrivErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivErrorCode {
    /// The frame was not valid JSON or not a known operation.
    BadRequest,
    /// The peer's user id is not allowed to talk to `priv`.
    Forbidden,
    /// Known operation that this platform cannot perform.
    Unsupported,
    Internal,
}

/// Result of [`PrivOp::HostInfo`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfo {
    pub hostname: String,
    /// Pretty OS name, from `PRETTY_NAME` in `/etc/os-release`.
    pub os: String,
    pub kernel: String,
    pub uptime_secs: u64,
    pub cpu_count: u32,
    pub mem_total_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_ops_have_a_simple_wire_form() {
        let req = PrivRequest {
            id: "01H".into(),
            actor: None,
            op: PrivOp::Ping,
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"id": "01H", "actor": null, "op": {"type": "ping"}})
        );
    }

    #[test]
    fn unknown_operations_do_not_parse() {
        for bad in [
            r#"{"id":"1","op":{"type":"run_shell","args":{"cmd":"id"}}}"#,
            r#"{"id":"1","op":{"type":"Ping"}}"#,
            r#"{"id":"1","op":"ping"}"#,
            r#"{"id":"1","op":{"type":"ping"},"extra":true}"#,
        ] {
            assert!(serde_json::from_str::<PrivRequest>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn response_round_trips() {
        let ok = PrivResponse::success(
            "a",
            PrivReply::Pong {
                version: "1".into(),
            },
        );
        let err = PrivResponse::failure("b", PrivErrorCode::Forbidden, "no");
        for r in [ok, err] {
            let s = serde_json::to_string(&r).unwrap();
            assert_eq!(serde_json::from_str::<PrivResponse>(&s).unwrap(), r);
        }
    }
}
