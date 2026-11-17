//! Command-line interface (docs/BACKEND_SPEC.md section 9).
//!
//! Every command from the spec exists here so `--help` shows the final surface and
//! scripts can rely on the names. Only some are implemented in Phase 0; the rest
//! print "not implemented yet" and exit with status 2.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::config::LogFormat;

#[derive(Debug, Parser)]
#[command(
    name = "opsbox",
    version,
    about = "All-in-one Linux server manager",
    propagate_version = true
)]
pub struct Cli {
    /// Configuration file (default: /etc/opsbox/config.toml; it is fine if that one is missing)
    #[arg(long, global = true, env = "OPSBOX_CONFIG", value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Log filter, e.g. "info" or "info,opsbox_priv=debug"
    #[arg(long, global = true, env = "OPSBOX_LOG_LEVEL", value_name = "FILTER")]
    pub log_level: Option<String>,

    /// Log format
    #[arg(long, global = true, env = "OPSBOX_LOG_FORMAT", value_enum)]
    pub log_format: Option<LogFormat>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    // ----------------------------------------------------------- Phase 0
    /// Run the HTTP server (unprivileged; used by opsbox.service)
    Serve(ServeArgs),
    /// Run the privileged helper (root; used by opsbox-priv.service)
    Priv(PrivArgs),
    /// Show whether serve and priv are reachable (exit status 0 = healthy)
    Status(StatusArgs),
    /// Print the version
    Version,
    /// Set this machine up. Phase 0: a stub that only prints what it would do
    Install(InstallArgs),
    /// Check that this binary works on this machine (used by the updater)
    Selftest,

    // ------------------------------------------------- not implemented yet
    /// Print a new pairing code
    Pair,
    /// Turn the optional web panel on or off
    Web {
        #[command(subcommand)]
        action: WebAction,
    },
    /// Print a new one-time setup token (only while no admin exists)
    SetupToken,
    /// Print a one-time code for safe mode
    RecoveryCode,
    /// Recover panel accounts over SSH
    User {
        #[command(subcommand)]
        action: UserAction,
    },
    /// Update opsbox
    Update {
        /// Release channel
        #[arg(long)]
        channel: Option<String>,
    },
    /// Go back to the previous version
    Rollback,
    /// Back up without the UI
    Backup {
        #[command(subcommand)]
        action: BackupAction,
    },
    /// Restore a snapshot
    Restore {
        snapshot: String,
        /// Where to restore to
        #[arg(long)]
        to: PathBuf,
    },
    /// Audit log tools
    Audit {
        #[command(subcommand)]
        action: AuditAction,
    },
    /// Diagnose common problems
    Doctor,
    /// Remove opsbox; apps and data keep running
    Uninstall {
        /// Keep /var/lib/opsbox and /etc/opsbox
        #[arg(long)]
        keep_data: bool,
    },
    /// Run the reverse proxy (internal)
    Proxy,
    /// Run the watchdog (internal)
    Watchdog,
}

#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Address to listen on [default: 127.0.0.1:8443]
    #[arg(long, env = "OPSBOX_LISTEN", value_name = "ADDR")]
    pub listen: Option<SocketAddr>,
    /// Privileged helper socket [default: /run/opsbox/priv.sock]
    #[arg(long, env = "OPSBOX_PRIV_SOCKET", value_name = "PATH")]
    pub socket: Option<PathBuf>,
    /// SQLite database file [default: /var/lib/opsbox/opsbox.db]
    #[arg(long, env = "OPSBOX_DB", value_name = "PATH")]
    pub db: Option<PathBuf>,
    /// Folder for staged uploads and downloads [default: /var/lib/opsbox/xfer]
    #[arg(long, env = "OPSBOX_XFER_DIR", value_name = "PATH")]
    pub xfer_dir: Option<PathBuf>,
    /// Mark cookies Secure: auto (behind an HTTPS proxy), always or never [default: auto]
    #[arg(long, env = "OPSBOX_SECURE_COOKIES", value_enum)]
    pub secure_cookies: Option<crate::config::SecureCookiesSetting>,
}

