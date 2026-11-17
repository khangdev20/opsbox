use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use opsbox_watchdog::WatchdogConfig;

use crate::config::Config;

fn data_dir_from_db(db_path: &Path) -> &Path {
    db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("/var/lib/opsbox"))
}

pub fn run(config: Config) -> Result<()> {
    // Watchdog runs under `MemoryMax=32M` in systemd, so keep the runtime single-threaded.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(2)
        .build()
        .context("cannot start the async runtime")?;

    let data_dir = data_dir_from_db(&config.db_path);
    let binary_symlink = std::env::current_exe()
        .ok()
        .filter(|p| p.parent() == Some(Path::new("/usr/local/bin")))
        .unwrap_or_else(|| PathBuf::from("/usr/local/bin/opsbox"));

    let wd_cfg = WatchdogConfig {
        listen: config.watchdog_listen,
        serve_listen: config.listen,
        priv_socket: config.priv_socket,
        proxy_socket: config.proxy_socket,
        db_path: config.db_path.clone(),
        master_key_path: config.master_key,
        runtime_dir: config.watchdog_runtime_dir,
        slots_dir: data_dir.join("bin"),
        binary_symlink,
        update_state_file: data_dir.join("update-state.json"),
        xfer_dir: config.xfer_dir,
        cert_path: opsbox_api::tls::DEFAULT_CERT_PATH.into(),
        key_path: opsbox_api::tls::DEFAULT_KEY_PATH.into(),
        fallback_cert_path: data_dir.join("panel-selfsigned.crt"),
        fallback_key_path: data_dir.join("panel-selfsigned.key"),
        ..WatchdogConfig::default()
    };

    runtime.block_on(opsbox_watchdog::run(wd_cfg, super::shutdown_signal()))?;
    Ok(())
}

pub fn recovery_code(config: &Config) -> Result<()> {
    let code = opsbox_watchdog::auth::generate_recovery_code(&config.watchdog_runtime_dir)
        .map_err(|e| {
            anyhow::anyhow!(
                "{e} (run as root with `sudo opsbox recovery-code` when Safe Mode runs as root)"
            )
        })?;

    let port = config.watchdog_listen.port();
    let host_hint = if config.watchdog_listen.ip().is_loopback() {
        "127.0.0.1".to_owned()
    } else {
        opsbox_api::tls::detect_public_ip()
            .map_or_else(|| "<server-ip>".to_owned(), |ip| ip.to_string())
    };
    println!("opsbox safe mode recovery code");
    println!("  code      {code}");
    println!("  expires   in 15 minutes (single use)");
    println!("  safe mode https://{host_hint}:{port}/");
    if config.watchdog_listen.ip().is_loopback() {
        println!("  tunnel    ssh -L {port}:127.0.0.1:{port} <user>@<server>");
    }
    Ok(())
}

/// Helper invoked inside a transient 128M systemd scope when verifying 64 MiB
/// Argon2id hashes while `opsbox-watchdog.service` stays capped at `MemoryMax=32M`.
/// Reads two lines from stdin: stored PHC hash, then plaintext password.
pub fn verify_pw() -> ExitCode {
    let mut input = String::new();
    if io::stdin().take(4096).read_to_string(&mut input).is_err() {
        return ExitCode::from(2);
    }
    let mut lines = input.splitn(3, '\n');
    let Some(phc) = lines.next().map(|s| s.trim_end_matches('\r')) else {
        return ExitCode::from(2);
    };
    let Some(pw) = lines.next().map(|s| s.trim_end_matches('\r')) else {
        return ExitCode::from(2);
    };
    if phc.is_empty() || pw.is_empty() {
        return ExitCode::from(2);
    }
    if opsbox_watchdog::auth::verify_password_sync(pw, phc) {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
