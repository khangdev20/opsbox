//! Control socket protocol between `serve`/CLI and `opsbox proxy`.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use opsbox_proto::frame::{self, FrameError};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

pub const DEFAULT_SOCKET_PATH: &str = "/run/opsbox-proxy/proxy.sock";
pub const DEFAULT_STATE_DIR: &str = "/var/lib/opsbox/proxy";
pub const DEFAULT_HTTP_ADDR: &str = "0.0.0.0:80";
pub const DEFAULT_HTTPS_ADDR: &str = "0.0.0.0:443";

const CALL_TIMEOUT: Duration = Duration::from_secs(10);

fn default_true() -> bool {
    true
}

/// How `opsbox proxy` connects to an upstream route target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum UpstreamTls {
    Plain,
    /// Connect using TLS; when `cert_sha256` is present (lowercase hex), the upstream leaf
    /// certificate must match that SHA-256 fingerprint.
    Https {
        cert_sha256: Option<String>,
    },
}

/// One reverse-proxy virtual host route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub host: String,
    pub upstream: String,
    pub upstream_tls: UpstreamTls,
    #[serde(default)]
    pub redirect_www: bool,
    /// Optional canonical host to permanently redirect requests for `host` to.
    #[serde(default)]
    pub redirect_to: Option<String>,
    #[serde(default = "default_true")]
    pub redirect_https: bool,
    #[serde(default = "default_true")]
    pub hsts: bool,
    /// Maximum request body size in bytes; 0 uses the proxy default.
    #[serde(default)]
    pub max_body_bytes: u64,
    #[serde(default)]
    pub is_panel: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CertState {
    None,
    Pending,
    Valid,
    Expiring,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertInfo {
    pub domain: String,
    pub status: CertState,
    pub not_before: Option<i64>,
    pub not_after: Option<i64>,
    pub days_remaining: Option<i64>,
    pub issuer: Option<String>,
    pub staging: bool,
    pub last_error: Option<String>,
    pub issued_at: Option<i64>,
    #[serde(default)]
    pub consecutive_failures: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyStatus {
    pub version: String,
    pub routes_count: usize,
    pub certs: Vec<CertInfo>,
    pub acme_staging: bool,
    #[serde(default)]
    pub network: Box<opsbox_proto::ProxyNetwork>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyRequest {
    pub id: String,
    pub op: ProxyOp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "args", rename_all = "snake_case")]
pub enum ProxyOp {
    Ping,
    Status,
    Listeners {
        action: opsbox_proto::ListenerAction,
    },
    SetRoutes {
        routes: Vec<Route>,
    },
    IssueCert {
        domain: String,
    },
    RemoveCert {
        domain: String,
    },
}

impl ProxyOp {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ping => "ping",
            Self::Status => "status",
            Self::Listeners { .. } => "listeners",
            Self::SetRoutes { .. } => "set_routes",
            Self::IssueCert { .. } => "issue_cert",
            Self::RemoveCert { .. } => "remove_cert",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyResponse {
    pub id: String,
    pub result: ProxyResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProxyResult {
    #[serde(rename = "ok")]
    Success { reply: ProxyReply },
    #[serde(rename = "error")]
    Failure { error: ProxyError },
}

impl ProxyResponse {
    pub fn success(id: impl Into<String>, reply: ProxyReply) -> Self {
        Self {
            id: id.into(),
            result: ProxyResult::Success { reply },
        }
    }

    pub fn failure(
        id: impl Into<String>,
        code: ProxyErrorCode,
        message: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            result: ProxyResult::Failure {
                error: ProxyError {
                    code,
                    message: message.into(),
                },
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ProxyReply {
    Pong { version: String },
    Status(ProxyStatus),
    Ok,
    Cert(CertInfo),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyError {
    pub code: ProxyErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyErrorCode {
    BadRequest,
    Forbidden,
    Invalid,
    NotFound,
    Internal,
    Unavailable,
}

// ------------------------------------------------------------------ framing I/O

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReadError {
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub(crate) async fn write_frame<T: Serialize>(
    stream: &mut UnixStream,
    value: &T,
) -> Result<(), ReadError> {
    let bytes = frame::encode(value)?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

pub(crate) async fn read_body(stream: &mut UnixStream) -> Result<Option<Vec<u8>>, ReadError> {
    let mut header = [0u8; 4];
    match stream.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(ReadError::Io(e)),
    }
    let len = frame::body_len(header)?;
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    Ok(Some(body))
}

// ------------------------------------------------------------------ client

#[derive(Debug, thiserror::Error)]
pub enum ProxyClientError {
    #[error("cannot reach opsbox-proxy at {path}: {source}")]
    Connect { path: String, source: io::Error },
    #[error("opsbox-proxy did not answer in time")]
    Timeout,
    #[error("communication with opsbox-proxy failed: {0}")]
    Io(String),
    #[error("opsbox-proxy sent an unexpected reply: {0}")]
    Protocol(String),
    #[error("{}", .0.message)]
    Remote(ProxyError),
}

impl From<ReadError> for ProxyClientError {
    fn from(e: ReadError) -> Self {
        match e {
            ReadError::Frame(fe) => Self::Protocol(fe.to_string()),
            ReadError::Io(ie) => Self::Io(ie.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProxyClient {
    socket_path: Arc<PathBuf>,
    timeout: Duration,
}

impl ProxyClient {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: Arc::new(socket_path.into()),
            timeout: CALL_TIMEOUT,
        }
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    pub async fn call(&self, rid: &str, op: ProxyOp) -> Result<ProxyReply, ProxyClientError> {
        tokio::time::timeout(self.timeout, self.call_inner(rid, op))
            .await
            .map_err(|_| ProxyClientError::Timeout)?
    }

    async fn call_inner(&self, rid: &str, op: ProxyOp) -> Result<ProxyReply, ProxyClientError> {
        let mut stream = UnixStream::connect(&*self.socket_path)
            .await
            .map_err(|source| ProxyClientError::Connect {
                path: self.socket_path.display().to_string(),
                source,
            })?;
        let req = ProxyRequest {
            id: rid.to_owned(),
            op,
        };
        write_frame(&mut stream, &req).await?;
        let body = read_body(&mut stream).await?.ok_or_else(|| {
            ProxyClientError::Protocol("connection closed without an answer".into())
        })?;
        let resp: ProxyResponse =
            frame::decode(&body).map_err(|e| ProxyClientError::Protocol(e.to_string()))?;
        match resp.result {
            ProxyResult::Failure { error } => Err(ProxyClientError::Remote(error)),
            ProxyResult::Success { reply } if resp.id == rid => Ok(reply),
            ProxyResult::Success { .. } => Err(ProxyClientError::Protocol(format!(
                "answer is for request {:?}, not {rid:?}",
                resp.id
            ))),
        }
    }

    pub async fn ping(&self, rid: &str) -> Result<String, ProxyClientError> {
        match self.call(rid, ProxyOp::Ping).await? {
            ProxyReply::Pong { version } => Ok(version),
            other => Err(ProxyClientError::Protocol(format!(
                "unexpected reply {other:?}"
            ))),
        }
    }

    pub async fn status(&self, rid: &str) -> Result<ProxyStatus, ProxyClientError> {
        match self.call(rid, ProxyOp::Status).await? {
            ProxyReply::Status(s) => Ok(s),
            other => Err(ProxyClientError::Protocol(format!(
                "unexpected reply {other:?}"
            ))),
        }
    }

    pub async fn set_routes(&self, rid: &str, routes: Vec<Route>) -> Result<(), ProxyClientError> {
        match self.call(rid, ProxyOp::SetRoutes { routes }).await? {
            ProxyReply::Ok => Ok(()),
            other => Err(ProxyClientError::Protocol(format!(
                "unexpected reply {other:?}"
            ))),
        }
    }

    pub async fn issue_cert(&self, rid: &str, domain: &str) -> Result<CertInfo, ProxyClientError> {
        match self
            .call(
                rid,
                ProxyOp::IssueCert {
                    domain: domain.to_owned(),
                },
            )
            .await?
        {
            ProxyReply::Cert(info) => Ok(info),
            other => Err(ProxyClientError::Protocol(format!(
                "unexpected reply {other:?}"
            ))),
        }
    }

    pub async fn remove_cert(&self, rid: &str, domain: &str) -> Result<(), ProxyClientError> {
        match self
            .call(
                rid,
                ProxyOp::RemoveCert {
                    domain: domain.to_owned(),
                },
            )
            .await?
        {
            ProxyReply::Ok => Ok(()),
            other => Err(ProxyClientError::Protocol(format!(
                "unexpected reply {other:?}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_and_response_round_trip() {
        let req = ProxyRequest {
            id: "r1".into(),
            op: ProxyOp::SetRoutes {
                routes: vec![Route {
                    host: "panel.example.com".into(),
                    upstream: "127.0.0.1:8443".into(),
                    upstream_tls: UpstreamTls::Https {
                        cert_sha256: Some("abcd".into()),
                    },
                    redirect_www: false,
                    redirect_to: None,
                    redirect_https: true,
                    hsts: true,
                    max_body_bytes: 0,
                    is_panel: true,
                }],
            },
        };
        let encoded = frame::encode(&req).unwrap();
        let decoded: ProxyRequest = frame::decode(&encoded[4..]).unwrap();
        assert_eq!(decoded, req);

        assert!(
            serde_json::from_str::<ProxyRequest>(r#"{"id":"1","op":{"type":"ping"},"extra":1}"#)
                .is_err()
        );
    }
}