#[derive(Debug, Args)]
pub struct PrivArgs {
    /// Socket to listen on [default: /run/opsbox/priv.sock]
    #[arg(long, env = "OPSBOX_PRIV_SOCKET", value_name = "PATH")]
    pub socket: Option<PathBuf>,
    /// Also allow this numeric user id (repeatable; adds to the configured list)
    #[arg(
        long = "allow-uid",
        env = "OPSBOX_PRIV_ALLOW_UIDS",
        value_name = "UID",
        value_delimiter = ','
    )]
    pub allow_uids: Vec<u32>,
    /// Also allow this user name (repeatable; adds to the configured list)
    #[arg(
        long = "allow-user",
        env = "OPSBOX_PRIV_ALLOW_USERS",
        value_name = "NAME",
        value_delimiter = ','
    )]
    pub allow_users: Vec<String>,
    /// Socket file mode, octal [default: 0660]
    #[arg(long, env = "OPSBOX_PRIV_SOCKET_MODE", value_name = "MODE")]
    pub socket_mode: Option<String>,
    /// Group that owns the socket; empty for none [default: opsbox]
    #[arg(long, env = "OPSBOX_PRIV_SOCKET_GROUP", value_name = "GROUP")]
    pub socket_group: Option<String>,
    /// File with the key that seals secrets [default: /etc/opsbox/master.key]
    #[arg(long, env = "OPSBOX_PRIV_MASTER_KEY", value_name = "PATH")]
    pub master_key: Option<PathBuf>,
    /// Folder for staged uploads and downloads [default: /var/lib/opsbox/xfer]
    #[arg(long, env = "OPSBOX_XFER_DIR", value_name = "PATH")]
    pub xfer_dir: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Address serve listens on [default: 127.0.0.1:8443]
    #[arg(long, env = "OPSBOX_LISTEN", value_name = "ADDR")]
    pub listen: Option<SocketAddr>,
    /// Privileged helper socket [default: /run/opsbox/priv.sock]
    #[arg(long, env = "OPSBOX_PRIV_SOCKET", value_name = "PATH")]
    pub socket: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct InstallArgs {
    /// Pairing secret from the desktop app
    #[arg(long, value_name = "SECRET")]
    pub pair: Option<String>,
    /// Turn the web panel on
    #[arg(long)]
    pub web: bool,
    /// Release channel
    #[arg(long, value_name = "CHANNEL")]
    pub channel: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum WebAction {
    Enable {
        /// Serve the panel on this domain
        #[arg(long)]
        domain: Option<String>,
    },
    Disable,
}

#[derive(Debug, Subcommand)]
pub enum UserAction {
    #[command(name = "reset-2fa")]
    Reset2fa {
        name: String,
    },
    Unlock {
        name: String,
    },
    Password {
        name: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum BackupAction {
    Now,
    List,
}

#[derive(Debug, Subcommand)]
pub enum AuditAction {
    /// Check the audit log's hash chain
    Verify,
}

impl Command {
    /// For commands that are declared but not built yet: the name to report.
    pub fn unimplemented_name(&self) -> Option<&'static str> {
        Some(match self {
            Self::Serve(_)
            | Self::Priv(_)
            | Self::Status(_)
            | Self::Version
            | Self::Install(_)
            | Self::Selftest
            | Self::SetupToken
            | Self::User { .. }
            | Self::Audit { .. } => return None,
            Self::Pair => "pair",
            Self::Web {
                action: WebAction::Enable { .. },
            } => "web enable",
            Self::Web {
                action: WebAction::Disable,
            } => "web disable",
            Self::RecoveryCode => "recovery-code",
            Self::Update { .. } => "update",
            Self::Rollback => "rollback",
            Self::Backup {
                action: BackupAction::Now,
            } => "backup now",
            Self::Backup {
                action: BackupAction::List,
            } => "backup list",
            Self::Restore { .. } => "restore",
            Self::Doctor => "doctor",
            Self::Uninstall { .. } => "uninstall",
            Self::Proxy => "proxy",
            Self::Watchdog => "watchdog",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn every_command_from_the_spec_parses() {
        let lines = [
            "install",
            "install --pair abc --web --channel beta",
            "status",
            "pair",
            "web enable",
            "web enable --domain panel.example.com",
            "web disable",
            "setup-token",
            "recovery-code",
            "user reset-2fa alice",
            "user unlock alice",
            "user password alice",
            "update",
            "update --channel beta",
            "rollback",
            "backup now",
            "backup list",
            "restore snap1 --to /tmp/x",
            "audit verify",
            "doctor",
            "uninstall",
            "uninstall --keep-data",
            "serve",
            "priv",
            "proxy",
            "watchdog",
            "selftest",
            "version",
        ];
        for line in lines {
            let args = std::iter::once("opsbox").chain(line.split_whitespace());
            Cli::try_parse_from(args)
                .unwrap_or_else(|e| panic!("`opsbox {line}` does not parse: {e}"));
        }
    }

    #[test]
    fn unimplemented_commands_are_named_and_phase_0_ones_are_not() {
        let parse = |line: &str| {
            Cli::try_parse_from(std::iter::once("opsbox").chain(line.split_whitespace()))
                .unwrap()
                .command
        };
        for implemented in ["serve", "priv", "status", "version", "install", "selftest"] {
            assert_eq!(
                parse(implemented).unimplemented_name(),
                None,
                "{implemented}"
            );
        }
        assert_eq!(parse("doctor").unimplemented_name(), Some("doctor"));
        assert_eq!(parse("web enable").unimplemented_name(), Some("web enable"));
        for done in [
            "setup-token",
            "user unlock a",
            "user reset-2fa a",
            "user password a",
            "audit verify",
        ] {
            assert_eq!(parse(done).unimplemented_name(), None, "{done}");
        }
    }
}
