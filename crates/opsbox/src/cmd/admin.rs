//! Commands an administrator runs over SSH: `setup-token`, `user ...`, `audit verify`.
//!
//! They work on the database directly, so they work when the web server is down or
//! locked. When started as root they first become the `opsbox` user: the database
//! files belong to that user, and a root-owned `-wal` file left behind would stop
//! `serve` from opening the database.

use std::io::{BufRead, IsTerminal, Write};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use nix::unistd::{Uid, User, geteuid, setgid, setuid};
use opsbox_auth::{Auth, AuthConfig, AuthError};
use opsbox_priv::PrivClient;
use opsbox_store::{AuditError, Store};
use tracing::warn;

use crate::cli::UserAction;
use crate::config::Config;

const SERVICE_USER: &str = "opsbox";

/// Give up root, if we have it, in favour of the service account.
fn drop_to_service_user() -> Result<()> {
    if !geteuid().is_root() {
        return Ok(());
    }
    match User::from_name(SERVICE_USER) {
        Ok(Some(user)) => {
            // Groups first, then gid, then uid: after setuid we could no longer change them.
            #[cfg(not(target_vendor = "apple"))]
            nix::unistd::setgroups(&[user.gid]).context("cannot drop supplementary groups")?;
            setgid(user.gid).context("cannot switch group")?;
            setuid(Uid::from_raw(user.uid.as_raw())).context("cannot switch user")?;
            Ok(())
        }
        _ => {
            warn!(
                "user `{SERVICE_USER}` does not exist; continuing as root (fine on a development machine)"
            );
            Ok(())
        }
    }
}

fn open(config: &Config) -> Result<Arc<Store>> {
    drop_to_service_user()?;
    let store = Store::open(&config.db_path)
        .with_context(|| format!("cannot open the database at {} (are you root or the opsbox user? use --db for another path)", config.db_path.display()))?;
    Ok(Arc::new(store))
}

pub fn auth_for(config: &Config, store: Arc<Store>) -> Auth {
    let mut cfg = AuthConfig::default();
    cfg.hash_params.memory_kib = config.argon2_memory_kib;
    cfg.hash_params.iterations = config.argon2_iterations;
    // The helper is not used by these commands.
    Auth::new(store, PrivClient::new(&config.priv_socket), cfg)
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start the async runtime")
}

/// Who to record in the audit log for a command run from the shell.
fn actor() -> String {
    let who = std::env::var("SUDO_USER")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "unknown".into());
    format!("cli:{who}")
}

pub fn setup_token(config: &Config) -> Result<()> {
    let store = open(config)?;
    let auth = auth_for(config, store);
    match auth.issue_setup_token() {
        Ok(token) => {
            print_setup_banner(config, &token);
            Ok(())
        }
        Err(AuthError::NotFound) => bail!(
            "an admin account already exists, so setup is closed. To get back into the panel use `opsbox user password <name>` or `opsbox user reset-2fa <name>`"
        ),
        Err(e) => Err(e.into()),
    }
}

/// The text shown at install time, on the console and in the journal.
pub fn setup_banner(config: &Config, token: &str) -> String {
    let url = format!("http://{}/#/setup", config.listen);
    format!(
        "opsbox first-run setup\n  setup url    {url}\n  setup token  {token}   (single use, valid for 1 hour)\n  The server listens on this machine only. From your computer: ssh -L {port}:127.0.0.1:{port} <user>@<server>\n",
        port = config.listen.port()
    )
}

fn print_setup_banner(config: &Config, token: &str) {
    print!("{}", setup_banner(config, token));
}

pub fn user(config: &Config, action: &UserAction) -> Result<()> {
    let store = open(config)?;
    let auth = auth_for(config, store);
    let rt = runtime()?;
    let by = actor();
    match action {
        UserAction::Reset2fa { name } => {
            auth.reset_2fa(name, &by).map_err(user_err)?;
            println!(
                "Two-factor authentication was reset for {name} and all their sessions ended."
            );
            println!(
                "At the next sign-in they enter their password only and are taken to set up a new authenticator."
            );
        }
        UserAction::Unlock { name } => {
            auth.unlock(name, &by).map_err(user_err)?;
            println!(
                "{name} is unlocked. (A locked-out address stays locked until its wait ends.)"
            );
        }
        UserAction::Password { name } => {
            let password = read_new_password(name)?;
            rt.block_on(auth.set_password(name, &password, &by))
                .map_err(user_err)?;
            println!("The password for {name} was changed and all their sessions ended.");
        }
    }
    Ok(())
}

fn user_err(e: AuthError) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}

/// Ask twice on a terminal; read one line from standard input when piped (scripts).
fn read_new_password(name: &str) -> Result<String> {
    if std::io::stdin().is_terminal() {
        let first = rpassword::prompt_password(format!("New password for {name}: "))?;
        let again = rpassword::prompt_password("Repeat it: ")?;
        if first != again {
            bail!("the two passwords do not match");
        }
        Ok(first)
    } else {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        let pw = line.trim_end_matches(['\n', '\r']).to_owned();
        if pw.is_empty() {
            bail!("no password was given on standard input");
        }
        Ok(pw)
    }
}

/// `opsbox audit verify`. Returns `false` if the chain is broken.
pub fn audit_verify(config: &Config) -> Result<bool> {
    let store = open(config)?;
    match store.verify_audit() {
        Ok(report) => {
            println!(
                "Audit log is intact: {} entries, head {}",
                report.entries, report.head
            );
            Ok(true)
        }
        Err(AuditError::Broken { seq, reason }) => {
            println!("AUDIT LOG IS BROKEN at entry {seq}: {reason}");
            std::io::stdout().flush().ok();
            Ok(false)
        }
        Err(e) => Err(e.into()),
    }
}
