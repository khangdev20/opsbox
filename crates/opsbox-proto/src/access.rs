//! Types for the Access page (docs/BACKEND_SPEC.md 6.4): who can log in to the server.

use serde::{Deserialize, Serialize};

/// A Linux account that can log in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessUser {
    pub name: String,
    pub uid: u32,
    pub groups: Vec<String>,
    /// Member of `sudo`, `wheel` or `admin`.
    pub sudo: bool,
    /// The account is expired, so no login (password or key) works.
    pub locked: bool,
    /// Last login as short text (`Nov 17 07:00`), or `never`, or empty when unknown.
    pub last_login: String,
}

/// One line of an `authorized_keys` file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizedKey {
    pub user: String,
    /// The comment at the end of the line, or empty.
    pub name: String,
    /// `ed25519`, `rsa 3072`, `ecdsa-nistp256`, ...
    pub kind: String,
    /// `SHA256:...`, as `ssh-keygen -l` prints it. Also the key's id.
    pub fingerprint: String,
    /// Restrictions in front of the key (`command="..."`), if any.
    pub options: Option<String>,
    /// The key type is old or weak (DSA, short RSA).
    pub weak: bool,
}

/// What the SSH daemon is set to do (from `sshd -T`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshStatus {
    /// An `sshd` binary exists.
    pub installed: bool,
    /// The service is running.
    pub active: bool,
    pub port: u16,
    /// `None` when the effective settings could not be read.
    pub password_login: Option<bool>,
    /// `yes`, `no`, `prohibit-password` or `forced-commands-only`.
    pub root_login: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessState {
    pub users: Vec<AccessUser>,
    pub keys: Vec<AuthorizedKey>,
    pub ssh: SshStatus,
}
