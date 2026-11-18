use std::path::PathBuf;

use nix::unistd::User;

pub const DEFAULT_SOCKET_PATH: &str = "/run/opsbox/priv.sock";
pub const DEFAULT_MASTER_KEY_PATH: &str = "/etc/opsbox/master.key";
pub const DEFAULT_XFER_DIR: &str = "/var/lib/opsbox/xfer";
pub const DEFAULT_TRASH_DIR: &str = "/var/lib/opsbox/trash";
pub const DEFAULT_APPS_DIR: &str = "/var/lib/opsbox/apps";
pub const DEFAULT_CONFIG_PATH: &str = "/etc/opsbox/config.toml";
pub const DEFAULT_DB_PATH: &str = "/var/lib/opsbox/opsbox.db";
pub const DEFAULT_BACKUP_DIR: &str = "/var/backups/opsbox";
pub const DEFAULT_BACKUP_CACHE_DIR: &str = "/var/cache/opsbox/restic";

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
    /// File holding the 32-byte key that seals secrets. Created (0600) on first use.
    pub master_key_path: PathBuf,
    /// Where uploads and downloads are staged. `serve` creates it; `priv` only reads it.
    pub xfer_dir: PathBuf,
    /// Where deleted files wait for seven days.
    pub trash_dir: PathBuf,
    /// Where service source checkouts and app files live.
    pub apps_dir: PathBuf,
    /// Path to `config.toml` updated by `WebLocalOnly` and backed up by `opsbox` self-backup.
    pub config_path: PathBuf,
    /// Path to `opsbox.db` used for read-only online SQLite backups.
    pub db_path: PathBuf,
    /// Local restic repository path (defaults to `/var/backups/opsbox`).
    pub backup_dir: PathBuf,
    /// Local restic cache & ephemeral staging directory (defaults to `/var/cache/opsbox/restic`).
    pub backup_cache_dir: PathBuf,
    /// Optional explicit `restic` CLI binary path (defaults to `/usr/local/bin/restic`).
    pub restic_bin: Option<PathBuf>,
    /// Docker Engine Unix socket path (defaults to `/var/run/docker.sock`).
    pub docker_socket: PathBuf,
    /// Optional explicit Docker CLI binary path (defaults to searching system PATH).
    pub docker_bin: Option<PathBuf>,
    /// Allow `file://` git clones inside integration tests (never enabled in production).
    pub allow_test_git: bool,
    /// Override only in fixture tests; production uses Linux procfs.
    pub network_proc_root: PathBuf,
    pub network_systemctl: Option<PathBuf>,
}

impl PrivConfig {
    pub fn new(socket_path: impl Into<PathBuf>, allowed_uids: Vec<u32>) -> Self {
        Self {
            socket_path: socket_path.into(),
            allowed_uids,
            socket_mode: 0o660,
            socket_group: None,
            master_key_path: DEFAULT_MASTER_KEY_PATH.into(),
            xfer_dir: DEFAULT_XFER_DIR.into(),
            trash_dir: DEFAULT_TRASH_DIR.into(),
            apps_dir: DEFAULT_APPS_DIR.into(),
            config_path: DEFAULT_CONFIG_PATH.into(),
            db_path: DEFAULT_DB_PATH.into(),
            backup_dir: DEFAULT_BACKUP_DIR.into(),
            backup_cache_dir: DEFAULT_BACKUP_CACHE_DIR.into(),
            restic_bin: None,
            docker_socket: opsbox_docker::DEFAULT_SOCKET_PATH.into(),
            docker_bin: None,
            allow_test_git: false,
            network_proc_root: "/proc".into(),
            network_systemctl: None,
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
