//! Configuration: built-in defaults, overridden by `/etc/opsbox/config.toml`,
//! overridden by environment variables, overridden by command-line flags.
//!
//! clap resolves "flag over environment variable" itself, so by the time values
//! reach [`Overrides`] there is one optional value per setting. See docs/CONFIG.md.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde::Deserialize;

use crate::cli::{Cli, Command};

pub const DEFAULT_CONFIG_PATH: &str = "/etc/opsbox/config.toml";
pub const DEFAULT_LISTEN: &str = "0.0.0.0:8443";
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
    backup: BackupSection,
    proxy: ProxySection,
    watchdog: WatchdogSection,
    #[serde(rename = "priv")]
    privileged: PrivSection,
    store: StoreSection,
    update: UpdateSection,
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

/// Encrypted restic backup repository and cache settings.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct BackupSection {
    dir: PathBuf,
    cache_dir: PathBuf,
}

impl Default for BackupSection {
    fn default() -> Self {
        Self {
            dir: opsbox_priv::DEFAULT_BACKUP_DIR.into(),
            cache_dir: opsbox_priv::DEFAULT_BACKUP_CACHE_DIR.into(),
        }
    }
}

/// Reverse proxy, ACME, and public IP settings.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ProxySection {
    listen_http: SocketAddr,
    listen_https: SocketAddr,
    fallback_http: u16,
    fallback_https: u16,
    socket: PathBuf,
    state_dir: PathBuf,
    acme_staging: bool,
    acme_directory: Option<String>,
    acme_email: Option<String>,
    public_ips: Vec<IpAddr>,
}

