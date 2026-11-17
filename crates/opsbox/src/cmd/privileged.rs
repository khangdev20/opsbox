use anyhow::{Context, Result, bail};
use nix::unistd::geteuid;
use opsbox_priv::{PrivConfig, PrivServer, resolve_users};
use tracing::warn;

use crate::config::{Config, DEFAULT_CONFIG_PATH};

pub fn run(config: Config, explicit_config: Option<&std::path::Path>) -> Result<()> {
    if !geteuid().is_root() {
        warn!(
            "not running as root: this is fine for development, but real privileged operations will fail"
        );
    }

    let (mut uids, missing) = resolve_users(&config.allowed_users);
    for name in &missing {
        warn!(user = %name, "allowed user does not exist on this machine; skipping it");
    }
    uids.extend(&config.allowed_uids);
    uids.sort_unstable();
    uids.dedup();
    if uids.is_empty() {
        bail!(
            "no user is allowed to use the privileged helper: none of {:?} exists here and no --allow-uid was given",
            config.allowed_users
        );
    }

    let mut priv_config = PrivConfig::new(&config.priv_socket, uids);
    priv_config.socket_mode = config.socket_mode;
    priv_config.socket_group.clone_from(&config.socket_group);
    priv_config.master_key_path.clone_from(&config.master_key);
    priv_config.xfer_dir.clone_from(&config.xfer_dir);
    priv_config.trash_dir.clone_from(&config.trash_dir);
    priv_config.config_path = explicit_config
        .map(std::path::Path::to_path_buf)
        .or(config.loaded_from)
        .unwrap_or_else(|| std::path::PathBuf::from(DEFAULT_CONFIG_PATH));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start the async runtime")?;
    runtime.block_on(async {
        let server = PrivServer::bind(priv_config).context("cannot start the privileged helper")?;
        server.run(super::shutdown_signal()).await;
        Ok(())
    })
}
