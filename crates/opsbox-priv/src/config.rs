use std::path::PathBuf;

use nix::unistd::User;

pub const DEFAULT_SOCKET_PATH: &str = "/run/opsbox/priv.sock";

/// Everything the server needs, already resolved (no user names left).
#[derive(Debug, Clone)]
pub struct PrivConfig {
    pub socket_path: PathBuf,
    /// Only processes running as one of these user ids may talk to `priv`.
    pub allowed_uids: Vec<u32>,
    /// File mode of the socket, e.g. `0o660`.
    pub socket_mode: u32,
    /// Group that owns the socket. `None` leaves the group as created.
    pub socket_group: Option<String>,
}

impl PrivConfig {
    pub fn new(socket_path: impl Into<PathBuf>, allowed_uids: Vec<u32>) -> Self {
        Self {
            socket_path: socket_path.into(),
            allowed_uids,
            socket_mode: 0o660,
            socket_group: None,
        }
    }
}

/// Look up user names in the system user database.
///
/// Returns the ids found and the names that do not exist (the caller decides
/// whether that is worth a warning).
pub fn resolve_users(names: &[String]) -> (Vec<u32>, Vec<String>) {
    let mut uids = Vec::new();
    let mut missing = Vec::new();
    for name in names {
        match User::from_name(name) {
            Ok(Some(user)) => uids.push(user.uid.as_raw()),
            _ => missing.push(name.clone()),
        }
    }
    (uids, missing)
}
