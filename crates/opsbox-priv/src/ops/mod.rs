//! One module per privileged operation. Adding an operation means a new
//! `PrivOp` variant in `opsbox-proto`, a module here, and one line in [`dispatch`];
//! the compiler enforces the last two because the `match` is exhaustive.

mod host_info;
mod ping;

use opsbox_proto::{PrivError, PrivOp, PrivReply};

/// Blocking; the server runs it on a blocking thread.
pub(crate) fn dispatch(op: &PrivOp) -> Result<PrivReply, PrivError> {
    match op {
        PrivOp::Ping => Ok(ping::run()),
        PrivOp::HostInfo => host_info::run().map(PrivReply::HostInfo),
    }
}
