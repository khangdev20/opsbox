//! The error format of docs/BACKEND_SPEC.md section 6.1.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use opsbox_priv::ClientError;
use opsbox_proto::{ApiError, ErrorCode, PrivErrorCode};
use tracing::error;

/// An error ready to be sent: status code plus the JSON body.
#[derive(Debug)]
pub struct HttpError(pub ApiError);

impl HttpError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self(ApiError::new(code, message))
    }

    pub fn not_found() -> Self {
        Self::new(ErrorCode::NotFound, "There is nothing at this address.")
    }

    pub fn internal() -> Self {
        Self::new(
            ErrorCode::Internal,
            "Something went wrong on the server. Check the opsbox log for details.",
        )
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.0.error.code.http_status())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(self.0)).into_response()
    }
}

/// Failures talking to `priv`. The technical detail goes to the log (inside the
/// request's span, so it carries the request id); the caller gets a sentence a
/// person can act on.
impl From<ClientError> for HttpError {
    fn from(e: ClientError) -> Self {
        error!(error = %e, "privileged helper call failed");
        match e {
            ClientError::Connect { .. } | ClientError::Timeout => Self::new(
                ErrorCode::Unavailable,
                "The privileged helper is not running. Start opsbox-priv and try again.",
            ),
            ClientError::Remote(remote) if remote.code == PrivErrorCode::Forbidden => Self::new(
                ErrorCode::Unavailable,
                "The privileged helper does not accept this service. Check the allowed users in its configuration.",
            ),
            ClientError::Remote(_) | ClientError::Io(_) | ClientError::Protocol(_) => {
                Self::internal()
            }
        }
    }
}

/// Handler for a known path used with the wrong method.
pub(crate) async fn method_not_allowed() -> HttpError {
    HttpError::new(
        ErrorCode::MethodNotAllowed,
        "This address does not accept that kind of request.",
    )
}
