use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use opsbox_proto::frame;
use opsbox_proto::{
    HostInfo, PrivError, PrivOp, PrivReply, PrivRequest, PrivResponse, PrivResult, Secret,
};
use tokio::net::UnixStream;

use crate::io::{ReadError, WriteError, read_body, write_frame};

/// How long a single call may take, connect included.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Could not connect. [`ClientError::connect_kind`] says why (not running,
    /// permission denied, ...).
    #[error("cannot reach the privileged helper at {path}: {source}")]
    Connect { path: String, source: io::Error },
    #[error("the privileged helper did not answer in time")]
    Timeout,
    #[error("communication with the privileged helper failed: {0}")]
    Io(String),
    #[error("the privileged helper sent something unexpected: {0}")]
    Protocol(String),
    /// `priv` understood the request and refused or failed it.
    #[error("the privileged helper reported an error: {}", .0.message)]
    Remote(PrivError),
}

impl ClientError {
    /// For `Connect` errors, the underlying I/O error kind.
    pub fn connect_kind(&self) -> Option<io::ErrorKind> {
        match self {
            Self::Connect { source, .. } => Some(source.kind()),
            _ => None,
        }
    }
}

impl From<ReadError> for ClientError {
    fn from(e: ReadError) -> Self {
        match e {
            ReadError::Frame(e) => Self::Protocol(e.to_string()),
            ReadError::Io(e) => Self::Io(e.to_string()),
        }
    }
}

impl From<WriteError> for ClientError {
    fn from(e: WriteError) -> Self {
        match e {
            WriteError::Frame(e) => Self::Protocol(e.to_string()),
            WriteError::Io(e) => Self::Io(e.to_string()),
        }
    }
}

/// Calls `priv` over its Unix socket. Cheap to clone. Each call opens its own
/// connection, so a restarted `priv` is picked up without any reconnect logic.
#[derive(Debug, Clone)]
pub struct PrivClient {
    socket_path: Arc<PathBuf>,
    timeout: Duration,
}

impl PrivClient {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: Arc::new(socket_path.into()),
            timeout: CALL_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    /// Perform `op`. `request_id` appears in the logs of both processes; `actor`
    /// is who it is done for.
    pub async fn call(
        &self,
        request_id: &str,
        actor: Option<&str>,
        op: PrivOp,
    ) -> Result<PrivReply, ClientError> {
        tokio::time::timeout(self.timeout, self.call_inner(request_id, actor, op))
            .await
            .map_err(|_| ClientError::Timeout)?
    }

    async fn call_inner(
        &self,
        request_id: &str,
        actor: Option<&str>,
        op: PrivOp,
    ) -> Result<PrivReply, ClientError> {
        let mut stream = UnixStream::connect(&*self.socket_path)
            .await
            .map_err(|source| ClientError::Connect {
                path: self.socket_path.display().to_string(),
                source,
            })?;

        let request = PrivRequest {
            id: request_id.to_owned(),
            actor: actor.map(str::to_owned),
            op,
        };
        write_frame(&mut stream, &request).await?;

        let body = read_body(&mut stream)
            .await?
            .ok_or_else(|| ClientError::Protocol("connection closed without an answer".into()))?;
        let response: PrivResponse =
            frame::decode(&body).map_err(|e| ClientError::Protocol(e.to_string()))?;

        match response.result {
            PrivResult::Failure { error } => Err(ClientError::Remote(error)),
            PrivResult::Success { reply } if response.id == request_id => Ok(reply),
            PrivResult::Success { .. } => Err(ClientError::Protocol(format!(
                "answer is for request {:?}, not {request_id:?}",
                response.id
            ))),
        }
    }

    /// Liveness check. Returns the helper's version.
    pub async fn ping(&self, request_id: &str) -> Result<String, ClientError> {
        match self.call(request_id, None, PrivOp::Ping).await? {
            PrivReply::Pong { version } => Ok(version),
            other => Err(ClientError::Protocol(format!("unexpected reply {other:?}"))),
        }
    }

    pub async fn host_info(
        &self,
        request_id: &str,
        actor: Option<&str>,
    ) -> Result<HostInfo, ClientError> {
        match self.call(request_id, actor, PrivOp::HostInfo).await? {
            PrivReply::HostInfo(info) => Ok(info),
            other => Err(ClientError::Protocol(format!("unexpected reply {other:?}"))),
        }
    }

    /// Create a TOTP secret bound to `context` (e.g. `totp:<user id>`). Returns the
    /// sealed value to store and the base32 secret to show once.
    pub async fn totp_new_secret(
        &self,
        request_id: &str,
        context: &str,
    ) -> Result<(String, Secret), ClientError> {
        let op = PrivOp::TotpNewSecret {
            context: context.to_owned(),
        };
        match self.call(request_id, None, op).await? {
            PrivReply::TotpSecret {
                sealed,
                secret_base32,
            } => Ok((sealed, secret_base32)),
            other => Err(ClientError::Protocol(format!("unexpected reply {other:?}"))),
        }
    }

    /// The time step that `code` matches (newer than `after_step`), if any.
    pub async fn totp_verify(
        &self,
        request_id: &str,
        sealed: &str,
        context: &str,
        code: &str,
        after_step: u64,
    ) -> Result<Option<u64>, ClientError> {
        let op = PrivOp::TotpVerify {
            sealed: sealed.to_owned(),
            context: context.to_owned(),
            code: Secret::new(code),
            after_step,
        };
        match self.call(request_id, None, op).await? {
            PrivReply::TotpResult { matched_step } => Ok(matched_step),
            other => Err(ClientError::Protocol(format!("unexpected reply {other:?}"))),
        }
    }
}
