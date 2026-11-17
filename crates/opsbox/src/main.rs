//! The `opsbox` binary: one executable, many roles (docs/BACKEND_SPEC.md section 2).

mod cli;
mod cmd;
mod config;
mod logging;

use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

use crate::cli::{AuditAction, Cli, Command};
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
        Command::Cleanup { .. } => return Ok(ok_or_failure(cmd::cleanup::run())),
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
            cmd::privileged::run(config)?;
        }
        // Handled above; listed so adding a command forces a decision here.
        _ => unreachable!("unimplemented commands return before this match"),
    }
    Ok(ExitCode::SUCCESS)
}
