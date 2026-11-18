//! The error format of docs/BACKEND_SPEC.md section 6.1.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use opsbox_auth::AuthError;
use opsbox_priv::ClientError;
use opsbox_proto::{ApiError, ErrorCode, PrivErrorCode};
use tracing::{error, warn};

pub(crate) const HELPER_DOWN: &str =
    "OpsBox's helper is not running. Restart it with `sudo systemctl restart opsbox-priv`.";
pub(crate) const HELPER_TIMEOUT: &str = "The helper did not answer in time. The operation may still be running; check Containers or System > Jobs before trying again.";
pub(crate) const HELPER_FORBIDDEN: &str = "The privileged helper does not accept this service. Check the allowed users in its configuration.";

/// An error ready to be sent: status code plus the JSON body.
#[derive(Debug)]
pub struct HttpError(pub ApiError, pub Option<u64>);

impl HttpError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self(ApiError::new(code, message), None)
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
        let mut response = (status, Json(self.0)).into_response();
        if let Some(secs) = self.1 {
            response.headers_mut().insert("retry-after", secs.into());
        }
        response
    }
}

/// Failures talking to `priv`. `PrivClient::call` already logged the real error with the
/// request id, the op name and the elapsed time; the caller gets a sentence a person can
/// act on, and each kind of failure says something different: not running (restart it),
/// too slow (it may still be working, do not start it again) or refused (configuration).
impl From<ClientError> for HttpError {
    fn from(e: ClientError) -> Self {
        match e {
            ClientError::Connect { .. }
                if e.connect_kind() == Some(std::io::ErrorKind::PermissionDenied) =>
            {
                warn!(error = %e, "privileged helper socket refused this service's user");
                Self::new(ErrorCode::Unavailable, HELPER_FORBIDDEN)
            }
            ClientError::Connect { .. } => Self::new(ErrorCode::Unavailable, HELPER_DOWN),
            ClientError::Timeout => Self::new(ErrorCode::Unavailable, HELPER_TIMEOUT),
            ClientError::Remote(remote) if remote.code == PrivErrorCode::Forbidden => {
                // Which service user was refused is in the helper's log (uid and pid); the
                // person only needs to know where to look.
                warn!(
                    reason = %remote.message,
                    "privileged helper refused this service's user; see the opsbox-priv log for its uid"
                );
                Self::new(ErrorCode::Unavailable, HELPER_FORBIDDEN)
            }
            ClientError::Remote(remote) => {
                // `priv` words its refusals and failures for people, so they pass through.
                let code = match remote.code {
                    PrivErrorCode::Invalid | PrivErrorCode::BadRequest => ErrorCode::Validation,
                    PrivErrorCode::Refused | PrivErrorCode::Forbidden => ErrorCode::Forbidden,
                    PrivErrorCode::ConfirmRequired => ErrorCode::ConfirmationRequired,
                    PrivErrorCode::NotFound => ErrorCode::NotFound,
                    PrivErrorCode::Conflict => ErrorCode::Conflict,
                    PrivErrorCode::Unsupported => ErrorCode::Unsupported,
                    PrivErrorCode::Internal => ErrorCode::Internal,
                };
                Self::new(code, remote.message)
            }
            ClientError::Io(_) | ClientError::Protocol(_) => {
                error!(error = %e, "privileged helper answered badly");
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

impl From<AuthError> for HttpError {
    fn from(e: AuthError) -> Self {
        match e {
            AuthError::InvalidCredentials => Self::new(ErrorCode::Unauthorized, e.to_string()),
            AuthError::Locked(secs) => {
                let mut err = Self::new(ErrorCode::RateLimited, e.to_string());
                err.1 = Some(secs);
                err
            }
            AuthError::Invalid { field, message } => Self(
                ApiError::new(ErrorCode::Validation, message).with_field(field),
                None,
            ),
            AuthError::NotFound => Self::not_found(),
            AuthError::HelperUnavailable => Self::new(ErrorCode::Unavailable, e.to_string()),
            AuthError::Internal(detail) => {
                error!(detail, "internal auth error");
                Self::internal()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opsbox_proto::PrivError;
    use std::io;

    fn connect(kind: io::ErrorKind) -> ClientError {
        ClientError::Connect {
            path: "/run/opsbox/priv.sock".into(),
            source: io::Error::from(kind),
        }
    }

    fn message(e: ClientError) -> (ErrorCode, String) {
        let HttpError(api, _) = e.into();
        (api.error.code, api.error.message)
    }

    #[test]
    fn a_missing_or_refusing_socket_says_the_helper_is_not_running() {
        for kind in [io::ErrorKind::NotFound, io::ErrorKind::ConnectionRefused] {
            let (code, text) = message(connect(kind));
            assert_eq!(code, ErrorCode::Unavailable);
            assert_eq!(text, HELPER_DOWN);
            assert!(text.contains("sudo systemctl restart opsbox-priv"));
        }
    }

    #[test]
    fn a_timeout_does_not_claim_the_helper_is_down_or_tell_people_to_start_it() {
        let (code, text) = message(ClientError::Timeout);
        assert_eq!(code, ErrorCode::Unavailable);
        assert_eq!(text, HELPER_TIMEOUT);
        assert!(text.contains("may still be running"));
        assert!(!text.contains("not running") && !text.contains("systemctl"));
    }

    #[test]
    fn a_refused_service_user_has_its_own_message() {
        let remote = ClientError::Remote(PrivError {
            code: PrivErrorCode::Forbidden,
            message: "This user may not use the privileged helper.".into(),
        });
        let (_, text) = message(remote);
        assert_eq!(text, HELPER_FORBIDDEN);
        let (_, denied) = message(connect(io::ErrorKind::PermissionDenied));
        assert_eq!(denied, HELPER_FORBIDDEN);
    }

    #[test]
    fn the_three_messages_differ() {
        assert_ne!(HELPER_DOWN, HELPER_TIMEOUT);
        assert_ne!(HELPER_DOWN, HELPER_FORBIDDEN);
        assert_ne!(HELPER_TIMEOUT, HELPER_FORBIDDEN);
    }

    #[test]
    fn a_remote_refusal_passes_its_own_sentence_through() {
        let (code, text) = message(ClientError::Remote(PrivError {
            code: PrivErrorCode::Conflict,
            message: "A database named pg-main already exists.".into(),
        }));
        assert_eq!(code, ErrorCode::Conflict);
        assert_eq!(text, "A database named pg-main already exists.");
    }
}
