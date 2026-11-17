//! `opsbox doctor`: report problems and concrete fixes without changing the host.

use std::time::Duration;

use anyhow::Result;
use opsbox_priv::PrivClient;
use opsbox_proto::CheckStatus;

use crate::config::Config;

pub fn run(config: &Config) -> Result<bool> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let status = super::status::collect(config).await;
        print!("{}", status.render());
        println!("\nDiagnostics (read-only):");
        if !config.listen.ip().is_loopback() {
            println!("WARN  panel: {} has no native TLS. Bind to loopback or use a trusted HTTPS proxy.", config.listen);
        }
        let client = PrivClient::new(&config.priv_socket).with_timeout(Duration::from_secs(40));
        let report = match client.doctor("doctor").await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("FAIL  helper diagnostics: {e}");
                eprintln!("      Run sudo opsbox doctor; check sudo systemctl status opsbox-priv and the configured socket/allowed_users.");
                return Ok(false);
            }
        };
        for c in &report.checks {
            let tag = match c.status { CheckStatus::Ok => "OK", CheckStatus::Warn => "WARN", CheckStatus::Fail => "FAIL", CheckStatus::Skip => "SKIP" };
            println!("{tag:4}  {}: {}", c.id, c.message);
            if matches!(c.status, CheckStatus::Warn | CheckStatus::Fail) && let Some(fix) = &c.fix {
                println!("      {fix}");
            }
        }
        Ok(status.healthy() && report.healthy())
    })
}
