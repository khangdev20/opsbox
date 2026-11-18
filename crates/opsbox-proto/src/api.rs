//! HTTP API shapes shared with clients (docs/BACKEND_SPEC.md sections 6.1 and 6.4).

use serde::{Deserialize, Serialize};

use crate::privilege::HostInfo;

/// Machine-readable error category. The wire form is snake_case, e.g. `"validation"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Validation,
    Unauthorized,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    Conflict,
    /// The change could lock the caller out; repeat it with `confirm: true` to go ahead.
    ConfirmationRequired,
    /// This machine cannot do that (no supported package manager, no systemd...).
    Unsupported,
    RateLimited,
    /// The request body is larger than this endpoint accepts.
    TooLarge,
    /// The request exceeded the endpoint's time budget.
    Timeout,
    /// A dependency (for example the privileged helper) is not reachable.
    Unavailable,
    Internal,
}

impl ErrorCode {
    /// The HTTP status code this category maps to.
    pub fn http_status(self) -> u16 {
        match self {
            Self::Validation => 400,
            Self::Unauthorized => 401,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::MethodNotAllowed => 405,
            Self::Conflict | Self::ConfirmationRequired => 409,
            Self::Unsupported => 501,
            Self::RateLimited => 429,
            Self::TooLarge => 413,
            Self::Timeout => 408,
            Self::Unavailable => 503,
            Self::Internal => 500,
        }
    }
}

/// The body of every error response: `{ "error": { "code", "message", "field" } }`.
///
/// `message` is a human sentence that the UI shows verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub error: ApiErrorBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiErrorBody {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<serde_json::Value>,
}

impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            error: ApiErrorBody {
                code,
                message: message.into(),
                field: None,
                capacity: None,
            },
        }
    }

    /// Attach the name of the offending request field.
    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.error.field = Some(field.into());
        self
    }
}

/// Response of `GET /api/v1/host`.
///
/// The field names and units match what `web/js/api.js` returns in mock mode, so
/// the page modules do not change: `memTotal` is in GB and `upDays` is whole days.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostView {
    pub name: String,
    pub os: String,
    pub kernel: String,
    pub cores: u32,
    pub mem_total: f64,
    pub up_days: u64,
    pub uptime_secs: u64,
    pub version: String,
}

impl HostView {
    pub fn from_info(info: HostInfo, version: &str) -> Self {
        const GIB: f64 = (1u64 << 30) as f64;
        Self {
            name: info.hostname,
            os: info.os,
            kernel: info.kernel,
            cores: info.cpu_count,
            mem_total: (info.mem_total_bytes as f64 / GIB * 100.0).round() / 100.0,
            up_days: info.uptime_secs / 86_400,
            uptime_secs: info.uptime_secs,
            version: version.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_body_matches_spec() {
        let e = ApiError::new(ErrorCode::Validation, "Enter a domain.").with_field("domain");
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"error": {"code": "validation", "message": "Enter a domain.", "field": "domain"}})
        );
    }

    #[test]
    fn field_is_omitted_when_absent() {
        let e = ApiError::new(ErrorCode::NotFound, "No such thing.");
        let v = serde_json::to_value(&e).unwrap();
        assert!(v["error"].get("field").is_none());
    }

    #[test]
    fn host_view_uses_ui_units() {
        let view = HostView::from_info(
            HostInfo {
                hostname: "box".into(),
                os: "Ubuntu 24.04 LTS".into(),
                kernel: "6.8.0-48".into(),
                uptime_secs: 3 * 86_400 + 500,
                cpu_count: 8,
                mem_total_bytes: 8 << 30,
            },
            "0.1.0",
        );
        assert_eq!(view.up_days, 3);
        assert_eq!(view.mem_total, 8.0);
        let v = serde_json::to_value(&view).unwrap();
        assert_eq!(v["upDays"], 3);
        assert_eq!(v["memTotal"], 8.0);
        assert_eq!(v["name"], "box");
    }
}
