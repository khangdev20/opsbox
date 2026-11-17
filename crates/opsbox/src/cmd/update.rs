//! `opsbox update`, `opsbox rollback`, and internal `opsbox update-verify`.

use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};
use opsbox_priv::PrivClient;
use opsbox_update::{Channel, SlotLayout, UpdateOutcome};

use crate::VERSION;
use crate::config::Config;

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

pub fn run_update(config: &Config, channel: Option<&str>, version: Option<&str>) -> Result<()> {
    let ch = match channel {
        Some(c) => Channel::parse(c)
            .with_context(|| format!("unknown channel '{c}' (use stable or beta)"))?,
        None => Channel::Stable,
    };
    let target_ver = match version {
        Some(v) => v.trim_start_matches('v').to_owned(),
        None => {
            let res = opsbox_update::check_latest(VERSION, ch)
                .context("checking GitHub Releases failed")?;
            if !res.update_available {
                println!(
                    "opsbox {VERSION} is already up to date ({} channel).",
                    ch.as_str()
                );
                return Ok(());
            }
            res.latest
                .map(|r| r.version)
                .context("no release found on GitHub")?
        }
    };

    println!("Downloading and verifying opsbox {target_ver}...");
    let staged = opsbox_update::download_and_verify(&target_ver, &config.xfer_dir, None)
        .context("download or signature verification failed")?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = PrivClient::new(&config.priv_socket).with_timeout(Duration::from_secs(60));
        client
            .self_update_install(
                "cli-update",
                "cli",
                &staged.to_string_lossy(),
                &target_ver,
                opsbox_store::LATEST_VERSION,
            )
            .await
    })
    .map_err(|e| anyhow::anyhow!("privileged update failed: {e}"))?;

    println!(
        "opsbox {target_ver} installed; restarting units (automatic rollback check in 2 minutes)."
    );
    Ok(())
}

pub fn run_rollback(config: &Config) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = PrivClient::new(&config.priv_socket).with_timeout(Duration::from_secs(60));
        client
            .self_update_rollback("cli-rollback", "cli", None)
            .await
    })
    .map_err(|e| anyhow::anyhow!("rollback failed: {e}"))?;

    println!("Rolled back to previous opsbox version; restarting units.");
    Ok(())
}

pub fn run_update_verify(config: &Config, from: Option<&str>, to: &str) -> bool {
    let healthy = super::status::run(config);
    let layout = SlotLayout::default();
    let mut state = layout.read_state();

    if healthy {
        state.last_outcome = Some(UpdateOutcome {
            status: "updated".to_owned(),
            from_version: from.unwrap_or(VERSION).to_owned(),
            to_version: to.to_owned(),
            message: format!("Updated to {to} and passed health check."),
            at: now_secs(),
            audited: false,
        });
        let _ = layout.write_state(&state);
        return true;
    }

    let Some(prev) = from else {
        state.last_outcome = Some(UpdateOutcome {
            status: "failed".to_owned(),
            from_version: VERSION.to_owned(),
            to_version: to.to_owned(),
            message: format!(
                "Update to {to} failed health check and no previous slot is available."
            ),
            at: now_secs(),
            audited: false,
        });
        let _ = layout.write_state(&state);
        return false;
    };

    let db_schema = if config.db_path.exists() {
        opsbox_store::db_schema_version(&config.db_path).unwrap_or(0)
    } else {
        0
    };

    match layout.rollback_slot(Some(prev), db_schema) {
        Ok(rolled) => {
            state = layout.read_state();
            state.last_outcome = Some(UpdateOutcome {
                status: "rolled_back".to_owned(),
                from_version: to.to_owned(),
                to_version: rolled.clone(),
                message: format!("Rolled back from {to} to {rolled} after health check failed."),
                at: now_secs(),
                audited: false,
            });
            let _ = layout.write_state(&state);
            let _ = Command::new("systemctl")
                .args(["restart", "opsbox-priv.service", "opsbox.service"])
                .status();
        }
        Err(e) => {
            state.last_outcome = Some(UpdateOutcome {
                status: "failed".to_owned(),
                from_version: to.to_owned(),
                to_version: prev.to_owned(),
                message: format!("Health check failed for {to} and rollback was refused: {e}"),
                at: now_secs(),
                audited: false,
            });
            let _ = layout.write_state(&state);
        }
    }
    false
}
