//! Configuration: built-in defaults, overridden by `/etc/opsbox/config.toml`,
//! overridden by environment variables, overridden by command-line flags.
//!
//! clap resolves "flag over environment variable" itself, so by the time values
//! reach [`Overrides`] there is one optional value per setting. See docs/CONFIG.md.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde::Deserialize;

use crate::cli::{Cli, Command};

pub const DEFAULT_CONFIG_PATH: &str = "/etc/opsbox/config.toml";
pub const DEFAULT_LISTEN: &str = "127.0.0.1:8443";
pub const DEFAULT_DB_PATH: &str = "/var/lib/opsbox/opsbox.db";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Text,
    Json,
}

/// When session cookies get the `Secure` attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum SecureCookiesSetting {
    /// Behind an HTTPS proxy (`X-Forwarded-Proto: https`).
    Auto,
    Always,
    Never,
}

// ------------------------------------------------------------------ file model

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileConfig {
    api: ApiSection,
    auth: AuthSection,
    files: FilesSection,
    terminal: TerminalSection,
    #[serde(rename = "priv")]
    privileged: PrivSection,
    store: StoreSection,
    log: LogSection,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ApiSection {
    listen: SocketAddr,
    secure_cookies: SecureCookiesSetting,
}

impl Default for ApiSection {
    fn default() -> Self {
        Self {
            listen: DEFAULT_LISTEN.parse().expect("valid default"),
            secure_cookies: SecureCookiesSetting::Auto,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PrivSection {
    socket: PathBuf,
    allowed_users: Vec<String>,
    allowed_uids: Vec<u32>,
    socket_mode: String,
    socket_group: String,
    master_key: PathBuf,
}

impl Default for PrivSection {
    fn default() -> Self {
        Self {
            socket: opsbox_priv::DEFAULT_SOCKET_PATH.into(),
            // `root` so that an administrator can run `sudo opsbox status`.
            allowed_users: vec!["opsbox".into(), "root".into()],
            allowed_uids: Vec::new(),
            socket_mode: "0660".into(),
            socket_group: "opsbox".into(),
            master_key: opsbox_priv::DEFAULT_MASTER_KEY_PATH.into(),
        }
    }
}

/// Password hashing cost (docs/BACKEND_SPEC.md 6.2: 64 MB, 3 passes). Lower it on a
/// very small machine if a sign-in takes noticeably longer than a third of a second.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct AuthSection {
    argon2_memory_kib: u32,
    argon2_iterations: u32,
}

impl Default for AuthSection {
    fn default() -> Self {
        Self {
            argon2_memory_kib: 64 * 1024,
            argon2_iterations: 3,
        }
    }
}

/// Where file transfers and the trash live, and how large an upload may be.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FilesSection {
    allow_external_xfer_dir: bool,
    xfer_dir: PathBuf,
    trash_dir: PathBuf,
    apps_dir: PathBuf,
    max_upload_mb: u64,
}

impl Default for FilesSection {
    fn default() -> Self {
        Self {
            allow_external_xfer_dir: false,
            xfer_dir: opsbox_priv::DEFAULT_XFER_DIR.into(),
            trash_dir: opsbox_priv::DEFAULT_TRASH_DIR.into(),
            apps_dir: "/var/lib/opsbox/apps".into(),
            max_upload_mb: 2048,
        }
    }
}

/// The terminal page.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct TerminalSection {
    /// A terminal nobody has typed in for this many minutes is closed.
    idle_timeout_min: u64,
}

impl Default for TerminalSection {
    fn default() -> Self {
        Self {
            idle_timeout_min: 30,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct StoreSection {
    db_path: PathBuf,
}

impl Default for StoreSection {
    fn default() -> Self {
        Self {
            db_path: DEFAULT_DB_PATH.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct LogSection {
    level: String,
    format: LogFormat,
}

impl Default for LogSection {
    fn default() -> Self {
        Self {
            level: "info".into(),
            format: LogFormat::Text,
        }
    }
}

// ------------------------------------------------------------- resolved model

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    pub priv_socket: PathBuf,
    pub allowed_users: Vec<String>,
    pub allowed_uids: Vec<u32>,
    pub socket_mode: u32,
    pub socket_group: Option<String>,
    pub master_key: PathBuf,
    pub secure_cookies: SecureCookiesSetting,
    pub argon2_memory_kib: u32,
    pub argon2_iterations: u32,
    pub xfer_dir: PathBuf,
    pub trash_dir: PathBuf,
    pub apps_dir: PathBuf,
    pub max_upload_mb: u64,
    pub terminal_idle_min: u64,
    pub db_path: PathBuf,
    pub log_level: String,
    pub log_format: LogFormat,
    /// The file the values were read from, if one was found.
    pub loaded_from: Option<PathBuf>,
}

/// Values given on the command line or in the environment.
#[derive(Debug, Default)]
pub struct Overrides {
    pub listen: Option<SocketAddr>,
    pub priv_socket: Option<PathBuf>,
    pub db_path: Option<PathBuf>,
    pub allow_uids: Vec<u32>,
    pub allow_users: Vec<String>,
    pub socket_mode: Option<String>,
    pub socket_group: Option<String>,
    pub master_key: Option<PathBuf>,
    pub xfer_dir: Option<PathBuf>,
    pub secure_cookies: Option<SecureCookiesSetting>,
    pub log_level: Option<String>,
    pub log_format: Option<LogFormat>,
}

impl Overrides {
    pub fn from_cli(cli: &Cli) -> Self {
        let mut o = Self {
            log_level: cli.log_level.clone(),
            log_format: cli.log_format,
            ..Self::default()
        };
        match &cli.command {
            Command::Serve(a) => {
                o.listen = a.listen;
                o.priv_socket.clone_from(&a.socket);
                o.db_path.clone_from(&a.db);
                o.secure_cookies = a.secure_cookies;
                o.xfer_dir.clone_from(&a.xfer_dir);
            }
            Command::Priv(a) => {
                o.priv_socket.clone_from(&a.socket);
                o.allow_uids.clone_from(&a.allow_uids);
                o.allow_users.clone_from(&a.allow_users);
                o.socket_mode.clone_from(&a.socket_mode);
                o.socket_group.clone_from(&a.socket_group);
                o.master_key.clone_from(&a.master_key);
                o.xfer_dir.clone_from(&a.xfer_dir);
            }
            Command::Status(a) => {
                o.listen = a.listen;
                o.priv_socket.clone_from(&a.socket);
            }
            _ => {}
        }
        o
    }
}

impl Config {
    /// Load the configuration. `explicit_path` comes from `--config` / `OPSBOX_CONFIG`:
    /// if given, the file must exist; if not, the default path is used when present.
    pub fn load(explicit_path: Option<&Path>, overrides: Overrides) -> Result<Self> {
        let (file, loaded_from) = read_file(explicit_path)?;
        Self::resolve(file, loaded_from, overrides)
    }

    fn resolve(file: FileConfig, loaded_from: Option<PathBuf>, o: Overrides) -> Result<Self> {
        let mut allowed_uids = file.privileged.allowed_uids;
        allowed_uids.extend(o.allow_uids);
        let mut allowed_users = file.privileged.allowed_users;
        allowed_users.extend(o.allow_users);

        let socket_group = o.socket_group.unwrap_or(file.privileged.socket_group);
        let mode = o.socket_mode.unwrap_or(file.privileged.socket_mode);

        let xfer_dir = o.xfer_dir.unwrap_or(file.files.xfer_dir);
        if !file.files.allow_external_xfer_dir
            && xfer_dir != std::path::Path::new(opsbox_priv::DEFAULT_XFER_DIR)
        {
            bail!(
                "files.xfer_dir is fixed at /var/lib/opsbox/xfer; set files.allow_external_xfer_dir = true explicitly for an alternate staging directory"
            );
        }
        Ok(Self {
            listen: o.listen.unwrap_or(file.api.listen),
            priv_socket: o.priv_socket.unwrap_or(file.privileged.socket),
            allowed_users,
            allowed_uids,
            socket_mode: parse_mode(&mode)?,
            socket_group: (!socket_group.is_empty()).then_some(socket_group),
            master_key: o.master_key.unwrap_or(file.privileged.master_key),
            secure_cookies: o.secure_cookies.unwrap_or(file.api.secure_cookies),
            argon2_memory_kib: file.auth.argon2_memory_kib,
            argon2_iterations: file.auth.argon2_iterations,
            xfer_dir,
            trash_dir: file.files.trash_dir,
            apps_dir: file.files.apps_dir,
            max_upload_mb: file.files.max_upload_mb,
            terminal_idle_min: file.terminal.idle_timeout_min.clamp(1, 24 * 60),
            db_path: o.db_path.unwrap_or(file.store.db_path),
            log_level: o.log_level.unwrap_or(file.log.level),
            log_format: o.log_format.unwrap_or(file.log.format),
            loaded_from,
        })
    }
}

fn read_file(explicit: Option<&Path>) -> Result<(FileConfig, Option<PathBuf>)> {
    let (path, required) = match explicit {
        Some(p) => (p.to_path_buf(), true),
        None => (PathBuf::from(DEFAULT_CONFIG_PATH), false),
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let parsed = toml::from_str(&text)
                .with_context(|| format!("invalid configuration in {}", path.display()))?;
            Ok((parsed, Some(path)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !required => {
            Ok((FileConfig::default(), None))
        }
        Err(e) => {
            Err(e).with_context(|| format!("cannot read configuration file {}", path.display()))
        }
    }
}

/// Parse an octal file mode such as `0660` or `660`.
fn parse_mode(text: &str) -> Result<u32> {
    match u32::from_str_radix(text.trim_start_matches("0o"), 8) {
        Ok(mode) if mode <= 0o777 => Ok(mode),
        _ => bail!("socket mode {text:?} is not an octal file mode like 0660"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_str(toml_text: &str, o: Overrides) -> Result<Config> {
        let file: FileConfig = toml::from_str(toml_text)?;
        Config::resolve(file, None, o)
    }

    #[test]
    fn external_staging_requires_explicit_opt_in() {
        assert!(load_str("[files]\nxfer_dir = \"/tmp/xfer\"\n", Overrides::default()).is_err());
        assert!(
            load_str(
                "[files]\nxfer_dir = \"/tmp/xfer\"\nallow_external_xfer_dir = true\n",
                Overrides::default()
            )
            .is_ok()
        );
        assert!(
            load_str(
                "",
                Overrides {
                    xfer_dir: Some("/tmp/xfer".into()),
                    ..Overrides::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn defaults_match_the_documentation() {
        let c = load_str("", Overrides::default()).unwrap();
        assert_eq!(c.listen, "127.0.0.1:8443".parse().unwrap());
        assert_eq!(c.priv_socket, Path::new("/run/opsbox/priv.sock"));
        assert_eq!(c.db_path, Path::new("/var/lib/opsbox/opsbox.db"));
        assert_eq!(c.allowed_users, ["opsbox", "root"]);
        assert!(c.allowed_uids.is_empty());
        assert_eq!(c.socket_mode, 0o660);
        assert_eq!(c.socket_group.as_deref(), Some("opsbox"));
        assert_eq!(c.log_level, "info");
        assert_eq!(c.log_format, LogFormat::Text);
        assert_eq!(c.master_key, Path::new("/etc/opsbox/master.key"));
        assert_eq!(c.secure_cookies, SecureCookiesSetting::Auto);
        assert_eq!((c.argon2_memory_kib, c.argon2_iterations), (65536, 3));
    }

    #[test]
    fn file_overrides_defaults() {
        let c = load_str(
            r#"
            [api]
            listen = "127.0.0.1:9000"
            [priv]
            socket = "/tmp/p.sock"
            allowed_users = ["svc"]
            allowed_uids = [1000]
            socket_mode = "0600"
            socket_group = ""
            [store]
            db_path = "/tmp/x.db"
            [log]
            level = "debug"
            format = "json"
            "#,
            Overrides::default(),
        )
        .unwrap();
        assert_eq!(c.listen.port(), 9000);
        assert_eq!(c.priv_socket, Path::new("/tmp/p.sock"));
        assert_eq!(c.allowed_users, ["svc"]);
        assert_eq!(c.allowed_uids, [1000]);
        assert_eq!(c.socket_mode, 0o600);
        assert_eq!(c.socket_group, None);
        assert_eq!(c.db_path, Path::new("/tmp/x.db"));
        assert_eq!(c.log_level, "debug");
        assert_eq!(c.log_format, LogFormat::Json);
    }

    #[test]
    fn flags_and_env_override_the_file() {
        let c = load_str(
            "[api]\nlisten = \"127.0.0.1:9000\"\n[log]\nlevel = \"warn\"\n",
            Overrides {
                listen: Some("127.0.0.1:7000".parse().unwrap()),
                log_level: Some("trace".into()),
                db_path: Some("/tmp/cli.db".into()),
                ..Overrides::default()
            },
        )
        .unwrap();
        assert_eq!(c.listen.port(), 7000);
        assert_eq!(c.log_level, "trace");
        assert_eq!(c.db_path, Path::new("/tmp/cli.db"));
    }

    #[test]
    fn allow_lists_are_additive() {
        let c = load_str(
            "[priv]\nallowed_users = [\"opsbox\"]\nallowed_uids = [5]\n",
            Overrides {
                allow_uids: vec![1000],
                allow_users: vec!["dev".into()],
                ..Overrides::default()
            },
        )
        .unwrap();
        assert_eq!(c.allowed_uids, [5, 1000]);
        assert_eq!(c.allowed_users, ["opsbox", "dev"]);
    }

    #[test]
    fn typos_and_bad_values_are_errors_not_silently_ignored() {
        assert!(load_str("[api]\nlisten_addr = \"x\"\n", Overrides::default()).is_err());
        assert!(load_str("[nonsense]\n", Overrides::default()).is_err());
        assert!(load_str("[api]\nlisten = \"not an address\"\n", Overrides::default()).is_err());
        assert!(load_str("[priv]\nsocket_mode = \"abc\"\n", Overrides::default()).is_err());
        assert!(load_str("[priv]\nsocket_mode = \"1777\"\n", Overrides::default()).is_err());
    }

    #[test]
    fn mode_parsing() {
        assert_eq!(parse_mode("0660").unwrap(), 0o660);
        assert_eq!(parse_mode("660").unwrap(), 0o660);
        assert_eq!(parse_mode("0666").unwrap(), 0o666);
        assert!(parse_mode("0888").is_err());
    }

    #[test]
    fn explicit_config_path_must_exist_but_the_default_may_be_absent() {
        let missing = Path::new("/nonexistent/opsbox/config.toml");
        assert!(Config::load(Some(missing), Overrides::default()).is_err());
    }

    #[test]
    fn reads_a_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.toml");
        std::fs::write(&path, "[api]\nlisten = \"127.0.0.1:1234\"\n").unwrap();
        let c = Config::load(Some(&path), Overrides::default()).unwrap();
        assert_eq!(c.listen.port(), 1234);
        assert_eq!(c.loaded_from.as_deref(), Some(path.as_path()));
    }
}
