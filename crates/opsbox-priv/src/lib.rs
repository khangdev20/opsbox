//! The privileged helper.
//!
//! `opsbox priv` runs as root, listens on a Unix socket and executes a closed set of
//! typed operations ([`opsbox_proto::PrivOp`]) for `serve`. [`PrivClient`] is what
//! `serve` uses to call it.
//!
//! Trust model: file permissions on the socket are the first gate, the kernel's
//! peer credentials (`SO_PEERCRED` on Linux) are the second. The server checks the
//! connecting process's user id against an allow-list *before reading a byte*.

mod client;
mod config;
mod io;
mod ops;
mod server;

pub use client::{ClientError, PrivClient};
pub use config::{DEFAULT_SOCKET_PATH, PrivConfig, resolve_users};
pub use server::{PrivServer, ServerError};

/// Version reported by `Ping`, so `status` can notice a stale helper after an update.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
