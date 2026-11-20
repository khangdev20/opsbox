//! `opsbox proxy`: reverse proxy on :80/:443 for panel and app domains.

use anyhow::{Context, Result};
use nix::unistd::geteuid;
use opsbox_priv::resolve_users;
use opsbox_proxy::{AcmeDirectory, ProxyConfig};

use crate::config::Config;

pub fn run(config: Config) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("cannot start the async runtime")?;

    let (mut uids, _) = resolve_users(&["opsbox".to_owned(), "root".to_owned()]);
    uids.push(geteuid().as_raw());
    uids.push(0);

    let acme_directory = if let Some(ref url) = config.acme_directory {
        AcmeDirectory::Custom(url.clone())
    } else if config.acme_staging {
        AcmeDirectory::Staging
    } else {
        AcmeDirectory::Production
    };

    let mut proxy_cfg = ProxyConfig::new(&config.priv_proxy_socket);
    proxy_cfg.listen_http = config.proxy_listen_http;
    proxy_cfg.listen_https = config.proxy_listen_https;
    proxy_cfg.fallback_http = config.proxy_fallback_http;
    proxy_cfg.fallback_https = config.proxy_fallback_https;
    proxy_cfg.socket_path = config.proxy_socket;
    proxy_cfg.state_dir = config.proxy_state_dir;
    proxy_cfg.allowed_uids = uids;
    proxy_cfg.acme_directory = acme_directory;
    proxy_cfg.acme_email = config.acme_email;
    proxy_cfg.max_body_bytes = config.max_upload_mb.saturating_mul(1 << 20);

    runtime
        .block_on(opsbox_proxy::run(proxy_cfg, super::shutdown_signal()))
        .context("opsbox proxy failed")?;
    Ok(())
}