impl Default for ProxySection {
    fn default() -> Self {
        Self {
            listen_http: opsbox_proxy::DEFAULT_HTTP_ADDR
                .parse()
                .expect("valid default"),
            listen_https: opsbox_proxy::DEFAULT_HTTPS_ADDR
                .parse()
                .expect("valid default"),
            fallback_http: 8080,
            fallback_https: 8445,
            socket: opsbox_proxy::DEFAULT_SOCKET_PATH.into(),
            state_dir: opsbox_proxy::DEFAULT_STATE_DIR.into(),
            acme_staging: false,
            acme_directory: None,
            acme_email: None,
            public_ips: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct WatchdogSection {
    listen: Option<SocketAddr>,
    runtime_dir: PathBuf,
}

impl Default for WatchdogSection {
    fn default() -> Self {
        Self {
            listen: None,
            runtime_dir: "/run/opsbox-watchdog".into(),
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

/// Self-update settings. `install.sh --channel beta` writes `channel = "beta"` here.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct UpdateSection {
    channel: Option<String>,
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
    pub backup_dir: PathBuf,
    pub backup_cache_dir: PathBuf,
    pub proxy_listen_http: SocketAddr,
    pub proxy_listen_https: SocketAddr,
    pub proxy_fallback_http: u16,
    pub proxy_fallback_https: u16,
    pub proxy_socket: PathBuf,
    pub proxy_state_dir: PathBuf,
    pub acme_staging: bool,
    pub acme_directory: Option<String>,
    pub acme_email: Option<String>,
    pub public_ips: Vec<IpAddr>,
    pub watchdog_listen: SocketAddr,
    pub watchdog_runtime_dir: PathBuf,
    pub db_path: PathBuf,
    /// `[update] channel`, if the file names one (the panel setting still wins).
    pub update_channel: Option<opsbox_update::Channel>,
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
    pub proxy_listen_http: Option<SocketAddr>,
    pub proxy_listen_https: Option<SocketAddr>,
    pub proxy_socket: Option<PathBuf>,
    pub proxy_state_dir: Option<PathBuf>,
    pub acme_staging: bool,
    pub watchdog_listen: Option<SocketAddr>,
    pub watchdog_runtime_dir: Option<PathBuf>,
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
                o.proxy_socket.clone_from(&a.proxy_socket);
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
            Command::Proxy(a) => {
                o.proxy_listen_http = a.http;
                o.proxy_listen_https = a.https;
                o.proxy_socket.clone_from(&a.proxy_socket);
                o.proxy_state_dir.clone_from(&a.state_dir);
                o.priv_socket.clone_from(&a.socket);
                o.acme_staging = a.staging;
            }
            Command::Status(a) => {
                o.listen = a.listen;
                o.priv_socket.clone_from(&a.socket);
                o.proxy_socket.clone_from(&a.proxy_socket);
            }
            Command::Watchdog(a) => {
                o.watchdog_listen = a.listen;
                o.watchdog_runtime_dir.clone_from(&a.runtime_dir);
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
        if file.proxy.fallback_http == 0 || file.proxy.fallback_https == 0 {
            bail!("proxy fallback ports must be between 1 and 65535");
        }
        let listen = o.listen.unwrap_or(file.api.listen);
        let watchdog_listen = o
            .watchdog_listen
            .or(file.watchdog.listen)
            .unwrap_or_else(|| SocketAddr::new(listen.ip(), 8444));
        let update_channel = match file.update.channel.as_deref() {
            Some(c) => Some(opsbox_update::Channel::parse(c).with_context(|| {
                format!("update.channel {c:?} is not a channel (use \"stable\" or \"beta\")")
            })?),
            None => None,
        };
        Ok(Self {
            listen,
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
            backup_dir: file.backup.dir,
            backup_cache_dir: file.backup.cache_dir,
            proxy_listen_http: o.proxy_listen_http.unwrap_or(file.proxy.listen_http),
            proxy_listen_https: o.proxy_listen_https.unwrap_or(file.proxy.listen_https),
            proxy_fallback_http: file.proxy.fallback_http,
            proxy_fallback_https: file.proxy.fallback_https,
            proxy_socket: o.proxy_socket.unwrap_or(file.proxy.socket),
            proxy_state_dir: o.proxy_state_dir.unwrap_or(file.proxy.state_dir),
            acme_staging: o.acme_staging || file.proxy.acme_staging,
            acme_directory: file.proxy.acme_directory,
            acme_email: file.proxy.acme_email,
            public_ips: file.proxy.public_ips,
            watchdog_listen,
            watchdog_runtime_dir: o.watchdog_runtime_dir.unwrap_or(file.watchdog.runtime_dir),
            db_path: o.db_path.unwrap_or(file.store.db_path),
            update_channel,
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
        assert_eq!(c.listen, "0.0.0.0:8443".parse().unwrap());
        assert_eq!(c.priv_socket, Path::new("/run/opsbox/priv.sock"));
        assert_eq!(c.db_path, Path::new("/var/lib/opsbox/opsbox.db"));
        assert_eq!(c.allowed_users, ["opsbox", "root"]);
        assert!(c.allowed_uids.is_empty());
        assert_eq!(c.socket_mode, 0o660);
        assert_eq!(c.socket_group.as_deref(), Some("opsbox"));
        assert_eq!(c.log_level, "info");
        assert_eq!(c.log_format, LogFormat::Text);
        assert_eq!(c.master_key, Path::new("/etc/opsbox/master.key"));
        assert_eq!(c.backup_dir, Path::new("/var/backups/opsbox"));
        assert_eq!(c.backup_cache_dir, Path::new("/var/cache/opsbox/restic"));
        assert_eq!(c.secure_cookies, SecureCookiesSetting::Auto);
        assert_eq!((c.argon2_memory_kib, c.argon2_iterations), (65536, 3));
    }

    #[test]
    fn update_channel_comes_from_the_file() {
        assert_eq!(
            load_str("", Overrides::default()).unwrap().update_channel,
            None
        );
        let c = load_str("[update]\nchannel = \"beta\"\n", Overrides::default()).unwrap();
        assert_eq!(c.update_channel, Some(opsbox_update::Channel::Beta));
        assert!(load_str("[update]\nchannel = \"nightly\"\n", Overrides::default()).is_err());
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

    #[test]
    fn proxy_config_file_and_overrides() {
        let c = load_str(
            r#"
            [proxy]
            listen_http = "127.0.0.1:8080"
            listen_https = "127.0.0.1:8444"
            socket = "/tmp/proxy.sock"
            state_dir = "/tmp/proxy-state"
            acme_staging = true
            acme_directory = "https://acme-staging-v02.api.letsencrypt.org/directory"
            acme_email = "ops@example.com"
            public_ips = ["203.0.113.10", "2001:db8::10"]
            "#,
            Overrides::default(),
        )
        .unwrap();
        assert_eq!(c.proxy_listen_http.port(), 8080);
        assert_eq!(c.proxy_listen_https.port(), 8444);
        assert_eq!(c.proxy_socket, Path::new("/tmp/proxy.sock"));
        assert_eq!(c.proxy_state_dir, Path::new("/tmp/proxy-state"));
        assert!(c.acme_staging);
        assert_eq!(
            c.acme_directory.as_deref(),
            Some("https://acme-staging-v02.api.letsencrypt.org/directory")
        );
        assert_eq!(c.acme_email.as_deref(), Some("ops@example.com"));
        assert_eq!(c.public_ips.len(), 2);

        let c2 = load_str(
            "[proxy]\nlisten_http = \"127.0.0.1:8080\"\nacme_staging = false\n",
            Overrides {
                proxy_listen_http: Some("127.0.0.1:9080".parse().unwrap()),
                proxy_socket: Some("/tmp/cli-proxy.sock".into()),
                acme_staging: true,
                ..Overrides::default()
            },
        )
        .unwrap();
        assert_eq!(c2.proxy_listen_http.port(), 9080);
        assert_eq!(c2.proxy_socket, Path::new("/tmp/cli-proxy.sock"));
        assert!(c2.acme_staging);
    }

    #[test]
    fn watchdog_config_follows_api_ip_and_allows_override() {
        let public = load_str("[api]\nlisten = \"0.0.0.0:8443\"\n", Overrides::default()).unwrap();
        assert_eq!(public.watchdog_listen, "0.0.0.0:8444".parse().unwrap());

        let local = load_str("[api]\nlisten = \"127.0.0.1:8443\"\n", Overrides::default()).unwrap();
        assert_eq!(local.watchdog_listen, "127.0.0.1:8444".parse().unwrap());

        let custom = load_str(
            "[watchdog]\nlisten = \"127.0.0.1:9444\"\nruntime_dir = \"/tmp/wd\"\n",
            Overrides::default(),
        )
        .unwrap();
        assert_eq!(custom.watchdog_listen, "127.0.0.1:9444".parse().unwrap());
        assert_eq!(custom.watchdog_runtime_dir, Path::new("/tmp/wd"));
    }
}
