//! `opsbox update`, `opsbox rollback`, and internal `opsbox update-verify`.

use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};
use opsbox_priv::PrivClient;
use opsbox_update::{Channel, NoOfferReason, SlotLayout, UpdateOutcome};

use crate::VERSION;
use crate::config::Config;

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

pub fn run_update(config: &Config, channel: Option<&str>, version: Option<&str>) -> Result<()> {
    // --channel, then [update] channel from the config file, then the default for this
    // version. (The panel's own setting lives in the database; the CLI does not read it.)
    let explicit = match channel {
        Some(c) => Some(
            Channel::parse(c)
                .with_context(|| format!("unknown channel '{c}' (use stable or beta)"))?,
        ),
        None => config.update_channel,
    };
    let target_ver = match version {
        // An explicit version skips the comparison: this is the manual path for an
        // install that cannot see a newer release.
        Some(v) => v.trim_start_matches('v').to_owned(),
        None => {
            let res = match explicit {
                Some(ch) => opsbox_update::check_latest(VERSION, ch),
                None => opsbox_update::check_latest_default_channel(VERSION),
            }
            .context("checking GitHub Releases failed")?;
            match res.reason {
                Some(NoOfferReason::DevBuild) => {
                    println!(
                        "opsbox {VERSION} is a development build and is not updated automatically.\n\
                         To install a release, run: opsbox update --version v<version>"
                    );
                    return Ok(());
                }
                Some(NoOfferReason::NetworkError) => anyhow::bail!(
                    "could not reach GitHub to look for a release ({})",
                    res.detail.as_deref().unwrap_or("unknown error")
                ),
                Some(NoOfferReason::RateLimited) => anyhow::bail!(
                    "GitHub is limiting requests right now; try again in a few minutes"
                ),
                Some(NoOfferReason::NoReleasesOnChannel) => {
                    println!(
                        "There is no release on the {} channel yet.",
                        res.channel.as_str()
                    );
                    return Ok(());
                }
                Some(NoOfferReason::UpToDate) => {
                    println!(
                        "opsbox {VERSION} is already up to date ({} channel).",
                        res.channel.as_str()
                    );
                    return Ok(());
                }
                None => {}
            }
            if res.migration {
                println!(
                    "Your install predates version tagging; updating to get accurate versions."
                );
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
                .args([
                    "restart",
                    "opsbox-priv.service",
                    "opsbox-proxy.service",
                    "opsbox.service",
                ])
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
