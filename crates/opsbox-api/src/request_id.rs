//! Request ids and per-request logging.
//!
//! Every request gets a fresh ULID (an inbound `X-Request-Id` is ignored: it is
//! untrusted input). The id is returned in the `X-Request-Id` response header, put
//! in a tracing span so every log line of the request carries it, and passed to
//! `priv` so one request can be followed across both processes.

use std::time::Instant;

use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;
use tracing::{Instrument, debug, info, info_span};

pub const HEADER: &str = "x-request-id";

#[derive(Debug, Clone)]
pub struct RequestId(pub String);

pub(crate) async fn middleware(mut req: Request, next: Next) -> Response {
    let id = ulid::Ulid::generate().to_string();
    req.extensions_mut().insert(RequestId(id.clone()));

    // Path only: query strings may carry secrets in later phases.
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let span = info_span!("request", request_id = %id, %method, %path);
    let started = Instant::now();

    let mut response = next.run(req).instrument(span.clone()).await;

    if let Ok(value) = HeaderValue::from_str(&id) {
        response.headers_mut().insert(HEADER, value);
    }
    if path.starts_with("/api/")
        || path == "/readyz"
        || path == "/health/ready"
        || path == "/healthz"
    {
        response
            .headers_mut()
            .insert("cache-control", HeaderValue::from_static("no-store"));
    }
    let status = response.status().as_u16();
    let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    span.in_scope(|| {
        // API calls and failures are worth an info line; health probes and the many
        // static asset requests are not.
        if path.starts_with("/api/") || status >= 400 {
            info!(status, latency_ms, "request");
        } else {
            debug!(status, latency_ms, "request");
        }
    });
    response
}
