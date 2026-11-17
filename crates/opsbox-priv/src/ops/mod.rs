//! One module per privileged operation. Adding an operation means a new
//! `PrivOp` variant in `opsbox-proto`, a module here, and one line in [`dispatch`];
//! the compiler enforces the last two because the `match` is exhaustive.

mod facts;
mod host_info;
mod metrics;
mod ping;
mod totp;

pub use totp::{base32_encode, code_at, verify_at};

use opsbox_proto::{PrivError, PrivOp, PrivReply};

use crate::secrets::Secrets;

/// Blocking; the server runs it on a blocking thread.
pub(crate) fn dispatch(op: &PrivOp, secrets: &Secrets) -> Result<PrivReply, PrivError> {
    match op {
        PrivOp::Ping => Ok(ping::run()),
        PrivOp::HostInfo => host_info::run().map(PrivReply::HostInfo),
        PrivOp::MetricsSample { procs } => metrics::run(*procs),
        PrivOp::SystemFacts => Ok(PrivReply::SystemFacts(facts::collect())),
        PrivOp::TotpNewSecret { context } => totp::new_secret(secrets, context),
        PrivOp::TotpVerify {
            sealed,
            context,
            code,
            after_step,
        } => totp::verify(secrets, sealed, context, code.expose(), *after_step),
    }
}
