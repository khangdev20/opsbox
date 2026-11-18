//! Bounded token buckets. IP limits run before parsing bodies or hashing passwords;
//! the session/token limit runs only after successful authentication.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use opsbox_proto::ErrorCode;

use crate::HttpError;

const MAX_BUCKETS: usize = 4096;
const IDLE: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Copy)]
struct Policy {
    burst: f64,
    per_second: f64,
}

const IP: Policy = Policy {
    burst: 120.0,
    per_second: 2.0,
};
const AUTH: Policy = Policy {
    burst: 20.0,
    per_second: 1.0 / 3.0,
};
const SESSION: Policy = Policy {
    burst: 120.0,
    per_second: 4.0,
};
/// Reveal endpoints: a handful in a burst, then about one every 30 seconds.
const REVEAL: Policy = Policy {
    burst: 10.0,
    per_second: 1.0 / 30.0,
};

struct Bucket {
    tokens: f64,
    last: Instant,
}

#[derive(Default)]
struct Buckets(HashMap<String, Bucket>);

impl Buckets {
    fn take(&mut self, key: &str, policy: Policy, now: Instant) -> Result<(), u64> {
        if !self.0.contains_key(key) && self.0.len() >= MAX_BUCKETS {
            self.0.retain(|_, b| now.duration_since(b.last) < IDLE);
            // Never evict an active bucket: rotating identifiers must not reset a limit.
            if self.0.len() >= MAX_BUCKETS {
                return Err(60);
            }
        }
        let b = self.0.entry(key.to_owned()).or_insert(Bucket {
            tokens: policy.burst,
            last: now,
        });
        b.tokens = (b.tokens + now.duration_since(b.last).as_secs_f64() * policy.per_second)
            .min(policy.burst);
        b.last = now;
        if b.tokens < 1.0 {
            Err(((1.0 - b.tokens) / policy.per_second).ceil().max(1.0) as u64)
        } else {
            b.tokens -= 1.0;
            Ok(())
        }
    }
}

#[derive(Default)]
struct Inner {
    ip: Buckets,
    auth: Buckets,
    session: Buckets,
    reveal: Buckets,
}

#[derive(Clone, Default)]
pub(crate) struct RateLimits(Arc<Mutex<Inner>>);

fn limited(secs: u64) -> HttpError {
    let mut e = HttpError::new(
        ErrorCode::RateLimited,
        "Too many requests. Wait a moment and try again.",
    );
    e.1 = Some(secs);
    e
}

impl RateLimits {
    pub(crate) fn session(&self, id: &str) -> Result<(), HttpError> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .session
            .take(id, SESSION, Instant::now())
            .map_err(limited)
    }

    /// Per-session budget for secret reveal endpoints.
    pub(crate) fn reveal(&self, session_id: &str) -> Result<(), HttpError> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reveal
            .take(session_id, REVEAL, Instant::now())
            .map_err(limited)
    }
}

pub(crate) async fn middleware(
    State(limits): State<RateLimits>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    if !path.starts_with("/api/") {
        return next.run(req).await;
    }
    let auth = path == "/api/v1/auth/login"
        || path.starts_with("/api/v1/setup/")
        || path.starts_with("/api/v1/invites/")
        || path.starts_with("/api/v1/auth/totp/");
    let (parts, body) = req.into_parts();
    let ip = crate::session::client_ip(&parts);
    let result = {
        let mut inner = limits.0.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        inner.ip.take(&ip, IP, now).and_then(|()| {
            if auth {
                inner.auth.take(&ip, AUTH, now)
            } else {
                Ok(())
            }
        })
    };
    if let Err(secs) = result {
        return limited(secs).into_response();
    }
    next.run(Request::from_parts(parts, body)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_refill_and_retry_after_use_elapsed_time() {
        let mut b = Buckets::default();
        let now = Instant::now();
        for _ in 0..20 {
            assert_eq!(b.take("ip", AUTH, now), Ok(()));
        }
        assert_eq!(b.take("ip", AUTH, now), Err(3));
        assert_eq!(b.take("another", AUTH, now), Ok(()));
        assert_eq!(b.take("ip", AUTH, now + Duration::from_secs(3)), Ok(()));
        assert_eq!(b.take("ip", AUTH, now + Duration::from_secs(3)), Err(3));
    }

    #[test]
    fn rotating_keys_cannot_evict_live_limits_or_grow_memory() {
        let mut b = Buckets::default();
        let now = Instant::now();
        for i in 0..MAX_BUCKETS {
            assert!(b.take(&i.to_string(), IP, now).is_ok());
        }
        assert_eq!(b.take("new", IP, now), Err(60));
        assert_eq!(b.0.len(), MAX_BUCKETS);
        assert_eq!(b.take("new", IP, now + IDLE), Ok(()));
        assert_eq!(b.0.len(), 1);
    }
}
