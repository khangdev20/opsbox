//! Consistent limit errors and response headers, including early middleware errors.

use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use opsbox_proto::ErrorCode;

use crate::HttpError;

pub(crate) async fn middleware(req: Request, next: Next) -> Response {
    let api = req.uri().path().starts_with("/api/");
    let mut response = next.run(req).await;
    if api {
        let code = match response.status() {
            StatusCode::PAYLOAD_TOO_LARGE => Some((
                ErrorCode::TooLarge,
                "That request is larger than this endpoint accepts.",
            )),
            StatusCode::REQUEST_TIMEOUT => Some((
                ErrorCode::Timeout,
                "This request took too long. Check its result before trying again.",
            )),
            _ => None,
        };
        if let Some((code, message)) = code {
            response = HttpError::new(code, message).into_response();
        }
        // Login/enrolment/setup/token responses must never be cached, including by
        // an intermediary configured in a later phase.
        response
            .headers_mut()
            .insert("cache-control", HeaderValue::from_static("no-store"));
    }
    let h = response.headers_mut();
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("referrer-policy", HeaderValue::from_static("same-origin"));
    h.insert(
        "content-security-policy",
        HeaderValue::from_static("frame-ancestors 'none'; base-uri 'self'; object-src 'none'"),
    );
    response
}
