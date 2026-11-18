//! The `opsbox` binary: one executable, many roles (docs/BACKEND_SPEC.md section 2).

mod cli;
mod cmd;
mod config;
mod logging;

use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

use crate::cli::{AuditAction, BackupAction, CleanupAction, Cli, Command, WebAction};
use crate::config::{Config, Overrides};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Exit status for a command that is declared but not built yet.
const EXIT_NOT_IMPLEMENTED: u8 = 2;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("opsbox: error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<ExitCode> {
    if let Some(name) = cli.command.unimplemented_name() {
        eprintln!("opsbox: `{name}` is not implemented yet");
        return Ok(ExitCode::from(EXIT_NOT_IMPLEMENTED));
    }

    let ok_or_failure = |ok: bool| {
        if ok {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    };
    match &cli.command {
        Command::Version => println!("opsbox {VERSION}"),
        Command::Install(args) => cmd::install::run(args),
        Command::Selftest => return Ok(ok_or_failure(cmd::selftest::run())),
        Command::Status(_) => {
            let config = Config::load(cli.config.as_deref(), Overrides::from_cli(cli))?;
            return Ok(ok_or_failure(cmd::status::run(&config)));
        }
        Command::Doctor => {
            let config = Config::load(cli.config.as_deref(), Overrides::from_cli(cli))?;
            return Ok(ok_or_failure(cmd::doctor::run(&config)?));
        }
        Command::Serve(_) => {
            let config = Config::load(cli.config.as_deref(), Overrides::from_cli(cli))?;
            logging::init(&config)?;
            cmd::serve::run(config)?;
        }
        Command::Cleanup { action, .. } => match action {
            Some(CleanupAction::Scan { timings }) => {
                return Ok(ok_or_failure(cmd::cleanup::scan(*timings)));
            }
            None => return Ok(ok_or_failure(cmd::cleanup::run())),
        },
        Command::SetupToken => {
            let config = Config::load(cli.config.as_deref(), Overrides::from_cli(cli))?;
            cmd::admin::setup_token(&config)?;
        }
        Command::User { action } => {
            let config = Config::load(cli.config.as_deref(), Overrides::from_cli(cli))?;
            cmd::admin::user(&config, action)?;
        }
        Command::Audit {
            action: AuditAction::Verify,
        } => {
            let config = Config::load(cli.config.as_deref(), Overrides::from_cli(cli))?;
            return Ok(ok_or_failure(cmd::admin::audit_verify(&config)?));
        }
        Command::Priv(_) => {
            let config = Config::load(cli.config.as_deref(), Overrides::from_cli(cli))?;
            logging::init(&config)?;
            cmd::privileged::run(config, cli.config.as_deref())?;
        }
        Command::Proxy(_) => {
            let config = Config::load(cli.config.as_deref(), Overrides::from_cli(cli))?;
            logging::init(&config)?;
            cmd::proxy::run(config)?;
        }
        Command::Watchdog(_) => {
            let config = Config::load(cli.config.as_deref(), Overrides::from_cli(cli))?;
            logging::init(&config)?;
            cmd::watchdog::run(config)?;
        }
        Command::WatchdogVerifyPw => return Ok(cmd::watchdog::verify_pw()),
        Command::RecoveryCode => {
            let config = Config::load(cli.config.as_deref(), Overrides::from_cli(cli))?;
            cmd::watchdog::recovery_code(&config)?;
        }
        Command::Update { channel, version } => {
            let config = Config::load(cli.config.as_deref(), Overrides::from_cli(cli))?;
            cmd::update::run_update(&config, channel.as_deref(), version.as_deref())?;
        }
        Command::Rollback => {
            let config = Config::load(cli.config.as_deref(), Overrides::from_cli(cli))?;
            cmd::update::run_rollback(&config)?;
        }
        Command::UpdateVerify { from, to } => {
            let config = Config::load(cli.config.as_deref(), Overrides::from_cli(cli))?;
            return Ok(ok_or_failure(cmd::update::run_update_verify(
                &config,
                from.as_deref(),
                to,
            )));
        }
        Command::Web {
            action: WebAction::LocalOnly { mode },
        } => {
            cmd::web::local_only(cli.config.as_deref(), *mode)?;
        }
        Command::Web {
            action:
                WebAction::Domain {
                    name,
                    skip_dns_check,
                },
        } => {
            let config = Config::load(cli.config.as_deref(), Overrides::from_cli(cli))?;
            cmd::web::domain(&config, name, *skip_dns_check)?;
        }
        Command::Backup { action } => {
            let config = Config::load(cli.config.as_deref(), Overrides::from_cli(cli))?;
            match action {
                BackupAction::List => cmd::backup::list(&config)?,
                BackupAction::Now { job, prune, check } => {
                    return Ok(ok_or_failure(cmd::backup::now(
                        &config,
                        job.as_deref(),
                        *prune,
                        *check,
                    )?));
                }
            }
        }
        Command::Restore { snapshot, to } => {
            let config = Config::load(cli.config.as_deref(), Overrides::from_cli(cli))?;
            cmd::backup::restore(&config, snapshot, to)?;
        }
        // Handled above; listed so adding a command forces a decision here.
        _ => unreachable!("unimplemented commands return before this match"),
    }
    Ok(ExitCode::SUCCESS)
}
