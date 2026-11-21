//! `opsbox proxy`: reverse proxy on :80/:443 for panel and app domains.

use anyhow::{Context, Result};
use nix::unistd::geteuid;
use opsbox_priv::resolve_users;
use opsbox_proxy::{AcmeDirectory, ProxyConfig};

use crate::config::{Config, ProxyMode};

pub fn run(config: Config) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("cannot start the async runtime")?;

    runtime.block_on(run_mode(config, run_builtin, super::shutdown_signal()))
}

// Keep disabled services alive until systemd stops them (the unit uses Restart=always).
// The builtin callback owns every listener, control socket and ACME startup action.
async fn run_mode<F: std::future::Future<Output = Result<()>>>(
    config: Config,
    builtin: impl FnOnce(Config) -> F,
    shutdown: impl std::future::Future<Output = ()>,
) -> Result<()> {
    if config.proxy_mode == ProxyMode::Builtin {
        builtin(config).await
    } else {
        tracing::info!(mode = %config.proxy_mode, "public proxy listeners disabled");
        shutdown.await;
        Ok(())
    }
}

async fn run_builtin(config: Config) -> Result<()> {
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

    opsbox_proxy::run(proxy_cfg, super::shutdown_signal())
        .await
        .context("opsbox proxy failed")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Overrides, ProxyMode};
    use std::cell::Cell;

    #[tokio::test]
    async fn proxy_mode_external_and_off_never_start_or_bind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for mode in ["external", "off", "builtin"] {
            std::fs::write(&path, format!("[proxy]\nmode = {mode:?}\n")).unwrap();
            let config = Config::load(Some(&path), Overrides::default()).unwrap();
            let started = Cell::new(false);
            let stopped = Cell::new(false);
            run_mode(
                config,
                |_| async {
                    started.set(true);
                    Ok(())
                },
                async {
                    stopped.set(true);
                },
            )
            .await
            .unwrap();
            assert_eq!(started.get(), mode == "builtin");
            assert_eq!(stopped.get(), mode != "builtin");
        }
    }

    #[tokio::test]
    async fn proxy_mode_external_waits_for_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[proxy]\nmode = \"external\"\n").unwrap();
        let config = Config::load(Some(&path), Overrides::default()).unwrap();
        assert_eq!(config.proxy_mode, ProxyMode::External);
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(10),
                run_mode(
                    config,
                    |_| async { panic!("must not bind") },
                    std::future::pending()
                )
            )
            .await
            .is_err()
        );
    }
}
