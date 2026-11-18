//! Privileged backup engine: restic invocation, password derivation, database dump
//! streaming, Docker volume snapshots, opsbox self-backup (SQLite online backup +
//! `/etc/opsbox` excluding `master.key`), offsite destination tests, restore, and
//! automated restore verification (docs/BACKEND_SPEC.md 7.12).

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use opsbox_backup::BackupError;
use opsbox_backup::destinations::{
    s3_restic_repo_uri, sftp_restic_repo_uri, validate_sftp_host, validate_sftp_path,
    validate_sftp_user,
};
use opsbox_backup::dumps::{
    DatabaseRestoreMethod, build_dump_spec, build_restore_spec, build_sanity_check_spec,
};
use opsbox_backup::restic::{
    backup_paths_args, backup_stdin_args, cat_config_args, check_args, date_suffix_utc, dump_args,
    forget_args, forget_single_snapshot_args, init_args, job_tags, parse_backup_summary,
    parse_check_output, parse_forget_output, parse_snapshots, parse_version, restore_args,
    snapshots_args,
};
use opsbox_backup::validation::{
    default_restored_path, default_restored_volume, validate_abs_path, validate_path_list,
    validate_read_data_subset, validate_restore_path, validate_retention, validate_volume_list,
};
use opsbox_deploy::generate_ed25519_deploy_key;
use opsbox_deploy::pipeline::scrub_secrets;
use opsbox_deploy::templates::get_template;
use opsbox_docker::{PortBindMode, TemplateContainerSpec};
use opsbox_proto::{
    BackupCheckRequest, BackupCheckResult, BackupCheckState, BackupDbKind,
    BackupDestinationTestRequest, BackupDestinationTestResult, BackupDestinationTestTarget,
    BackupForgetRequest, BackupForgetResult, BackupGenerateSftpKeyRequest, BackupLocalInfo,
    BackupRecoveryKeyResult, BackupRepoTarget, BackupRestoreMode, BackupRestoreRequest,
    BackupRestoreResult, BackupRestoreTarget, BackupRestoreTestRequest, BackupRestoreTestResult,
    BackupRestoreTestTarget, BackupRunResult, BackupRunSpec, BackupRunTarget, BackupS3Credentials,
    BackupSftpKeyResult, BackupSnapshotsRequest, BackupTargetKind, BackupTestStep,
    BackupTestStepStatus, DockerContainerAction, DockerExecRequest, PrivError, PrivErrorCode,
    ResticSnapshotInfo, Secret, valid_backup_id, valid_docker_ref, valid_service_name,
};
use opsbox_store::ReadOnlyStore;
use tokio::io::AsyncReadExt as _;
use tokio::process::Command;
use zeroize::Zeroizing;

use super::Env;
use super::deploy::{err, find_binary, open_deploy_vars};
use super::docker::{block_on_fut, map_docker_err};
use super::sealed;
use super::templates_compose::wait_for_template_health;

const RESTIC_VERSION_TIMEOUT: Duration = Duration::from_secs(15);
const RESTIC_META_TIMEOUT: Duration = Duration::from_secs(120);
const RESTIC_RUN_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const DUMP_STREAM_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_RESTIC_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_SUBSET: &str = "10%";

fn map_backup_err(e: BackupError) -> PrivError {
    match e {
        BackupError::Validation(msg) => err(PrivErrorCode::Invalid, msg),
        BackupError::Internal(msg) => err(PrivErrorCode::Internal, msg),
    }
}

fn derive_pw(secrets: &crate::secrets::Secrets) -> Result<Zeroizing<String>, PrivError> {
    secrets.derive_backup_password().map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Could not derive repository password: {e}"),
        )
    })
}

fn repo_lock(repo_uri: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
        OnceLock::new();
    let map = LOCKS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = map
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Arc::clone(
        guard
            .entry(repo_uri.to_owned())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
    )
}

fn now_unix() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    )
    .unwrap_or(0)
}

fn ensure_private_dir(path: &Path) -> Result<(), PrivError> {
    if let Err(e) = fs::create_dir_all(path) {
        return Err(err(
            PrivErrorCode::Internal,
            format!("Could not prepare directory '{}': {e}", path.display()),
        ));
    }
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
    Ok(())
}

fn find_restic_bin(env: &Env) -> Result<PathBuf, PrivError> {
    find_binary(
        env.restic_bin.as_deref(),
        &["restic"],
        &[
            "/usr/local/bin/restic",
            "/usr/bin/restic",
            "/opt/homebrew/bin/restic",
        ],
    )
    .ok_or_else(|| {
        err(
            PrivErrorCode::Unsupported,
            "restic is not installed on this server (expected /usr/local/bin/restic).",
        )
    })
}

/// Ephemeral `0700` staging directory containing a `0600` SFTP private key file.
struct SshKeyFileGuard {
    _dir: tempfile::TempDir,
    key_path: PathBuf,
    secret: Zeroizing<String>,
}

impl SshKeyFileGuard {
    fn create(cache_dir: &Path, private_key_pem: Zeroizing<String>) -> Result<Self, PrivError> {
        ensure_private_dir(cache_dir)?;
        let dir = tempfile::Builder::new()
            .prefix("opsbox-ssh-")
            .tempdir_in(cache_dir)
            .map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Could not create SSH key staging directory: {e}"),
                )
            })?;
        let _ = fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700));
        let key_path = dir.path().join("id_ed25519");
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&key_path)
            .map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Could not create ephemeral SSH key file: {e}"),
                )
            })?;
        file.write_all(private_key_pem.as_bytes()).map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Could not write ephemeral SSH key file: {e}"),
            )
        })?;
        file.flush().map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Could not flush ephemeral SSH key file: {e}"),
            )
        })?;
        Ok(Self {
            _dir: dir,
            key_path,
            secret: private_key_pem,
        })
    }
}

impl Drop for SshKeyFileGuard {
    fn drop(&mut self) {
        if let Ok(mut f) = OpenOptions::new().write(true).open(&self.key_path) {
            let zeros = vec![0u8; self.secret.len().max(256)];
            let _ = f.write_all(&zeros);
            let _ = f.flush();
        }
        let _ = fs::remove_file(&self.key_path);
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SealedS3SecretPayload {
    access_key_id: String,
    secret_access_key: String,
}

struct BackendConn {
    lock_key: String,
    repo_uri: String,
    local_dir: Option<PathBuf>,
    extra_opts: Vec<String>,
    envs: Vec<(String, Zeroizing<String>)>,
    scrub: Vec<String>,
    _ssh_guard: Option<SshKeyFileGuard>,
}

impl BackendConn {
    fn extra_env_pairs(&self) -> Vec<(&str, &str)> {
        self.envs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }

    fn append_scrub_strings(&self, out: &mut Vec<String>) {
        out.extend(self.scrub.iter().cloned());
    }
}

fn validate_dest_seal_context(destination_id: &str, ctx: &str) -> Result<(), PrivError> {
    if !valid_backup_id(destination_id) {
        return Err(err(
            PrivErrorCode::Invalid,
            "Invalid backup destination identifier.",
        ));
    }
    let expected = format!("backup:dest:{destination_id}");
    if ctx != expected {
        return Err(err(
            PrivErrorCode::Refused,
            "Backup secret context does not match destination ID.",
        ));
    }
    Ok(())
}

fn resolve_backend_conn(
    env: &Env,
    destination_id: &str,
    repo: &BackupRepoTarget,
) -> Result<BackendConn, PrivError> {
    if !valid_backup_id(destination_id) {
        return Err(err(
            PrivErrorCode::Invalid,
            "Invalid backup destination identifier.",
        ));
    }
    match repo {
        BackupRepoTarget::Local => {
            if destination_id != "local" {
                return Err(err(
                    PrivErrorCode::Invalid,
                    "Missing repository target configuration for remote backup destination.",
                ));
            }
            let repo_str = env.backup_dir.to_string_lossy().into_owned();
            Ok(BackendConn {
                lock_key: repo_str.clone(),
                repo_uri: repo_str,
                local_dir: Some(env.backup_dir.clone()),
                extra_opts: Vec::new(),
                envs: Vec::new(),
                scrub: Vec::new(),
                _ssh_guard: None,
            })
        }
        BackupRepoTarget::S3 {
            destination_id: repo_dest_id,
            endpoint,
            bucket,
            repo_path,
            region,
            credentials,
        } => {
            if repo_dest_id != destination_id {
                return Err(err(
                    PrivErrorCode::Invalid,
                    "Destination ID mismatch in S3 repository target.",
                ));
            }
            let (ak, sk) = match credentials {
                BackupS3Credentials::Plaintext {
                    access_key_id,
                    secret_access_key,
                } => (
                    Zeroizing::new(access_key_id.expose().to_owned()),
                    Zeroizing::new(secret_access_key.expose().to_owned()),
                ),
                BackupS3Credentials::Sealed {
                    seal_context,
                    value_enc,
                } => {
                    validate_dest_seal_context(destination_id, seal_context)?;
                    let opened = sealed::open_secret(&env.secrets, seal_context, value_enc)?;
                    let parsed: SealedS3SecretPayload = serde_json::from_str(opened.expose())
                        .map_err(|_| {
                            err(
                                PrivErrorCode::Invalid,
                                "Sealed S3 destination credentials are malformed.",
                            )
                        })?;
                    (
                        Zeroizing::new(parsed.access_key_id),
                        Zeroizing::new(parsed.secret_access_key),
                    )
                }
            };
            if ak.trim().is_empty() || sk.trim().is_empty() {
                return Err(err(
                    PrivErrorCode::Invalid,
                    "S3 access key and secret key cannot be empty.",
                ));
            }
            let uri = s3_restic_repo_uri(endpoint, bucket, repo_path);
            let mut envs = vec![
                ("AWS_ACCESS_KEY_ID".to_owned(), ak.clone()),
                ("AWS_SECRET_ACCESS_KEY".to_owned(), sk.clone()),
            ];
            if let Some(reg) = region.as_deref().filter(|s| !s.trim().is_empty()) {
                envs.push((
                    "AWS_DEFAULT_REGION".to_owned(),
                    Zeroizing::new(reg.to_owned()),
                ));
            }
            let scrub = vec![ak.to_string(), sk.to_string()];
            Ok(BackendConn {
                lock_key: format!("dest:{destination_id}:{uri}"),
                repo_uri: uri,
                local_dir: None,
                extra_opts: Vec::new(),
                envs,
                scrub,
                _ssh_guard: None,
            })
        }
        BackupRepoTarget::Sftp {
            destination_id: repo_dest_id,
            host,
            port,
            user,
            repo_path,
            seal_context,
            private_key_enc,
        } => {
            if repo_dest_id != destination_id {
                return Err(err(
                    PrivErrorCode::Invalid,
                    "Destination ID mismatch in SFTP repository target.",
                ));
            }
            let clean_host = validate_sftp_host(host).map_err(map_backup_err)?;
            let clean_user = validate_sftp_user(user).map_err(map_backup_err)?;
            let clean_path = validate_sftp_path(repo_path).map_err(map_backup_err)?;
            if *port == 0 {
                return Err(err(PrivErrorCode::Invalid, "Invalid SFTP port."));
            }
            validate_dest_seal_context(destination_id, seal_context)?;
            let pem = sealed::open_secret(&env.secrets, seal_context, private_key_enc)?;
            let guard = SshKeyFileGuard::create(
                &env.backup_cache_dir,
                Zeroizing::new(pem.expose().to_owned()),
            )?;
            let known_hosts = env
                .backup_cache_dir
                .join(format!("known_hosts_{destination_id}"));
            let key_str = guard.key_path.to_string_lossy();
            let kh_str = known_hosts.to_string_lossy();
            if key_str.contains('\'')
                || key_str.contains(char::is_whitespace)
                || kh_str.contains('\'')
                || kh_str.contains(char::is_whitespace)
            {
                return Err(err(
                    PrivErrorCode::Invalid,
                    "Cache path cannot contain whitespace or quotes for SFTP transport.",
                ));
            }
            let ssh_cmd = format!(
                "ssh -i {key_str} -p {port} -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile={kh_str} -o ConnectTimeout=15 {clean_user}@{clean_host} -s sftp"
            );
            let uri = sftp_restic_repo_uri(&clean_user, &clean_host, &clean_path);
            let scrub = vec![pem.expose().to_owned()];
            Ok(BackendConn {
                lock_key: format!("dest:{destination_id}:{uri}"),
                repo_uri: uri,
                local_dir: None,
                extra_opts: vec![format!("sftp.command={ssh_cmd}")],
                envs: Vec::new(),
                scrub,
                _ssh_guard: Some(guard),
            })
        }
    }
}

/// Ephemeral `0700` staging directory containing a `0600` restic password file.
struct PasswordFileGuard {
    _dir: tempfile::TempDir,
    path: PathBuf,
    secret: Zeroizing<String>,
}

impl PasswordFileGuard {
    fn create(cache_dir: &Path, secret: Zeroizing<String>) -> Result<Self, PrivError> {
        ensure_private_dir(cache_dir)?;
        let dir = tempfile::Builder::new()
            .prefix("opsbox-pw-")
            .tempdir_in(cache_dir)
            .map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Could not create password staging directory: {e}"),
                )
            })?;
        let _ = fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700));
        let path = dir.path().join("repo-password");
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Could not create ephemeral password file: {e}"),
                )
            })?;
        file.write_all(secret.as_bytes()).map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Could not write ephemeral password file: {e}"),
            )
        })?;
        file.flush().map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Could not flush ephemeral password file: {e}"),
            )
        })?;
        Ok(Self {
            _dir: dir,
            path,
            secret,
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn secret(&self) -> &str {
        self.secret.as_str()
    }
}

impl Drop for PasswordFileGuard {
    fn drop(&mut self) {
        if let Ok(mut f) = OpenOptions::new().write(true).open(&self.path) {
            let zeros = vec![0u8; self.secret.len().max(64)];
            let _ = f.write_all(&zeros);
            let _ = f.flush();
        }
        let _ = fs::remove_file(&self.path);
    }
}

struct ResticCmdOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

async fn read_capped<R: tokio::io::AsyncRead + Unpin>(mut reader: R) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    while let Ok(n) = reader.read(&mut chunk).await {
        if n == 0 {
            break;
        }
        let keep = n.min(MAX_RESTIC_OUTPUT_BYTES.saturating_sub(buf.len()));
        buf.extend_from_slice(&chunk[..keep]);
        if buf.len() >= MAX_RESTIC_OUTPUT_BYTES {
            let mut drain = [0u8; 4096];
            while let Ok(dn) = reader.read(&mut drain).await {
                if dn == 0 {
                    break;
                }
            }
            break;
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn clean_restic_command(
    restic_bin: &Path,
    cache_dir: &Path,
    args: &[String],
    extra_env: &[(&str, &str)],
) -> Command {
    #[cfg(target_os = "linux")]
    let mut cmd = {
        let mut c = if Path::new("/run/systemd/system").exists()
            && Path::new("/usr/bin/systemd-run").exists()
        {
            let mut c = Command::new("/usr/bin/systemd-run");
            c.args([
                "--scope",
                "--quiet",
                "--property=MemoryMax=256M",
                "--property=MemorySwapMax=0",
                "--",
                "/usr/bin/ionice",
            ]);
            c
        } else {
            Command::new("/usr/bin/ionice")
        };
        c.args(["-c", "3", "/usr/bin/nice", "-n", "19"])
            .arg(restic_bin);
        c
    };
    #[cfg(not(target_os = "linux"))]
    let mut cmd = Command::new(restic_bin);
    cmd.args(args)
        .env_clear()
        .env(
            "PATH",
            "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        )
        .env("LC_ALL", "C")
        .env("HOME", cache_dir)
        .env("RESTIC_CACHE_DIR", cache_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    cmd
}

async fn run_restic(
    restic_bin: &Path,
    cache_dir: &Path,
    current_dir: Option<&Path>,
    args: &[String],
    extra_env: &[(&str, &str)],
    timeout: Duration,
    scrub: &[&str],
) -> Result<ResticCmdOutput, PrivError> {
    let mut cmd = clean_restic_command(restic_bin, cache_dir, args, extra_env);
    if let Some(cwd) = current_dir {
        cmd.current_dir(cwd);
    }
    let mut child = cmd.spawn().map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Failed to start restic: {e}"),
        )
    })?;
    let out_pipe = child
        .stdout
        .take()
        .ok_or_else(|| err(PrivErrorCode::Internal, "Missing restic stdout pipe."))?;
    let err_pipe = child
        .stderr
        .take()
        .ok_or_else(|| err(PrivErrorCode::Internal, "Missing restic stderr pipe."))?;

    let wait_work = async {
        let (stdout, stderr, status) =
            tokio::join!(read_capped(out_pipe), read_capped(err_pipe), child.wait(),);
        let status = status.map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Failed waiting for restic: {e}"),
            )
        })?;
        Ok::<ResticCmdOutput, PrivError>(ResticCmdOutput {
            code: status.code(),
            stdout: scrub_secrets(&stdout, scrub),
            stderr: scrub_secrets(&stderr, scrub),
        })
    };

    tokio::time::timeout(timeout, wait_work)
        .await
        .map_err(|_| err(PrivErrorCode::Internal, "restic command timed out."))?
}

async fn run_restic_with_stdin(
    restic_bin: &Path,
    cache_dir: &Path,
    args: &[String],
    extra_env: &[(&str, &str)],
    stdin_bytes: &[u8],
    timeout: Duration,
    scrub: &[&str],
) -> Result<ResticCmdOutput, PrivError> {
    use tokio::io::AsyncWriteExt as _;

    let mut cmd = clean_restic_command(restic_bin, cache_dir, args, extra_env);
    cmd.stdin(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Failed to start restic: {e}"),
        )
    })?;
    let mut in_pipe = child
        .stdin
        .take()
        .ok_or_else(|| err(PrivErrorCode::Internal, "Missing restic stdin pipe."))?;
    let out_pipe = child
        .stdout
        .take()
        .ok_or_else(|| err(PrivErrorCode::Internal, "Missing restic stdout pipe."))?;
    let err_pipe = child
        .stderr
        .take()
        .ok_or_else(|| err(PrivErrorCode::Internal, "Missing restic stderr pipe."))?;

    let write_work = async move {
        let _ = in_pipe.write_all(stdin_bytes).await;
        let _ = in_pipe.shutdown().await;
    };

    let wait_work = async {
        let ((), stdout, stderr, status) = tokio::join!(
            write_work,
            read_capped(out_pipe),
            read_capped(err_pipe),
            child.wait(),
        );
        let status = status.map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Failed waiting for restic: {e}"),
            )
        })?;
        Ok::<ResticCmdOutput, PrivError>(ResticCmdOutput {
            code: status.code(),
            stdout: scrub_secrets(&stdout, scrub),
            stderr: scrub_secrets(&stderr, scrub),
        })
    };

    tokio::time::timeout(timeout, wait_work)
        .await
        .map_err(|_| err(PrivErrorCode::Internal, "restic command timed out."))?
}

fn restic_failure_msg(action: &str, out: &ResticCmdOutput, scrub: &[&str]) -> String {
    let detail = if out.stderr.trim().is_empty() {
        out.stdout.trim()
    } else {
        out.stderr.trim()
    };
    let clean = scrub_secrets(detail, scrub);
    if clean.is_empty() {
        format!("{action} failed (exit code {:?}).", out.code)
    } else {
        format!("{action} failed: {clean}")
    }
}

fn build_scrub_refs<'a>(pw: &'a PasswordFileGuard, backend: &'a BackendConn) -> Vec<&'a str> {
    let mut refs = vec![pw.secret()];
    for s in &backend.scrub {
        refs.push(s.as_str());
    }
    refs
}

fn is_missing_repo_err(out: &ResticCmdOutput) -> bool {
    let lower = format!("{}\n{}", out.stderr, out.stdout).to_ascii_lowercase();
    lower.contains("is there a repository")
        || lower.contains("unable to open config file")
        || lower.contains("does not exist")
        || lower.contains("no such file")
}

async fn ensure_repo_initialized(
    restic_bin: &Path,
    backend: &BackendConn,
    cache_dir: &Path,
    pw: &PasswordFileGuard,
) -> Result<bool, PrivError> {
    let envs = backend.extra_env_pairs();
    let scrub = build_scrub_refs(pw, backend);

    if let Some(ref local_dir) = backend.local_dir {
        ensure_private_dir(local_dir)?;
        if local_dir.join("config").is_file() {
            return Ok(false);
        }
    } else {
        let cat = cat_config_args(&backend.repo_uri, pw.path(), &backend.extra_opts);
        let cat_out = run_restic(
            restic_bin,
            cache_dir,
            None,
            &cat,
            &envs,
            RESTIC_META_TIMEOUT,
            &scrub,
        )
        .await?;
        if cat_out.code == Some(0) {
            return Ok(false);
        }
    }

    let args = init_args(&backend.repo_uri, pw.path(), &backend.extra_opts);
    let out = run_restic(
        restic_bin,
        cache_dir,
        None,
        &args,
        &envs,
        RESTIC_META_TIMEOUT,
        &scrub,
    )
    .await?;
    if out.code != Some(0) {
        if let Some(ref local_dir) = backend.local_dir
            && local_dir.join("config").is_file()
        {
            return Ok(false);
        }
        return Err(err(
            PrivErrorCode::Internal,
            restic_failure_msg("Initializing restic repository", &out, &scrub),
        ));
    }
    Ok(true)
}

fn path_canonical_or_logical(p: &Path) -> PathBuf {
    if let Ok(c) = fs::canonicalize(p) {
        return c;
    }
    let mut existing = PathBuf::new();
    let mut rest = Vec::new();
    let mut cur = p.to_path_buf();
    loop {
        if cur.exists() {
            existing = fs::canonicalize(&cur).unwrap_or(cur);
            break;
        }
        if let Some(name) = cur.file_name() {
            rest.push(name.to_os_string());
        }
        if !cur.pop() {
            break;
        }
    }
    for part in rest.into_iter().rev() {
        existing.push(part);
    }
    existing
}

fn validate_live_path(raw: &str, env: &Env) -> Result<PathBuf, PrivError> {
    let clean = validate_abs_path(raw).map_err(map_backup_err)?;
    let p = Path::new(&clean);
    if !p.exists() {
        return Err(err(
            PrivErrorCode::NotFound,
            format!("Backup path \"{clean}\" does not exist."),
        ));
    }
    let canon = fs::canonicalize(p).map_err(|e| {
        err(
            PrivErrorCode::Invalid,
            format!("Could not resolve backup path \"{clean}\": {e}"),
        )
    })?;
    let canon_str = canon.to_string_lossy();
    if canon == Path::new("/")
        || canon == Path::new("/proc")
        || canon.starts_with("/proc/")
        || canon == Path::new("/sys")
        || canon.starts_with("/sys/")
        || canon == Path::new("/dev")
        || canon.starts_with("/dev/")
    {
        return Err(err(
            PrivErrorCode::Invalid,
            format!("Path \"{clean}\" resolves to a restricted system directory."),
        ));
    }
    let master_canon = path_canonical_or_logical(&env.master_key);
    if canon == master_canon || canon.starts_with(&master_canon) {
        return Err(err(
            PrivErrorCode::Invalid,
            "The opsbox master key cannot be backed up.",
        ));
    }
    let backup_canon = path_canonical_or_logical(&env.backup_dir);
    if canon == backup_canon || canon.starts_with(&backup_canon) {
        return Err(err(
            PrivErrorCode::Invalid,
            "The local backup repository cannot be included as a backup source.",
        ));
    }
    let _ = canon_str;
    Ok(canon)
}

fn backup_excludes(env: &Env) -> Vec<String> {
    let mut raw_list = vec![
        env.master_key.to_string_lossy().into_owned(),
        path_canonical_or_logical(&env.master_key)
            .to_string_lossy()
            .into_owned(),
        env.backup_dir.to_string_lossy().into_owned(),
        path_canonical_or_logical(&env.backup_dir)
            .to_string_lossy()
            .into_owned(),
    ];
    let mut dedup = Vec::new();
    for item in raw_list.drain(..) {
        if !item.is_empty() && !dedup.contains(&item) {
            dedup.push(item);
        }
    }
    dedup
}

fn copy_etc_dir_excluding_master_key(
    src_dir: &Path,
    dst_dir: &Path,
    master_canon: &Path,
    backup_canon: &Path,
    copied_files: &mut usize,
) -> Result<(), PrivError> {
    ensure_private_dir(dst_dir)?;
    let entries = fs::read_dir(src_dir).map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Could not read '{}': {e}", src_dir.display()),
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Could not iterate '{}': {e}", src_dir.display()),
            )
        })?;
        let name = entry.file_name();
        if name == "master.key" {
            continue;
        }
        let src_path = entry.path();
        if path_canonical_or_logical(&src_path) == master_canon
            || path_canonical_or_logical(&src_path).starts_with(backup_canon)
        {
            continue;
        }
        let meta = fs::symlink_metadata(&src_path).map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Could not inspect '{}': {e}", src_path.display()),
            )
        })?;
        if meta.file_type().is_symlink() {
            // Never follow symlinks inside /etc/opsbox so master.key cannot be aliased in.
            continue;
        }
        let dst_path = dst_dir.join(&name);
        if meta.file_type().is_dir() {
            copy_etc_dir_excluding_master_key(
                &src_path,
                &dst_path,
                master_canon,
                backup_canon,
                copied_files,
            )?;
        } else if meta.file_type().is_file() {
            fs::copy(&src_path, &dst_path).map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Could not stage '{}': {e}", src_path.display()),
                )
            })?;
            let mode = meta.permissions().mode() & 0o777;
            let _ = fs::set_permissions(&dst_path, fs::Permissions::from_mode(mode));
            *copied_files += 1;
        }
    }
    Ok(())
}

fn contains_master_key_file(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        if entry.file_name() == "master.key" {
            return true;
        }
        let p = entry.path();
        if let Ok(meta) = fs::symlink_metadata(&p)
            && meta.file_type().is_dir()
            && contains_master_key_file(&p)
        {
            return true;
        }
    }
    false
}

fn stage_opsbox_self(
    db_path: &Path,
    config_path: &Path,
    master_key: &Path,
    backup_dir: &Path,
    backup_cache_dir: &Path,
) -> Result<(tempfile::TempDir, Vec<String>), PrivError> {
    ensure_private_dir(backup_cache_dir)?;
    let stage = tempfile::Builder::new()
        .prefix("opsbox-self-")
        .tempdir_in(backup_cache_dir)
        .map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Could not create self-backup staging directory: {e}"),
            )
        })?;
    let _ = fs::set_permissions(stage.path(), fs::Permissions::from_mode(0o700));

    let mut targets = Vec::new();
    if db_path.is_file() {
        let ro = ReadOnlyStore::open(db_path).map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Could not open database for hot backup: {e}"),
            )
        })?;
        let dst_db = stage.path().join("opsbox.db");
        ro.online_backup(&dst_db).map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("SQLite online backup failed: {e}"),
            )
        })?;
        let _ = fs::set_permissions(&dst_db, fs::Permissions::from_mode(0o600));
        targets.push("opsbox.db".to_owned());
    }

    let etc_dst = stage.path().join("etc-opsbox");
    let master_canon = path_canonical_or_logical(master_key);
    let backup_canon = path_canonical_or_logical(backup_dir);
    let mut copied_etc = 0usize;
    if let Some(etc_src) = config_path.parent().filter(|p| p.is_dir()) {
        copy_etc_dir_excluding_master_key(
            etc_src,
            &etc_dst,
            &master_canon,
            &backup_canon,
            &mut copied_etc,
        )?;
    } else if config_path.is_file()
        && config_path.file_name().and_then(|s| s.to_str()) != Some("master.key")
    {
        ensure_private_dir(&etc_dst)?;
        let dst_cfg = etc_dst.join("config.toml");
        fs::copy(config_path, &dst_cfg).map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Could not stage config.toml: {e}"),
            )
        })?;
        copied_etc += 1;
    }

    if copied_etc > 0 || etc_dst.is_dir() {
        targets.push("etc-opsbox".to_owned());
    }

    if contains_master_key_file(stage.path()) {
        return Err(err(
            PrivErrorCode::Internal,
            "Refusing self-backup because master.key was unexpectedly staged.",
        ));
    }
    if targets.is_empty() {
        return Err(err(
            PrivErrorCode::NotFound,
            "Neither opsbox.db nor /etc/opsbox files exist to back up.",
        ));
    }
    Ok((stage, targets))
}

async fn discard_partial_snapshot(
    restic_bin: &Path,
    backend: &BackendConn,
    cache_dir: &Path,
    pw: &PasswordFileGuard,
    stdout: &str,
) {
    if let Ok(sum) = parse_backup_summary(stdout) {
        let args = forget_single_snapshot_args(
            &backend.repo_uri,
            pw.path(),
            &backend.extra_opts,
            &sum.snapshot_id,
        );
        let envs = backend.extra_env_pairs();
        let scrub = build_scrub_refs(pw, backend);
        let _ = run_restic(
            restic_bin,
            cache_dir,
            None,
            &args,
            &envs,
            RESTIC_META_TIMEOUT,
            &scrub,
        )
        .await;
    }
}

fn existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut cur = path.to_path_buf();
    loop {
        if cur.exists() {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_mount_info(target: &Path) -> (Option<String>, Option<String>) {
    let Ok(mounts) = fs::read_to_string("/proc/self/mounts") else {
        return (None, None);
    };
    let canon = path_canonical_or_logical(target);
    let canon_str = canon.to_string_lossy();
    let mut best: Option<(usize, String, String)> = None;
    for line in mounts.lines() {
        let mut parts = line.split_whitespace();
        let _src = parts.next();
        let Some(mp_raw) = parts.next() else {
            continue;
        };
        let Some(fstype) = parts.next() else {
            continue;
        };
        let mp = mp_raw.replace("\\040", " ");
        let matches = mp == "/"
            || canon_str == mp
            || canon_str
                .strip_prefix(&mp)
                .is_some_and(|rest| rest.starts_with('/'));
        if matches {
            let len = mp.len();
            if best.as_ref().is_none_or(|b| len > b.0) {
                best = Some((len, mp, fstype.to_owned()));
            }
        }
    }
    match best {
        Some((_, mp, fs_type)) => (Some(mp), Some(fs_type)),
        None => (None, None),
    }
}

#[cfg(not(target_os = "linux"))]
fn linux_mount_info(_target: &Path) -> (Option<String>, Option<String>) {
    (None, None)
}

/// Inspect `/var/backups/opsbox`, `restic` presence/version, disk space, and whether local
/// backups share the same filesystem/device as opsbox data or root.
pub fn local_info_with(env: &Env, check_paths: &[String]) -> Result<BackupLocalInfo, PrivError> {
    let repo_dir = &env.backup_dir;
    let exists = repo_dir.exists();
    let repo_initialized = repo_dir.join("config").is_file();

    let restic_version = match find_restic_bin(env) {
        Ok(bin) => {
            let args = vec!["version".to_owned()];
            let out = block_on_fut(run_restic(
                &bin,
                &env.backup_cache_dir,
                None,
                &args,
                &[],
                RESTIC_VERSION_TIMEOUT,
                &[],
            ));
            out.ok().and_then(Result::ok).and_then(|o| {
                if o.code == Some(0) {
                    parse_version(&o.stdout)
                } else {
                    None
                }
            })
        }
        Err(_) => None,
    };
    let restic_installed = restic_version.is_some();

    let probe = existing_ancestor(repo_dir).unwrap_or_else(|| PathBuf::from("/"));
    let (mount_point, fs_type) = linux_mount_info(&probe);

    #[allow(clippy::useless_conversion)]
    let (total_bytes, available_bytes) = match nix::sys::statvfs::statvfs(&probe) {
        Ok(s) => {
            let frag = u64::from(s.fragment_size());
            let total = u64::from(s.blocks()).saturating_mul(frag);
            let avail = u64::from(s.blocks_available()).saturating_mul(frag);
            (Some(total), Some(avail))
        }
        Err(_) => (None, None),
    };

    let mut same_disk_paths = Vec::new();
    if let Ok(backup_meta) = fs::metadata(&probe) {
        let backup_dev = backup_meta.dev();
        let mut seen_labels = HashSet::new();
        let mut candidates: Vec<PathBuf> = vec![
            env.db_path.clone(),
            env.apps_dir.clone(),
            env.config_path.clone(),
            PathBuf::from("/var/lib/docker"),
        ];
        for raw in check_paths {
            let p = Path::new(raw);
            if p.is_absolute() && !p.components().any(|c| matches!(c, Component::ParentDir)) {
                candidates.push(p.to_path_buf());
            }
        }
        for c in candidates {
            if let Some(anc) = existing_ancestor(&c) {
                if anc == Path::new("/") && c != Path::new("/") {
                    continue;
                }
                if let Ok(m) = fs::metadata(&anc)
                    && m.dev() == backup_dev
                {
                    let label = c.display().to_string();
                    if seen_labels.insert(label.clone()) {
                        same_disk_paths.push(label);
                    }
                }
            }
        }
        if same_disk_paths.is_empty()
            && let Ok(root_meta) = fs::metadata("/")
            && root_meta.dev() == backup_dev
        {
            same_disk_paths.push("/".to_owned());
        }
    }

    Ok(BackupLocalInfo {
        path: repo_dir.display().to_string(),
        exists,
        repo_initialized,
        restic_installed,
        restic_version,
        mount_point,
        fs_type,
        total_bytes,
        available_bytes,
        same_disk: !same_disk_paths.is_empty(),
        same_disk_paths,
    })
}

/// Execute a backup target (`Database`, `Volume`, `Path`, `Opsbox`) against an open
/// locked `BackendConn`, returning the `ResticBackupSummary` (used by both normal runs
/// and pre-replace live safety snapshots).
#[allow(clippy::too_many_arguments)]
async fn execute_backup_target(
    env: &Env,
    restic_bin: &Path,
    backend: &BackendConn,
    pw: &PasswordFileGuard,
    job_id: &str,
    target: BackupRunTarget,
    extra_tags: &[String],
    log: &mut Vec<String>,
) -> Result<opsbox_backup::restic::ResticBackupSummary, PrivError> {
    let envs = backend.extra_env_pairs();
    let mut base_scrub_owned: Vec<String> = vec![pw.secret().to_owned()];
    backend.append_scrub_strings(&mut base_scrub_owned);

    match target {
        BackupRunTarget::Database {
            template,
            service_id,
            service_name,
            container_id,
            vars,
        } => {
            if !valid_backup_id(&service_id)
                || !valid_service_name(&service_name)
                || !valid_docker_ref(&container_id)
            {
                return Err(err(
                    PrivErrorCode::Invalid,
                    "Invalid database service or container identifier.",
                ));
            }
            let (opened, deploy_scrub) = open_deploy_vars(&env.secrets, &vars)?;
            let kv: Vec<(String, String)> = opened.into_iter().map(|v| (v.key, v.value)).collect();
            let dump_spec =
                build_dump_spec(template, &service_name, &kv).map_err(map_backup_err)?;

            let mut scrub_owned = base_scrub_owned;
            scrub_owned.extend(deploy_scrub);
            scrub_owned.extend(dump_spec.scrub_secrets);
            let scrub_refs: Vec<&str> = scrub_owned.iter().map(String::as_str).collect();

            log.push(format!(
                "Streaming {} dump from container {} ({}) directly into restic.",
                template.method_name(),
                service_name,
                container_id
            ));

            let mut tags = job_tags(job_id, BackupTargetKind::Database, &service_name);
            tags.extend(extra_tags.iter().cloned());
            let args = backup_stdin_args(
                &backend.repo_uri,
                pw.path(),
                &backend.extra_opts,
                &dump_spec.stdin_filename,
                &tags,
            );
            let mut cmd = clean_restic_command(restic_bin, &env.backup_cache_dir, &args, &envs);
            cmd.stdin(Stdio::piped());
            let mut child = cmd.spawn().map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Failed to start restic stdin backup: {e}"),
                )
            })?;
            let restic_stdin = child
                .stdin
                .take()
                .ok_or_else(|| err(PrivErrorCode::Internal, "Missing restic stdin pipe."))?;
            let out_pipe = child
                .stdout
                .take()
                .ok_or_else(|| err(PrivErrorCode::Internal, "Missing restic stdout pipe."))?;
            let err_pipe = child
                .stderr
                .take()
                .ok_or_else(|| err(PrivErrorCode::Internal, "Missing restic stderr pipe."))?;

            let exec_req = DockerExecRequest {
                id: container_id.clone(),
                cmd: dump_spec.cmd,
                env: dump_spec.env,
                stdin: None,
                user: None,
                working_dir: None,
            };

            let exec_fut = env
                .docker
                .exec_to_writer(exec_req, restic_stdin, DUMP_STREAM_TIMEOUT);
            let restic_fut = tokio::time::timeout(RESTIC_RUN_TIMEOUT, async {
                let (stdout, stderr, status) =
                    tokio::join!(read_capped(out_pipe), read_capped(err_pipe), child.wait(),);
                (stdout, stderr, status)
            });

            let (exec_res, restic_res) = tokio::join!(exec_fut, restic_fut);
            let (stdout, stderr, status_res) = match restic_res {
                Ok(v) => v,
                Err(_) => {
                    return Err(err(
                        PrivErrorCode::Internal,
                        "restic backup timed out while reading database dump.",
                    ));
                }
            };
            let status = status_res.map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Failed waiting for restic backup: {e}"),
                )
            })?;
            let stdout = scrub_secrets(&stdout, &scrub_refs);
            let stderr = scrub_secrets(&stderr, &scrub_refs);

            let exec_status = match exec_res {
                Ok(st) => st,
                Err(e) => {
                    discard_partial_snapshot(
                        restic_bin,
                        backend,
                        &env.backup_cache_dir,
                        pw,
                        &stdout,
                    )
                    .await;
                    return Err(map_docker_err(e, env.docker.socket_path()));
                }
            };
            if exec_status.exit_code != 0 || exec_status.bytes_written == 0 {
                discard_partial_snapshot(restic_bin, backend, &env.backup_cache_dir, pw, &stdout)
                    .await;
                let dump_err = scrub_secrets(exec_status.stderr.trim(), &scrub_refs);
                let detail = if dump_err.is_empty() && exec_status.bytes_written == 0 {
                    format!(
                        "Database dump command '{}' wrote 0 bytes (exit code {}).",
                        template.method_name(),
                        exec_status.exit_code
                    )
                } else if dump_err.is_empty() {
                    format!(
                        "Database dump command '{}' exited with status {}.",
                        template.method_name(),
                        exec_status.exit_code
                    )
                } else {
                    format!(
                        "Database dump command '{}' failed: {dump_err}",
                        template.method_name()
                    )
                };
                return Err(err(PrivErrorCode::Internal, detail));
            }
            if !status.success() {
                return Err(err(
                    PrivErrorCode::Internal,
                    restic_failure_msg(
                        "restic database backup",
                        &ResticCmdOutput {
                            code: status.code(),
                            stdout,
                            stderr,
                        },
                        &scrub_refs,
                    ),
                ));
            }
            parse_backup_summary(&stdout).map_err(map_backup_err)
        }
        BackupRunTarget::Volume {
            volumes,
            pause_containers,
        } => {
            let clean_vols = validate_volume_list(&volumes).map_err(map_backup_err)?;
            let mut mount_paths = Vec::with_capacity(clean_vols.len());
            for vol in &clean_vols {
                let mp = env
                    .docker
                    .inspect_volume_mountpoint(vol)
                    .await
                    .map_err(|e| map_docker_err(e, env.docker.socket_path()))?;
                let valid = validate_live_path(&mp, env)?;
                mount_paths.push(valid.to_string_lossy().into_owned());
            }

            let mut paused: Vec<String> = Vec::new();
            if pause_containers {
                let cids = env
                    .docker
                    .containers_mounting_volumes(&clean_vols)
                    .await
                    .map_err(|e| map_docker_err(e, env.docker.socket_path()))?;
                for cid in cids {
                    if let Err(e) = env.docker.pause_container(&cid).await {
                        for prev in paused.iter().rev() {
                            let _ = env.docker.unpause_container(prev).await;
                        }
                        return Err(map_docker_err(e, env.docker.socket_path()));
                    }
                    paused.push(cid);
                }
                if !paused.is_empty() {
                    log.push(format!(
                        "Temporarily paused {} container(s) for volume consistency.",
                        paused.len()
                    ));
                }
            }

            let what = clean_vols.join(", ");
            let mut tags = job_tags(job_id, BackupTargetKind::Volume, &what);
            tags.extend(extra_tags.iter().cloned());
            let excludes = backup_excludes(env);
            let args = backup_paths_args(
                &backend.repo_uri,
                pw.path(),
                &backend.extra_opts,
                &mount_paths,
                &excludes,
                &tags,
            );
            let scrub_refs: Vec<&str> = base_scrub_owned.iter().map(String::as_str).collect();
            let run_res = run_restic(
                restic_bin,
                &env.backup_cache_dir,
                None,
                &args,
                &envs,
                RESTIC_RUN_TIMEOUT,
                &scrub_refs,
            )
            .await;

            for cid in paused.iter().rev() {
                let _ = env.docker.unpause_container(cid).await;
            }

            let out = run_res?;
            if out.code != Some(0) {
                return Err(err(
                    PrivErrorCode::Internal,
                    restic_failure_msg("restic volume backup", &out, &scrub_refs),
                ));
            }
            parse_backup_summary(&out.stdout).map_err(map_backup_err)
        }
        BackupRunTarget::Path { paths } => {
            let clean = validate_path_list(&paths).map_err(map_backup_err)?;
            let mut resolved = Vec::with_capacity(clean.len());
            for p in &clean {
                let canon = validate_live_path(p, env)?;
                resolved.push(canon.to_string_lossy().into_owned());
            }
            let what = clean.join(", ");
            let mut tags = job_tags(job_id, BackupTargetKind::Path, &what);
            tags.extend(extra_tags.iter().cloned());
            let excludes = backup_excludes(env);
            let args = backup_paths_args(
                &backend.repo_uri,
                pw.path(),
                &backend.extra_opts,
                &resolved,
                &excludes,
                &tags,
            );
            let scrub_refs: Vec<&str> = base_scrub_owned.iter().map(String::as_str).collect();
            log.push(format!("Backing up {} path(s).", resolved.len()));
            let out = run_restic(
                restic_bin,
                &env.backup_cache_dir,
                None,
                &args,
                &envs,
                RESTIC_RUN_TIMEOUT,
                &scrub_refs,
            )
            .await?;
            if out.code != Some(0) {
                return Err(err(
                    PrivErrorCode::Internal,
                    restic_failure_msg("restic path backup", &out, &scrub_refs),
                ));
            }
            parse_backup_summary(&out.stdout).map_err(map_backup_err)
        }
        BackupRunTarget::Opsbox => {
            let env_db = env.db_path.clone();
            let env_cfg = env.config_path.clone();
            let env_mk = env.master_key.clone();
            let env_bk = env.backup_dir.clone();
            let env_cache = env.backup_cache_dir.clone();
            let (stage_dir, rel_targets) = tokio::task::spawn_blocking(move || {
                stage_opsbox_self(&env_db, &env_cfg, &env_mk, &env_bk, &env_cache)
            })
            .await
            .map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Self-backup staging task failed: {e}"),
                )
            })??;

            log.push("Staged online SQLite backup and /etc/opsbox (excluding master.key).".into());
            let mut tags = job_tags(
                job_id,
                BackupTargetKind::Opsbox,
                "opsbox (SQLite + /etc/opsbox)",
            );
            tags.extend(extra_tags.iter().cloned());
            let mut excludes = backup_excludes(env);
            excludes.push("master.key".to_owned());
            excludes.push("*/master.key".to_owned());
            let args = backup_paths_args(
                &backend.repo_uri,
                pw.path(),
                &backend.extra_opts,
                &rel_targets,
                &excludes,
                &tags,
            );
            let scrub_refs: Vec<&str> = base_scrub_owned.iter().map(String::as_str).collect();
            let out = run_restic(
                restic_bin,
                &env.backup_cache_dir,
                Some(stage_dir.path()),
                &args,
                &envs,
                RESTIC_RUN_TIMEOUT,
                &scrub_refs,
            )
            .await?;
            drop(stage_dir);
            if out.code != Some(0) {
                return Err(err(
                    PrivErrorCode::Internal,
                    restic_failure_msg("restic opsbox self-backup", &out, &scrub_refs),
                ));
            }
            parse_backup_summary(&out.stdout).map_err(map_backup_err)
        }
    }
}

/// Run a single validated backup job (`database`, `volume`, `path`, or `opsbox`).
pub fn run_backup_with(env: &Env, req: BackupRunSpec) -> Result<BackupRunResult, PrivError> {
    if !valid_backup_id(&req.job_id) {
        return Err(err(PrivErrorCode::Invalid, "Invalid backup job ID."));
    }
    if req.job_name.trim().is_empty() || req.job_name.len() > 80 {
        return Err(err(PrivErrorCode::Invalid, "Invalid backup job name."));
    }
    validate_retention(req.retain_daily, req.retain_weekly).map_err(map_backup_err)?;

    let restic_bin = find_restic_bin(env)?;
    let backend = resolve_backend_conn(env, &req.destination_id, &req.repo)?;
    let secret = derive_pw(&env.secrets)?;

    let started = Instant::now();
    let repo_mutex = repo_lock(&backend.lock_key);

    block_on_fut(async move {
        let _repo_guard = repo_mutex.lock().await;
        let pw = PasswordFileGuard::create(&env.backup_cache_dir, secret)?;
        let mut log = Vec::new();
        let envs = backend.extra_env_pairs();
        let scrub = build_scrub_refs(&pw, &backend);

        if ensure_repo_initialized(&restic_bin, &backend, &env.backup_cache_dir, &pw).await? {
            log.push(format!(
                "Initialized encrypted restic repository at {}.",
                backend.repo_uri
            ));
        }

        let summary = execute_backup_target(
            env,
            &restic_bin,
            &backend,
            &pw,
            &req.job_id,
            req.target,
            &[],
            &mut log,
        )
        .await?;

        log.push(format!(
            "Created snapshot {} ({} bytes processed, {} bytes added).",
            summary.short_id, summary.total_bytes_processed, summary.data_added
        ));

        let mut pruned_count = 0u32;
        if req.run_prune {
            let fg_args = forget_args(
                &backend.repo_uri,
                pw.path(),
                &backend.extra_opts,
                Some(&req.job_id),
                req.retain_daily,
                req.retain_weekly,
            );
            let fg_out = run_restic(
                &restic_bin,
                &env.backup_cache_dir,
                None,
                &fg_args,
                &envs,
                RESTIC_RUN_TIMEOUT,
                &scrub,
            )
            .await?;
            if fg_out.code != Some(0) {
                return Err(err(
                    PrivErrorCode::Internal,
                    restic_failure_msg("restic forget --prune", &fg_out, &scrub),
                ));
            }
            let parsed = parse_forget_output(&fg_out.stdout);
            pruned_count = u32::try_from(parsed.removed_snapshot_ids.len()).unwrap_or(u32::MAX);
            if pruned_count > 0 {
                log.push(format!("Pruned {pruned_count} expired snapshot(s)."));
            }
        }

        let mut check_state = BackupCheckState::Unverified;
        let mut check_error = None;
        if req.run_check {
            let chk_args = check_args(
                &backend.repo_uri,
                pw.path(),
                &backend.extra_opts,
                Some(DEFAULT_SUBSET),
            );
            let chk_out = run_restic(
                &restic_bin,
                &env.backup_cache_dir,
                None,
                &chk_args,
                &envs,
                RESTIC_RUN_TIMEOUT,
                &scrub,
            )
            .await?;
            let parsed = parse_check_output(&chk_out.stdout);
            let ok = chk_out.code == Some(0) && parsed.as_ref().is_none_or(|p| p.num_errors == 0);
            if ok {
                check_state = BackupCheckState::ChecksumOk;
                log.push(format!(
                    "Verified repository integrity ({DEFAULT_SUBSET} data subset)."
                ));
            } else {
                check_state = BackupCheckState::ChecksumFailed;
                let msg = restic_failure_msg("restic check", &chk_out, &scrub);
                log.push(format!("Integrity check failed: {msg}"));
                check_error = Some(msg);
            }
        }

        Ok(BackupRunResult {
            snapshot_id: summary.snapshot_id,
            short_id: summary.short_id,
            taken_at: now_unix(),
            size_bytes: summary.total_bytes_processed,
            data_added_bytes: summary.data_added,
            files_processed: summary.total_files_processed,
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX as u64),
            pruned_count,
            check_state,
            check_error,
            log,
        })
    })?
}

/// List snapshots stored in the encrypted restic repository.
pub fn list_snapshots_with(
    env: &Env,
    req: BackupSnapshotsRequest,
) -> Result<Vec<ResticSnapshotInfo>, PrivError> {
    let backend = resolve_backend_conn(env, &req.destination_id, &req.repo)?;
    if let Some(ref local_dir) = backend.local_dir
        && !local_dir.join("config").is_file()
    {
        return Ok(Vec::new());
    }
    let restic_bin = find_restic_bin(env)?;
    let secret = derive_pw(&env.secrets)?;
    let repo_mutex = repo_lock(&backend.lock_key);

    block_on_fut(async move {
        let _guard = repo_mutex.lock().await;
        if let Some(ref local_dir) = backend.local_dir
            && !local_dir.join("config").is_file()
        {
            return Ok(Vec::new());
        }
        let pw = PasswordFileGuard::create(&env.backup_cache_dir, secret)?;
        let args = snapshots_args(&backend.repo_uri, pw.path(), &backend.extra_opts);
        let envs = backend.extra_env_pairs();
        let scrub = build_scrub_refs(&pw, &backend);
        let out = run_restic(
            &restic_bin,
            &env.backup_cache_dir,
            None,
            &args,
            &envs,
            RESTIC_META_TIMEOUT,
            &scrub,
        )
        .await?;
        if out.code != Some(0) {
            if is_missing_repo_err(&out) {
                return Ok(Vec::new());
            }
            return Err(err(
                PrivErrorCode::Internal,
                restic_failure_msg("restic snapshots", &out, &scrub),
            ));
        }
        parse_snapshots(&out.stdout, now_unix()).map_err(map_backup_err)
    })?
}

/// Apply daily/weekly retention and prune unreferenced packs (`restic forget --prune`).
pub fn forget_with(env: &Env, req: BackupForgetRequest) -> Result<BackupForgetResult, PrivError> {
    if let Some(ref jid) = req.job_id
        && !valid_backup_id(jid)
    {
        return Err(err(PrivErrorCode::Invalid, "Invalid backup job ID."));
    }
    validate_retention(req.retain_daily, req.retain_weekly).map_err(map_backup_err)?;
    let backend = resolve_backend_conn(env, &req.destination_id, &req.repo)?;
    if let Some(ref local_dir) = backend.local_dir
        && !local_dir.join("config").is_file()
    {
        return Ok(BackupForgetResult {
            removed_snapshot_ids: Vec::new(),
            kept_count: 0,
            removed_count: 0,
            log: vec!["Local backup repository has no snapshots yet.".into()],
        });
    }
    let restic_bin = find_restic_bin(env)?;
    let secret = derive_pw(&env.secrets)?;
    let repo_mutex = repo_lock(&backend.lock_key);

    block_on_fut(async move {
        let _guard = repo_mutex.lock().await;
        let pw = PasswordFileGuard::create(&env.backup_cache_dir, secret)?;
        let args = forget_args(
            &backend.repo_uri,
            pw.path(),
            &backend.extra_opts,
            req.job_id.as_deref(),
            req.retain_daily,
            req.retain_weekly,
        );
        let envs = backend.extra_env_pairs();
        let scrub = build_scrub_refs(&pw, &backend);
        let out = run_restic(
            &restic_bin,
            &env.backup_cache_dir,
            None,
            &args,
            &envs,
            RESTIC_RUN_TIMEOUT,
            &scrub,
        )
        .await?;
        if out.code != Some(0) {
            if is_missing_repo_err(&out) {
                return Ok(BackupForgetResult {
                    removed_snapshot_ids: Vec::new(),
                    kept_count: 0,
                    removed_count: 0,
                    log: vec!["Backup repository has no snapshots yet.".into()],
                });
            }
            return Err(err(
                PrivErrorCode::Internal,
                restic_failure_msg("restic forget --prune", &out, &scrub),
            ));
        }
        let parsed = parse_forget_output(&out.stdout);
        let removed_count = u32::try_from(parsed.removed_snapshot_ids.len()).unwrap_or(u32::MAX);
        Ok(BackupForgetResult {
            removed_snapshot_ids: parsed.removed_snapshot_ids,
            kept_count: parsed.kept_count,
            removed_count,
            log: vec![format!(
                "Pruned {removed_count} snapshot(s); kept {}.",
                parsed.kept_count
            )],
        })
    })?
}

/// Run `restic check` (optionally `--read-data-subset`) on a repository.
pub fn check_with(env: &Env, req: BackupCheckRequest) -> Result<BackupCheckResult, PrivError> {
    let subset = match req.read_data_subset {
        Some(ref s) => Some(validate_read_data_subset(s).map_err(map_backup_err)?),
        None => Some(DEFAULT_SUBSET.to_owned()),
    };
    let backend = resolve_backend_conn(env, &req.destination_id, &req.repo)?;
    if let Some(ref local_dir) = backend.local_dir
        && !local_dir.join("config").is_file()
    {
        return Err(err(
            PrivErrorCode::NotFound,
            "No local backup repository exists yet — run a backup first.",
        ));
    }
    let restic_bin = find_restic_bin(env)?;
    let secret = derive_pw(&env.secrets)?;
    let repo_mutex = repo_lock(&backend.lock_key);

    block_on_fut(async move {
        let _guard = repo_mutex.lock().await;
        let pw = PasswordFileGuard::create(&env.backup_cache_dir, secret)?;
        let args = check_args(
            &backend.repo_uri,
            pw.path(),
            &backend.extra_opts,
            subset.as_deref(),
        );
        let envs = backend.extra_env_pairs();
        let scrub = build_scrub_refs(&pw, &backend);
        let out = run_restic(
            &restic_bin,
            &env.backup_cache_dir,
            None,
            &args,
            &envs,
            RESTIC_RUN_TIMEOUT,
            &scrub,
        )
        .await?;
        if out.code != Some(0) && is_missing_repo_err(&out) {
            return Err(err(
                PrivErrorCode::NotFound,
                "Backup repository has not been initialized yet — run a backup first.",
            ));
        }
        let parsed = parse_check_output(&out.stdout);
        let (num_errors, broken_packs) = match parsed {
            Some(p) => (p.num_errors, p.broken_packs),
            None => (u64::from(out.code != Some(0)), Vec::new()),
        };
        let ok = out.code == Some(0) && num_errors == 0 && broken_packs.is_empty();
        let summary = if ok {
            match subset {
                Some(ref sub) => {
                    format!("Repository checksum verification passed ({sub} read check).")
                }
                None => "Repository structural check passed.".into(),
            }
        } else {
            restic_failure_msg("restic check", &out, &scrub)
        };
        Ok(BackupCheckResult {
            ok,
            num_errors,
            broken_packs,
            summary,
        })
    })?
}

fn unique_test_suffix() -> String {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{:x}{:x}", now_unix(), n)
}

fn generate_ephemeral_token() -> String {
    let mut bytes = [0u8; 16];
    let _ = getrandom::fill(&mut bytes);
    let mut s = String::with_capacity(32);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

/// Test connectivity, repository initialization, encrypted probe write+readback, and delete permission.
pub fn test_destination_with(
    env: &Env,
    req: BackupDestinationTestRequest,
) -> Result<BackupDestinationTestResult, PrivError> {
    match req.target {
        BackupDestinationTestTarget::SftpReachability {
            host,
            port,
            user,
            repo_path,
        } => {
            let clean_host = validate_sftp_host(&host).map_err(map_backup_err)?;
            let _ = validate_sftp_user(&user).map_err(map_backup_err)?;
            let _ = validate_sftp_path(&repo_path).map_err(map_backup_err)?;
            if port == 0 {
                return Err(err(PrivErrorCode::Invalid, "SSH port must be 1..=65535."));
            }
            block_on_fut(async move {
                let addr = format!("{clean_host}:{port}");
                match tokio::time::timeout(
                    Duration::from_secs(8),
                    tokio::net::TcpStream::connect(&addr),
                )
                .await
                {
                    Ok(Ok(_stream)) => BackupDestinationTestResult {
                        ok: true,
                        verified: false,
                        steps: vec![
                            BackupTestStep {
                                status: BackupTestStepStatus::Ok,
                                message: format!("Reached {clean_host}:{port} over TCP."),
                            },
                            BackupTestStep {
                                status: BackupTestStepStatus::Warn,
                                message: "Save this destination to view opsbox's SSH public key, add it to ~/.ssh/authorized_keys, then run Test connection again.".into(),
                            },
                        ],
                    },
                    Ok(Err(e)) => BackupDestinationTestResult {
                        ok: false,
                        verified: false,
                        steps: vec![BackupTestStep {
                            status: BackupTestStepStatus::Fail,
                            message: format!("Could not connect to {clean_host}:{port}: {e}"),
                        }],
                    },
                    Err(_) => BackupDestinationTestResult {
                        ok: false,
                        verified: false,
                        steps: vec![BackupTestStep {
                            status: BackupTestStepStatus::Fail,
                            message: format!("Timed out connecting to {clean_host}:{port} after 8s."),
                        }],
                    },
                }
            })
        }
        BackupDestinationTestTarget::Restic { repo } => {
            let dest_id = match &repo {
                BackupRepoTarget::Local => "local",
                BackupRepoTarget::S3 { destination_id, .. }
                | BackupRepoTarget::Sftp { destination_id, .. } => destination_id.as_str(),
            };
            let restic_bin = find_restic_bin(env)?;
            let backend = resolve_backend_conn(env, dest_id, &repo)?;
            let secret = derive_pw(&env.secrets)?;
            let repo_mutex = repo_lock(&backend.lock_key);
            let provider_name = req.provider.display_name();

            block_on_fut(async move {
                let _guard = repo_mutex.lock().await;
                let pw = PasswordFileGuard::create(&env.backup_cache_dir, secret)?;
                let envs = backend.extra_env_pairs();
                let scrub = build_scrub_refs(&pw, &backend);
                let mut steps = Vec::new();

                // Step 1: check or initialize encrypted repo.
                let cat = cat_config_args(&backend.repo_uri, pw.path(), &backend.extra_opts);
                let cat_out = run_restic(
                    &restic_bin,
                    &env.backup_cache_dir,
                    None,
                    &cat,
                    &envs,
                    RESTIC_META_TIMEOUT,
                    &scrub,
                )
                .await?;
                if cat_out.code == Some(0) {
                    steps.push(BackupTestStep {
                        status: BackupTestStepStatus::Ok,
                        message: format!(
                            "Connected to existing encrypted {provider_name} repository."
                        ),
                    });
                } else {
                    let init_a = init_args(&backend.repo_uri, pw.path(), &backend.extra_opts);
                    let init_out = run_restic(
                        &restic_bin,
                        &env.backup_cache_dir,
                        None,
                        &init_a,
                        &envs,
                        RESTIC_META_TIMEOUT,
                        &scrub,
                    )
                    .await?;
                    if init_out.code == Some(0) {
                        steps.push(BackupTestStep {
                            status: BackupTestStepStatus::Ok,
                            message: format!(
                                "Created encrypted restic repository on {provider_name}."
                            ),
                        });
                    } else {
                        steps.push(BackupTestStep {
                            status: BackupTestStepStatus::Fail,
                            message: restic_failure_msg(
                                "Connecting to / initializing repository",
                                &init_out,
                                &scrub,
                            ),
                        });
                        return Ok(BackupDestinationTestResult {
                            ok: false,
                            verified: false,
                            steps,
                        });
                    }
                }

                // Step 2: encrypted probe write + dump readback.
                let probe_token = format!("opsbox-probe-{}", generate_ephemeral_token());
                let tags = vec!["opsbox-probe:true".to_owned()];
                let bk_a = backup_stdin_args(
                    &backend.repo_uri,
                    pw.path(),
                    &backend.extra_opts,
                    "opsbox-probe.txt",
                    &tags,
                );
                let bk_out = run_restic_with_stdin(
                    &restic_bin,
                    &env.backup_cache_dir,
                    &bk_a,
                    &envs,
                    probe_token.as_bytes(),
                    RESTIC_META_TIMEOUT,
                    &scrub,
                )
                .await?;
                if bk_out.code != Some(0) {
                    steps.push(BackupTestStep {
                        status: BackupTestStepStatus::Fail,
                        message: restic_failure_msg("Writing encrypted probe", &bk_out, &scrub),
                    });
                    return Ok(BackupDestinationTestResult {
                        ok: false,
                        verified: false,
                        steps,
                    });
                }
                let summary = match parse_backup_summary(&bk_out.stdout) {
                    Ok(s) => s,
                    Err(e) => {
                        steps.push(BackupTestStep {
                            status: BackupTestStepStatus::Fail,
                            message: e.to_string(),
                        });
                        return Ok(BackupDestinationTestResult {
                            ok: false,
                            verified: false,
                            steps,
                        });
                    }
                };
                let dmp_a = dump_args(
                    &backend.repo_uri,
                    pw.path(),
                    &backend.extra_opts,
                    &summary.snapshot_id,
                    "/opsbox-probe.txt",
                );
                let dmp_out = run_restic(
                    &restic_bin,
                    &env.backup_cache_dir,
                    None,
                    &dmp_a,
                    &envs,
                    RESTIC_META_TIMEOUT,
                    &scrub,
                )
                .await?;
                if dmp_out.code != Some(0) || dmp_out.stdout.trim() != probe_token {
                    let fg = forget_single_snapshot_args(
                        &backend.repo_uri,
                        pw.path(),
                        &backend.extra_opts,
                        &summary.snapshot_id,
                    );
                    let _ = run_restic(
                        &restic_bin,
                        &env.backup_cache_dir,
                        None,
                        &fg,
                        &envs,
                        RESTIC_META_TIMEOUT,
                        &scrub,
                    )
                    .await;
                    steps.push(BackupTestStep {
                        status: BackupTestStepStatus::Fail,
                        message: if dmp_out.code != Some(0) {
                            restic_failure_msg("Reading encrypted probe back", &dmp_out, &scrub)
                        } else {
                            "Probe readback did not match written payload.".into()
                        },
                    });
                    return Ok(BackupDestinationTestResult {
                        ok: false,
                        verified: false,
                        steps,
                    });
                }
                steps.push(BackupTestStep {
                    status: BackupTestStepStatus::Ok,
                    message: "Wrote encrypted probe and read it back cleanly.".into(),
                });

                // Step 3: delete-permission check (warn only on failure).
                let fg_a = forget_single_snapshot_args(
                    &backend.repo_uri,
                    pw.path(),
                    &backend.extra_opts,
                    &summary.snapshot_id,
                );
                let fg_out = run_restic(
                    &restic_bin,
                    &env.backup_cache_dir,
                    None,
                    &fg_a,
                    &envs,
                    RESTIC_META_TIMEOUT,
                    &scrub,
                )
                .await?;
                if fg_out.code == Some(0) {
                    steps.push(BackupTestStep {
                        status: BackupTestStepStatus::Ok,
                        message: "Verified snapshot deletion / retention prune permission.".into(),
                    });
                } else {
                    steps.push(BackupTestStep {
                        status: BackupTestStepStatus::Warn,
                        message: format!(
                            "Delete permission failed — old backups won't prune until granted ({})",
                            restic_failure_msg("forget", &fg_out, &scrub)
                        ),
                    });
                }

                Ok(BackupDestinationTestResult {
                    ok: true,
                    verified: true,
                    steps,
                })
            })?
        }
    }
}

/// Generate a fresh Ed25519 SSH keypair inside `priv` and seal its private key for `backup:dest:<id>`.
pub fn generate_sftp_key_with(
    env: &Env,
    req: &BackupGenerateSftpKeyRequest,
) -> Result<BackupSftpKeyResult, PrivError> {
    if !valid_backup_id(&req.destination_id) {
        return Err(err(
            PrivErrorCode::Invalid,
            "Invalid backup destination identifier.",
        ));
    }
    let pair = generate_ed25519_deploy_key(&format!("opsbox-backup-{}", req.destination_id))
        .map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Could not generate Ed25519 SFTP keypair: {e}"),
            )
        })?;
    let seal_context = format!("backup:dest:{}", req.destination_id);
    let reply = sealed::seal(
        &env.secrets,
        &seal_context,
        &Secret::new(pair.private_key_pem.as_str().to_owned()),
    )?;
    let opsbox_proto::PrivReply::Sealed {
        sealed: private_key_enc,
    } = reply
    else {
        return Err(err(
            PrivErrorCode::Internal,
            "Unexpected reply when sealing SFTP private key.",
        ));
    };
    Ok(BackupSftpKeyResult {
        public_key: pair.public_key,
        fingerprint: pair.fingerprint,
        seal_context,
        private_key_enc,
    })
}

/// Derive and return the deterministic restic repository recovery password (behind admin re-auth in `serve`).
pub fn recovery_key_with(env: &Env) -> Result<BackupRecoveryKeyResult, PrivError> {
    let pw = derive_pw(&env.secrets)?;
    let hostname = sysinfo::System::host_name().unwrap_or_else(|_| "opsbox-server".to_owned());
    Ok(BackupRecoveryKeyResult {
        recovery_key: Secret::new(pw.to_string()),
        hostname,
    })
}

async fn wait_for_db_sanity(
    env: &Env,
    container_id: &str,
    sanity_cmd: &[String],
    sanity_env: &[String],
    scrub_refs: &[&str],
) -> Result<String, PrivError> {
    let mut last_detail = "Database container did not become ready in time.".to_owned();
    for attempt in 0..30u8 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let req = DockerExecRequest {
            id: container_id.to_owned(),
            cmd: sanity_cmd.to_vec(),
            env: sanity_env.to_vec(),
            stdin: None,
            user: None,
            working_dir: None,
        };
        match env.docker.exec(req).await {
            Ok(out) if out.exit_code == 0 => {
                let txt = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                return Ok(txt);
            }
            Ok(out) => {
                let stderr_clean = scrub_secrets(out.stderr.trim(), scrub_refs);
                last_detail = if stderr_clean.is_empty() {
                    format!("Sanity check exited with code {}", out.exit_code)
                } else {
                    stderr_clean
                };
            }
            Err(e) => {
                last_detail = scrub_secrets(&e.to_string(), scrub_refs);
            }
        }
    }
    Err(err(PrivErrorCode::Internal, last_detail))
}

#[allow(clippy::too_many_arguments)]
async fn restore_database_into_container(
    env: &Env,
    restic_bin: &Path,
    backend: &BackendConn,
    pw: &PasswordFileGuard,
    restic_id: &str,
    template: BackupDbKind,
    service_name: &str,
    container_id: &str,
    stdin_filename: &str,
    kv: &[(String, String)],
    scrub_owned: &[String],
    log: &mut Vec<String>,
) -> Result<String, PrivError> {
    let restore_spec = build_restore_spec(template, service_name, kv).map_err(map_backup_err)?;
    let sanity_spec = build_sanity_check_spec(template, kv).map_err(map_backup_err)?;
    let mut all_scrub: Vec<String> = scrub_owned.to_vec();
    all_scrub.extend(restore_spec.scrub_secrets);
    all_scrub.extend(sanity_spec.scrub_secrets);
    let scrub_refs: Vec<&str> = all_scrub.iter().map(String::as_str).collect();

    let clean_stdin = format!("/{}", stdin_filename.trim_start_matches('/'));
    let dmp_args = dump_args(
        &backend.repo_uri,
        pw.path(),
        &backend.extra_opts,
        restic_id,
        &clean_stdin,
    );
    let envs = backend.extra_env_pairs();

    match restore_spec.method {
        DatabaseRestoreMethod::StdinExec { cmd, env: exec_env } => {
            let _ = wait_for_db_sanity(
                env,
                container_id,
                &sanity_spec.cmd,
                &sanity_spec.env,
                &scrub_refs,
            )
            .await?;

            log.push(format!(
                "Streaming snapshot {restic_id} ({clean_stdin}) into {service_name} ({container_id})."
            ));

            let mut rcmd =
                clean_restic_command(restic_bin, &env.backup_cache_dir, &dmp_args, &envs);
            let mut rchild = rcmd.spawn().map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Failed to start restic dump: {e}"),
                )
            })?;
            let r_stdout = rchild
                .stdout
                .take()
                .ok_or_else(|| err(PrivErrorCode::Internal, "Missing restic dump stdout."))?;
            let r_stderr = rchild
                .stderr
                .take()
                .ok_or_else(|| err(PrivErrorCode::Internal, "Missing restic dump stderr."))?;

            let exec_req = DockerExecRequest {
                id: container_id.to_owned(),
                cmd,
                env: exec_env,
                stdin: None,
                user: None,
                working_dir: None,
            };
            let exec_fut = env
                .docker
                .exec_from_reader(exec_req, r_stdout, RESTIC_RUN_TIMEOUT);
            let restic_fut = tokio::time::timeout(RESTIC_RUN_TIMEOUT, async {
                tokio::join!(read_capped(r_stderr), rchild.wait())
            });
            let (exec_res, restic_res) = tokio::join!(exec_fut, restic_fut);
            let (r_err_txt, r_status_res) = restic_res.map_err(|_| {
                err(
                    PrivErrorCode::Internal,
                    "restic dump timed out during database restore.",
                )
            })?;
            let r_status = r_status_res.map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Failed waiting for restic dump: {e}"),
                )
            })?;
            if !r_status.success() {
                return Err(err(
                    PrivErrorCode::Internal,
                    restic_failure_msg(
                        "restic dump",
                        &ResticCmdOutput {
                            code: r_status.code(),
                            stdout: String::new(),
                            stderr: scrub_secrets(&r_err_txt, &scrub_refs),
                        },
                        &scrub_refs,
                    ),
                ));
            }
            let exec_out = exec_res.map_err(|e| map_docker_err(e, env.docker.socket_path()))?;
            if exec_out.exit_code != 0 {
                let err_clean = scrub_secrets(exec_out.stderr.trim(), &scrub_refs);
                return Err(err(
                    PrivErrorCode::Internal,
                    if err_clean.is_empty() {
                        format!(
                            "Database restore command exited with code {}.",
                            exec_out.exit_code
                        )
                    } else {
                        format!("Database restore command failed: {err_clean}")
                    },
                ));
            }
        }
        DatabaseRestoreMethod::RedisRdbVolume => {
            ensure_private_dir(&env.backup_cache_dir)?;
            let rdb_stage = tempfile::Builder::new()
                .prefix("opsbox-rdb-")
                .tempfile_in(&env.backup_cache_dir)
                .map_err(|e| {
                    err(
                        PrivErrorCode::Internal,
                        format!("Could not stage temporary RDB dump: {e}"),
                    )
                })?;

            let mut rcmd =
                clean_restic_command(restic_bin, &env.backup_cache_dir, &dmp_args, &envs);
            let mut rchild = rcmd.spawn().map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Failed to start restic dump for Redis RDB: {e}"),
                )
            })?;
            let mut r_stdout = rchild
                .stdout
                .take()
                .ok_or_else(|| err(PrivErrorCode::Internal, "Missing restic dump stdout."))?;
            let r_stderr = rchild
                .stderr
                .take()
                .ok_or_else(|| err(PrivErrorCode::Internal, "Missing restic dump stderr."))?;

            let mut tfile = tokio::fs::File::create(rdb_stage.path())
                .await
                .map_err(|e| {
                    err(
                        PrivErrorCode::Internal,
                        format!("Failed opening RDB staging file: {e}"),
                    )
                })?;
            let (copy_res, err_txt, status_res) = tokio::join!(
                tokio::io::copy(&mut r_stdout, &mut tfile),
                read_capped(r_stderr),
                rchild.wait(),
            );
            let copied_bytes = copy_res.map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Failed staging Redis RDB payload: {e}"),
                )
            })?;
            drop(tfile);
            let status = status_res.map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Failed waiting for restic dump: {e}"),
                )
            })?;
            if !status.success() || copied_bytes == 0 {
                return Err(err(
                    PrivErrorCode::Internal,
                    restic_failure_msg(
                        "restic dump (Redis RDB)",
                        &ResticCmdOutput {
                            code: status.code(),
                            stdout: String::new(),
                            stderr: scrub_secrets(&err_txt, &scrub_refs),
                        },
                        &scrub_refs,
                    ),
                ));
            }

            let info = env
                .docker
                .inspect_container(container_id)
                .await
                .map_err(|e| map_docker_err(e, env.docker.socket_path()))?;
            let data_mount = info
                .mounts
                .iter()
                .find(|m| m.destination == "/data")
                .map(|m| m.source.clone())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    err(
                        PrivErrorCode::Internal,
                        "Redis container has no /data mount for RDB restore.",
                    )
                })?;

            let data_dir = if Path::new(&data_mount).is_absolute() {
                validate_live_path(&data_mount, env)?
            } else {
                let mp = env
                    .docker
                    .inspect_volume_mountpoint(&data_mount)
                    .await
                    .map_err(|e| map_docker_err(e, env.docker.socket_path()))?;
                validate_live_path(&mp, env)?
            };

            if info.running {
                env.docker
                    .container_action(container_id, DockerContainerAction::Stop)
                    .await
                    .map_err(|e| map_docker_err(e, env.docker.socket_path()))?;
            }

            let aof_dir = data_dir.join("appendonlydir");
            if aof_dir.is_dir() {
                let _ = fs::remove_dir_all(&aof_dir);
            }
            let aof_file = data_dir.join("appendonly.aof");
            if aof_file.exists() {
                let _ = fs::remove_file(&aof_file);
            }
            let dst_rdb = data_dir.join("dump.rdb");
            fs::copy(rdb_stage.path(), &dst_rdb).map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Could not write Redis dump.rdb: {e}"),
                )
            })?;
            let _ = fs::set_permissions(&dst_rdb, fs::Permissions::from_mode(0o644));

            env.docker
                .container_action(container_id, DockerContainerAction::Start)
                .await
                .map_err(|e| map_docker_err(e, env.docker.socket_path()))?;
            log.push(format!(
                "Loaded Redis RDB snapshot ({copied_bytes} bytes) into {service_name}."
            ));
        }
    }

    let sanity_out = wait_for_db_sanity(
        env,
        container_id,
        &sanity_spec.cmd,
        &sanity_spec.env,
        &scrub_refs,
    )
    .await?;
    log.push(format!(
        "Verified restored {service_name} database responds to sanity check ({sanity_out})."
    ));
    Ok(sanity_out)
}

fn copy_dir_recursive_safe(src: &Path, dst: &Path) -> Result<(), PrivError> {
    let meta = fs::symlink_metadata(src).map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Cannot stat restored entry '{}': {e}", src.display()),
        )
    })?;
    if src.file_name().and_then(|n| n.to_str()) == Some("master.key") {
        return Ok(());
    }
    if meta.file_type().is_symlink() {
        #[cfg(unix)]
        {
            let target = fs::read_link(src).map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Cannot read symlink '{}': {e}", src.display()),
                )
            })?;
            std::os::unix::fs::symlink(target, dst).map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Cannot restore symlink '{}': {e}", dst.display()),
                )
            })?;
        }
        return Ok(());
    }
    if meta.is_dir() {
        fs::create_dir_all(dst).map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Cannot create restore directory '{}': {e}", dst.display()),
            )
        })?;
        let _ = fs::set_permissions(dst, meta.permissions());
        for entry in fs::read_dir(src).map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Cannot read restored directory '{}': {e}", src.display()),
            )
        })? {
            let entry = entry.map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Cannot read directory entry: {e}"),
                )
            })?;
            copy_dir_recursive_safe(&entry.path(), &dst.join(entry.file_name()))?;
        }
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Cannot create restore parent '{}': {e}", parent.display()),
            )
        })?;
    }
    fs::copy(src, dst).map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Cannot copy restored file to '{}': {e}", dst.display()),
        )
    })?;
    let _ = fs::set_permissions(dst, meta.permissions());
    Ok(())
}

fn find_restored_source(stage: &Path, snapshot_paths: &[String]) -> PathBuf {
    for p in snapshot_paths {
        for cand_path in [Some(PathBuf::from(p)), fs::canonicalize(p).ok()]
            .into_iter()
            .flatten()
        {
            let s = cand_path.to_string_lossy();
            let rel = s.trim_start_matches('/');
            if rel.is_empty() {
                continue;
            }
            let candidate = stage.join(rel);
            if candidate.exists() {
                return candidate;
            }
        }
    }
    stage.to_path_buf()
}

fn clear_dir_contents(dir: &Path) -> Result<(), PrivError> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir).map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Cannot scan target directory '{}': {e}", dir.display()),
        )
    })? {
        let entry = entry.map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Cannot inspect directory entry: {e}"),
            )
        })?;
        let p = entry.path();
        let meta = fs::symlink_metadata(&p).map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Cannot stat '{}': {e}", p.display()),
            )
        })?;
        if meta.is_dir() && !meta.file_type().is_symlink() {
            fs::remove_dir_all(&p)
        } else {
            fs::remove_file(&p)
        }
        .map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Cannot clear existing entry '{}': {e}", p.display()),
            )
        })?;
    }
    Ok(())
}

fn find_opsbox_db_in_tree(root: &Path, depth: u8) -> Option<PathBuf> {
    let direct = root.join("opsbox.db");
    if direct.is_file() {
        return Some(direct);
    }
    if depth == 0 {
        return None;
    }
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir()
            && let Some(found) = find_opsbox_db_in_tree(&p, depth - 1)
        {
            return Some(found);
        }
    }
    None
}

/// Restore a snapshot (`copy` by default, or `replace` in place after typed confirmation
/// + taking a live safety snapshot first).
pub fn restore_with(
    env: &Env,
    req: BackupRestoreRequest,
) -> Result<BackupRestoreResult, PrivError> {
    if req.mode == BackupRestoreMode::Replace && req.confirm_replace.as_deref() != Some("replace") {
        return Err(err(
            PrivErrorCode::ConfirmRequired,
            "Type \"replace\" to confirm replacing live data in place.",
        ));
    }
    if !opsbox_proto::valid_restic_snapshot_id(&req.restic_id) {
        return Err(err(PrivErrorCode::Invalid, "Invalid snapshot ID."));
    }
    if matches!(req.target, BackupRestoreTarget::Opsbox { .. })
        && req.mode == BackupRestoreMode::Replace
    {
        return Err(err(
            PrivErrorCode::Refused,
            "opsbox itself cannot be replaced in place while running; restore to a copy instead.",
        ));
    }

    let restic_bin = find_restic_bin(env)?;
    let backend = resolve_backend_conn(env, &req.destination_id, &req.repo)?;
    let secret = derive_pw(&env.secrets)?;
    let repo_mutex = repo_lock(&backend.lock_key);
    let date_sfx = date_suffix_utc(now_unix());

    block_on_fut(async move {
        let _guard = repo_mutex.lock().await;
        let pw = PasswordFileGuard::create(&env.backup_cache_dir, secret)?;
        let envs = backend.extra_env_pairs();
        let scrub = build_scrub_refs(&pw, &backend);
        let mut log = Vec::new();
        let mut safety_id = None;
        let mut safety_short = None;

        // Mandatory pre-replace live safety snapshot BEFORE modifying anything live.
        if req.mode == BackupRestoreMode::Replace {
            let safety_job = format!("safety-{}", unique_test_suffix());
            let safety_tags = vec!["opsbox-safety:true".to_owned()];
            match &req.target {
                BackupRestoreTarget::Database {
                    template,
                    service_id,
                    service_name,
                    container_id,
                    vars,
                    ..
                } => {
                    log.push(format!(
                        "Taking live safety snapshot of {service_name} before replace restore."
                    ));
                    let sum = execute_backup_target(
                        env,
                        &restic_bin,
                        &backend,
                        &pw,
                        &safety_job,
                        BackupRunTarget::Database {
                            template: *template,
                            service_id: service_id.clone(),
                            service_name: service_name.clone(),
                            container_id: container_id.clone(),
                            vars: vars.clone(),
                        },
                        &safety_tags,
                        &mut log,
                    )
                    .await?;
                    safety_id = Some(sum.snapshot_id);
                    safety_short = Some(sum.short_id);
                }
                BackupRestoreTarget::Path { snapshot_paths, .. } => {
                    let clean = validate_path_list(snapshot_paths).map_err(map_backup_err)?;
                    let mut existing = Vec::new();
                    for p in &clean {
                        validate_restore_path(p, true).map_err(map_backup_err)?;
                        if Path::new(p).exists() {
                            existing.push(p.clone());
                        }
                    }
                    if !existing.is_empty() {
                        log.push(
                            "Taking live safety snapshot of path(s) before replace restore.".into(),
                        );
                        let sum = execute_backup_target(
                            env,
                            &restic_bin,
                            &backend,
                            &pw,
                            &safety_job,
                            BackupRunTarget::Path { paths: existing },
                            &safety_tags,
                            &mut log,
                        )
                        .await?;
                        safety_id = Some(sum.snapshot_id);
                        safety_short = Some(sum.short_id);
                    } else {
                        log.push(
                            "Live target path does not exist yet; skipping safety snapshot.".into(),
                        );
                    }
                }
                BackupRestoreTarget::Volume { source_volumes, .. } => {
                    let clean = validate_volume_list(source_volumes).map_err(map_backup_err)?;
                    log.push(
                        "Taking live safety snapshot of volume(s) before replace restore.".into(),
                    );
                    let sum = execute_backup_target(
                        env,
                        &restic_bin,
                        &backend,
                        &pw,
                        &safety_job,
                        BackupRunTarget::Volume {
                            volumes: clean,
                            pause_containers: true,
                        },
                        &safety_tags,
                        &mut log,
                    )
                    .await?;
                    safety_id = Some(sum.snapshot_id);
                    safety_short = Some(sum.short_id);
                }
                BackupRestoreTarget::Opsbox { .. } => {}
            }
        }

        let target_summary = match req.target {
            BackupRestoreTarget::Database {
                template,
                service_id,
                service_name,
                container_id,
                stdin_filename,
                vars,
            } => {
                if !valid_backup_id(&service_id)
                    || !valid_service_name(&service_name)
                    || !valid_docker_ref(&container_id)
                {
                    return Err(err(
                        PrivErrorCode::Invalid,
                        "Invalid database service or container identifier.",
                    ));
                }
                let (opened, deploy_scrub) = open_deploy_vars(&env.secrets, &vars)?;
                let kv: Vec<(String, String)> =
                    opened.into_iter().map(|v| (v.key, v.value)).collect();
                let mut scrub_owned: Vec<String> = scrub.iter().map(|s| (*s).to_owned()).collect();
                scrub_owned.extend(deploy_scrub);
                restore_database_into_container(
                    env,
                    &restic_bin,
                    &backend,
                    &pw,
                    &req.restic_id,
                    template,
                    &service_name,
                    &container_id,
                    &stdin_filename,
                    &kv,
                    &scrub_owned,
                    &mut log,
                )
                .await?;
                service_name
            }
            BackupRestoreTarget::Path {
                snapshot_paths,
                copy_target,
            } => {
                let clean = validate_path_list(&snapshot_paths).map_err(map_backup_err)?;
                let orig = &clean[0];
                let dest_str = match req.mode {
                    BackupRestoreMode::Copy => match copy_target {
                        Some(ref custom) if !custom.trim().is_empty() => {
                            validate_restore_path(custom, false).map_err(map_backup_err)?
                        }
                        _ => default_restored_path(orig, &date_sfx).map_err(map_backup_err)?,
                    },
                    BackupRestoreMode::Replace => {
                        validate_restore_path(orig, true).map_err(map_backup_err)?
                    }
                };
                if req.mode == BackupRestoreMode::Copy && Path::new(&dest_str).exists() {
                    return Err(err(
                        PrivErrorCode::Conflict,
                        format!("Copy restore destination '{dest_str}' already exists."),
                    ));
                }

                ensure_private_dir(&env.backup_cache_dir)?;
                let stage = tempfile::Builder::new()
                    .prefix("opsbox-rst-")
                    .tempdir_in(&env.backup_cache_dir)
                    .map_err(|e| {
                        err(
                            PrivErrorCode::Internal,
                            format!("Could not create restore staging directory: {e}"),
                        )
                    })?;
                let rst_a = restore_args(
                    &backend.repo_uri,
                    pw.path(),
                    &backend.extra_opts,
                    &req.restic_id,
                    stage.path(),
                );
                let out = run_restic(
                    &restic_bin,
                    &env.backup_cache_dir,
                    None,
                    &rst_a,
                    &envs,
                    RESTIC_RUN_TIMEOUT,
                    &scrub,
                )
                .await?;
                if out.code != Some(0) {
                    return Err(err(
                        PrivErrorCode::Internal,
                        restic_failure_msg("restic restore", &out, &scrub),
                    ));
                }

                let src = find_restored_source(stage.path(), &clean);
                let dst = Path::new(&dest_str);
                if req.mode == BackupRestoreMode::Replace && dst.is_dir() {
                    clear_dir_contents(dst)?;
                }
                copy_dir_recursive_safe(&src, dst)?;
                log.push(format!("Restored snapshot files into {dest_str}."));
                dest_str
            }
            BackupRestoreTarget::Volume {
                source_volumes,
                snapshot_paths,
                copy_volume,
                project,
                service_label,
            } => {
                let clean_vols = validate_volume_list(&source_volumes).map_err(map_backup_err)?;
                let orig_vol = &clean_vols[0];
                let target_vol = match req.mode {
                    BackupRestoreMode::Copy => match copy_volume {
                        Some(ref v) if !v.trim().is_empty() => {
                            let clean = validate_volume_list(std::slice::from_ref(v))
                                .map_err(map_backup_err)?;
                            clean[0].clone()
                        }
                        _ => {
                            default_restored_volume(orig_vol, &date_sfx).map_err(map_backup_err)?
                        }
                    },
                    BackupRestoreMode::Replace => orig_vol.clone(),
                };

                ensure_private_dir(&env.backup_cache_dir)?;
                let stage = tempfile::Builder::new()
                    .prefix("opsbox-vrst-")
                    .tempdir_in(&env.backup_cache_dir)
                    .map_err(|e| {
                        err(
                            PrivErrorCode::Internal,
                            format!("Could not create volume restore staging directory: {e}"),
                        )
                    })?;
                let rst_a = restore_args(
                    &backend.repo_uri,
                    pw.path(),
                    &backend.extra_opts,
                    &req.restic_id,
                    stage.path(),
                );
                let out = run_restic(
                    &restic_bin,
                    &env.backup_cache_dir,
                    None,
                    &rst_a,
                    &envs,
                    RESTIC_RUN_TIMEOUT,
                    &scrub,
                )
                .await?;
                if out.code != Some(0) {
                    return Err(err(
                        PrivErrorCode::Internal,
                        restic_failure_msg("restic volume restore", &out, &scrub),
                    ));
                }

                if req.mode == BackupRestoreMode::Copy {
                    env.docker
                        .ensure_volume(&target_vol, &project, &service_label)
                        .await
                        .map_err(|e| map_docker_err(e, env.docker.socket_path()))?;
                }

                let mp = env
                    .docker
                    .inspect_volume_mountpoint(&target_vol)
                    .await
                    .map_err(|e| map_docker_err(e, env.docker.socket_path()))?;
                let dst = validate_live_path(&mp, env)?;

                let mut paused: Vec<String> = Vec::new();
                if req.mode == BackupRestoreMode::Replace {
                    let cids = env
                        .docker
                        .containers_mounting_volumes(std::slice::from_ref(&target_vol))
                        .await
                        .map_err(|e| map_docker_err(e, env.docker.socket_path()))?;
                    for cid in cids {
                        if let Err(e) = env.docker.pause_container(&cid).await {
                            for prev in paused.iter().rev() {
                                let _ = env.docker.unpause_container(prev).await;
                            }
                            return Err(map_docker_err(e, env.docker.socket_path()));
                        }
                        paused.push(cid);
                    }
                    if let Err(e) = clear_dir_contents(&dst) {
                        for prev in paused.iter().rev() {
                            let _ = env.docker.unpause_container(prev).await;
                        }
                        return Err(e);
                    }
                }

                let src = find_restored_source(stage.path(), &snapshot_paths);
                let cp_res = copy_dir_recursive_safe(&src, &dst);
                for prev in paused.iter().rev() {
                    let _ = env.docker.unpause_container(prev).await;
                }
                cp_res?;
                log.push(format!("Restored snapshot into volume {target_vol}."));
                target_vol
            }
            BackupRestoreTarget::Opsbox { copy_target } => {
                let default_copy = env
                    .db_path
                    .parent()
                    .unwrap_or(Path::new("/var/lib/opsbox"))
                    .join(format!("opsbox-restored-{date_sfx}"));
                let dest_str = match copy_target {
                    Some(ref custom) if !custom.trim().is_empty() => {
                        validate_restore_path(custom, false).map_err(map_backup_err)?
                    }
                    _ => default_copy.to_string_lossy().into_owned(),
                };
                if Path::new(&dest_str) == env.db_path || Path::new(&dest_str).exists() {
                    return Err(err(
                        PrivErrorCode::Conflict,
                        format!("opsbox copy restore target '{dest_str}' already exists."),
                    ));
                }

                ensure_private_dir(&env.backup_cache_dir)?;
                let stage = tempfile::Builder::new()
                    .prefix("opsbox-selfrst-")
                    .tempdir_in(&env.backup_cache_dir)
                    .map_err(|e| {
                        err(
                            PrivErrorCode::Internal,
                            format!("Could not create opsbox restore staging directory: {e}"),
                        )
                    })?;
                let rst_a = restore_args(
                    &backend.repo_uri,
                    pw.path(),
                    &backend.extra_opts,
                    &req.restic_id,
                    stage.path(),
                );
                let out = run_restic(
                    &restic_bin,
                    &env.backup_cache_dir,
                    None,
                    &rst_a,
                    &envs,
                    RESTIC_RUN_TIMEOUT,
                    &scrub,
                )
                .await?;
                if out.code != Some(0) {
                    return Err(err(
                        PrivErrorCode::Internal,
                        restic_failure_msg("restic restore (opsbox)", &out, &scrub),
                    ));
                }

                if contains_master_key_file(stage.path()) {
                    return Err(err(
                        PrivErrorCode::Internal,
                        "Unexpected master.key found in restored archive.",
                    ));
                }

                let dst_root = Path::new(&dest_str);
                ensure_private_dir(dst_root)?;

                let restored_db = find_opsbox_db_in_tree(stage.path(), 4).ok_or_else(|| {
                    err(
                        PrivErrorCode::Internal,
                        "Restored archive did not contain opsbox.db.",
                    )
                })?;
                let dst_db = dst_root.join("opsbox.db");
                fs::copy(&restored_db, &dst_db).map_err(|e| {
                    err(
                        PrivErrorCode::Internal,
                        format!("Could not copy restored opsbox.db: {e}"),
                    )
                })?;
                let _ = fs::set_permissions(&dst_db, fs::Permissions::from_mode(0o600));
                if let Some(parent) = restored_db.parent() {
                    let etc_src = parent.join("etc-opsbox");
                    if etc_src.is_dir() {
                        copy_dir_recursive_safe(&etc_src, &dst_root.join("etc-opsbox"))?;
                    }
                }
                ReadOnlyStore::open(&dst_db)
                    .map_err(|e| e.to_string())
                    .and_then(|ro| ro.check_integrity())
                    .map_err(|e| {
                        err(
                            PrivErrorCode::Internal,
                            format!("Restored opsbox.db integrity check failed: {e}"),
                        )
                    })?;
                log.push(format!(
                    "Restored opsbox state copy to {dest_str} and verified SQLite integrity."
                ));
                dest_str
            }
        };

        let message = match req.mode {
            BackupRestoreMode::Copy => format!("Restored snapshot into copy: {target_summary}."),
            BackupRestoreMode::Replace => {
                format!("Replaced {target_summary} cleanly after taking a live safety snapshot.")
            }
        };
        Ok(BackupRestoreResult {
            ok: true,
            mode: req.mode,
            target_summary,
            message,
            safety_snapshot_id: safety_id,
            safety_snapshot_short_id: safety_short,
            log,
        })
    })?
}

/// Restore a snapshot into an ephemeral temporary container or directory, run a real
/// sanity check, tear down the temporary resources, and report pass/fail.
pub fn restore_test_with(
    env: &Env,
    req: BackupRestoreTestRequest,
) -> Result<BackupRestoreTestResult, PrivError> {
    if !opsbox_proto::valid_restic_snapshot_id(&req.restic_id) {
        return Err(err(PrivErrorCode::Invalid, "Invalid snapshot ID."));
    }
    let restic_bin = find_restic_bin(env)?;
    let backend = resolve_backend_conn(env, &req.destination_id, &req.repo)?;
    let secret = derive_pw(&env.secrets)?;
    let repo_mutex = repo_lock(&backend.lock_key);

    block_on_fut(async move {
        let _guard = repo_mutex.lock().await;
        let pw = PasswordFileGuard::create(&env.backup_cache_dir, secret)?;
        let envs = backend.extra_env_pairs();
        let scrub = build_scrub_refs(&pw, &backend);
        let mut log = Vec::new();

        match req.target {
            BackupRestoreTestTarget::Files { is_opsbox_self } => {
                ensure_private_dir(&env.backup_cache_dir)?;
                let stage = tempfile::Builder::new()
                    .prefix("opsbox-rtest-")
                    .tempdir_in(&env.backup_cache_dir)
                    .map_err(|e| {
                        err(
                            PrivErrorCode::Internal,
                            format!("Could not create restore-test directory: {e}"),
                        )
                    })?;
                let rst_a = restore_args(
                    &backend.repo_uri,
                    pw.path(),
                    &backend.extra_opts,
                    &req.restic_id,
                    stage.path(),
                );
                let out = run_restic(
                    &restic_bin,
                    &env.backup_cache_dir,
                    None,
                    &rst_a,
                    &envs,
                    RESTIC_RUN_TIMEOUT,
                    &scrub,
                )
                .await?;
                if out.code != Some(0) {
                    return Ok(BackupRestoreTestResult {
                        ok: false,
                        summary: restic_failure_msg("restic restore test", &out, &scrub),
                        log,
                    });
                }
                if is_opsbox_self {
                    let Some(db) = find_opsbox_db_in_tree(stage.path(), 4) else {
                        return Ok(BackupRestoreTestResult {
                            ok: false,
                            summary: "Restore test failed: opsbox.db missing in snapshot.".into(),
                            log,
                        });
                    };
                    if let Err(e) = ReadOnlyStore::open(&db)
                        .map_err(|e| e.to_string())
                        .and_then(|ro| ro.check_integrity())
                    {
                        return Ok(BackupRestoreTestResult {
                            ok: false,
                            summary: format!(
                                "Restore test SQLite PRAGMA integrity_check failed: {e}"
                            ),
                            log,
                        });
                    }
                    log.push("Verified restored opsbox.db with PRAGMA integrity_check.".into());
                    return Ok(BackupRestoreTestResult {
                        ok: true,
                        summary: "Restore test passed (opsbox.db SQLite integrity verified)."
                            .into(),
                        log,
                    });
                }
                let has_entries = fs::read_dir(stage.path())
                    .map(|mut d| d.next().is_some())
                    .unwrap_or(false);
                if !has_entries {
                    return Ok(BackupRestoreTestResult {
                        ok: false,
                        summary: "Restore test extracted 0 files from snapshot.".into(),
                        log,
                    });
                }
                log.push("Verified snapshot extraction into temporary directory.".into());
                Ok(BackupRestoreTestResult {
                    ok: true,
                    summary: "Restore test passed (snapshot extracted cleanly).".into(),
                    log,
                })
            }
            BackupRestoreTestTarget::Database {
                template,
                version,
                stdin_filename,
                db_user,
                db_name,
            } => {
                let tmpl = get_template(template.as_str()).ok_or_else(|| {
                    err(
                        PrivErrorCode::Invalid,
                        format!("Unknown database template '{}'.", template.as_str()),
                    )
                })?;
                let tag = tmpl
                    .versions
                    .iter()
                    .find(|v| *v == &version)
                    .cloned()
                    .or_else(|| tmpl.versions.first().cloned())
                    .unwrap_or_else(|| "latest".to_owned());
                let image_ref = format!("{}:{tag}", tmpl.image);
                let suffix = unique_test_suffix();
                let service = format!("rtest-{suffix}");
                let cname = format!("opsbox-rtest-{suffix}");
                let vol_name = format!("opsbox-vol-rtest-{suffix}");
                let ephem_pw = generate_ephemeral_token();
                let u = db_user
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("app");
                let db = db_name
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("app");

                let kv: Vec<(String, String)> = match template {
                    BackupDbKind::Postgres => vec![
                        ("POSTGRES_USER".into(), u.to_owned()),
                        ("POSTGRES_DB".into(), db.to_owned()),
                        ("POSTGRES_PASSWORD".into(), ephem_pw.clone()),
                    ],
                    BackupDbKind::Mysql => vec![
                        ("MYSQL_DATABASE".into(), db.to_owned()),
                        ("MYSQL_ROOT_PASSWORD".into(), ephem_pw.clone()),
                        ("MYSQL_USER".into(), u.to_owned()),
                        ("MYSQL_PASSWORD".into(), ephem_pw.clone()),
                    ],
                    BackupDbKind::Mariadb => vec![
                        ("MARIADB_DATABASE".into(), db.to_owned()),
                        ("MARIADB_ROOT_PASSWORD".into(), ephem_pw.clone()),
                        ("MARIADB_USER".into(), u.to_owned()),
                        ("MARIADB_PASSWORD".into(), ephem_pw.clone()),
                    ],
                    BackupDbKind::Redis => vec![("REDIS_PASSWORD".into(), ephem_pw.clone())],
                    BackupDbKind::Mongo => vec![
                        ("MONGO_INITDB_ROOT_USERNAME".into(), u.to_owned()),
                        ("MONGO_INITDB_ROOT_PASSWORD".into(), ephem_pw.clone()),
                    ],
                };
                let env_strings: Vec<String> = kv.iter().map(|(k, v)| format!("{k}={v}")).collect();
                let vol_binds: Vec<(String, String)> = tmpl
                    .volumes
                    .first()
                    .map(|v| vec![(vol_name.clone(), v.container_path.clone())])
                    .unwrap_or_default();

                let mut scrub_owned: Vec<String> = scrub.iter().map(|s| (*s).to_owned()).collect();
                scrub_owned.push(ephem_pw);
                let scrub_refs: Vec<&str> = scrub_owned.iter().map(String::as_str).collect();

                let spec = TemplateContainerSpec {
                    project: "rtest",
                    service: &service,
                    template_id: &tmpl.id,
                    deploy_id: &suffix,
                    container_name: &cname,
                    network_name: "opsbox-rtest",
                    image_ref: &image_ref,
                    env: &env_strings,
                    cmd: &tmpl.cmd,
                    volume_binds: &vol_binds,
                    container_port: tmpl.port,
                    port_bind: PortBindMode::InternalOnly,
                    memory_limit_mb: Some(512),
                    cpu_limit_millis: None,
                };

                let _ = env.docker.pull_image(&image_ref).await;
                let (cid, host_port, container_ip) =
                    match env.docker.create_and_start_template_container(&spec).await {
                        Ok(v) => v,
                        Err(e) => {
                            let _ = env.docker.remove_volume(&vol_name).await;
                            return Ok(BackupRestoreTestResult {
                                ok: false,
                                summary: format!(
                                    "Could not start temporary test database container: {}",
                                    scrub_secrets(&e.to_string(), &scrub_refs)
                                ),
                                log,
                            });
                        }
                    };

                let res = async {
                    wait_for_template_health(
                        &env.docker,
                        &cid,
                        tmpl,
                        true,
                        host_port,
                        container_ip.as_deref(),
                        &scrub_refs,
                    )
                    .await
                    .map_err(|r| err(PrivErrorCode::Internal, r))?;
                    restore_database_into_container(
                        env,
                        &restic_bin,
                        &backend,
                        &pw,
                        &req.restic_id,
                        template,
                        &service,
                        &cid,
                        &stdin_filename,
                        &kv,
                        &scrub_owned,
                        &mut log,
                    )
                    .await
                }
                .await;

                // Always tear down the temporary container and volume.
                let _ = env.docker.force_stop_and_remove_container(&cid).await;
                for (vname, _) in &vol_binds {
                    let _ = env.docker.remove_volume(vname).await;
                }

                match res {
                    Ok(query_out) => Ok(BackupRestoreTestResult {
                        ok: true,
                        summary: format!(
                            "Restore test passed ({} ephemeral container sanity query: {query_out}).",
                            tmpl.name
                        ),
                        log,
                    }),
                    Err(e) => Ok(BackupRestoreTestResult {
                        ok: false,
                        summary: format!("Restore test failed: {}", e.message),
                        log,
                    }),
                }
            }
        }
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::files;
    use crate::secrets::Secrets;
    use opsbox_docker::DockerClient;
    use opsbox_store::Store;
    use tempfile::TempDir;

    fn write_mock_restic(dir: &Path) -> PathBuf {
        let script = dir.join("restic");
        // Stateful bash-less POSIX sh mock restic that verifies `--password-file` is passed
        // and records backed-up filenames / stdin contents for assertions.
        let body = r#"#!/bin/sh
set -eu
repo=""
pw=""
sub=""
stdin_mode=0
stdin_name=""
restore_target=""
restore_snap=""
args_log="${RESTIC_TEST_LOG:-}"
prev=""
for a in "$@"; do
  case "$prev" in
    -r) repo="$a"; prev=""; continue ;;
    --password-file) pw="$a"; prev=""; continue ;;
    --stdin-filename) stdin_name="$a"; prev=""; continue ;;
    --target) restore_target="$a"; prev=""; continue ;;
    -o|--tag|--exclude|--group-by|--keep-daily|--keep-weekly) prev=""; continue ;;
  esac
  case "$a" in
    -r|--password-file|--stdin-filename|--target|-o|--tag|--exclude|--group-by|--keep-daily|--keep-weekly)
      prev="$a"
      ;;
    --stdin) stdin_mode=1 ;;
    version) sub="version" ;;
    init) sub="init" ;;
    cat) sub="cat" ;;
    backup) sub="backup" ;;
    snapshots) sub="snapshots" ;;
    forget) sub="forget" ;;
    check) sub="check" ;;
    dump) sub="dump" ;;
    restore) sub="restore" ;;
    *)
      if [ "$sub" = "restore" ] && [ -z "$restore_snap" ]; then
        restore_snap="$a"
      fi
      ;;
  esac
done

if [ "$sub" = "version" ]; then
  echo "restic 0.19.1 compiled with go1.24.2 on linux/amd64"
  exit 0
fi

if [ -z "$pw" ] || [ ! -f "$pw" ]; then
  echo "missing password file" >&2
  exit 2
fi
if [ -n "$args_log" ]; then
  printf "%s\n" "$*" >> "$args_log"
fi

case "$repo" in
  s3:*)
    if [ -z "${AWS_ACCESS_KEY_ID:-}" ] || [ -z "${AWS_SECRET_ACCESS_KEY:-}" ]; then
      echo "missing S3 env creds" >&2
      exit 3
    fi
    repo_fs="${RESTIC_CACHE_DIR}/mock-s3-$(printf '%s' "$repo" | tr -c 'A-Za-z0-9._-' '_')"
    ;;
  sftp:*)
    repo_fs="${RESTIC_CACHE_DIR}/mock-sftp-$(printf '%s' "$repo" | tr -c 'A-Za-z0-9._-' '_')"
    ;;
  *)
    repo_fs="$repo"
    ;;
esac

case "$sub" in
  cat)
    if [ ! -f "$repo_fs/config" ]; then
      echo "Fatal: unable to open config file: does not exist" >&2
      exit 10
    fi
    cat "$repo_fs/config"
    ;;
  init)
    mkdir -p "$repo_fs"
    echo "{}" > "$repo_fs/config"
    echo "{\"message_type\":\"initialized\",\"id\":\"testrepo\",\"repository\":\"$repo\"}"
    ;;
  backup)
    mkdir -p "$repo_fs"
    if [ ! -f "$repo_fs/config" ]; then
      echo "not initialized" >&2
      exit 1
    fi
    if [ "$stdin_mode" -eq 1 ]; then
      cat > "$repo_fs/last_stdin_payload"
      bytes=$(wc -c < "$repo_fs/last_stdin_payload" | tr -d ' ')
      printf "%s:%s\n" "$stdin_name" "$bytes" > "$repo_fs/last_stdin"
      echo "{\"message_type\":\"summary\",\"snapshot_id\":\"43deae25000000000000000000000001\",\"total_bytes_processed\":$bytes,\"data_added\":$bytes,\"total_files_processed\":1}"
    else
      if [ -f "opsbox.db" ] && [ -f "etc-opsbox/config.toml" ]; then
        cp "opsbox.db" "$repo_fs/captured_opsbox.db"
        if [ -f "etc-opsbox/master.key" ]; then
          echo "MASTER_KEY_LEAKED" >&2
          exit 9
        fi
        cp "etc-opsbox/config.toml" "$repo_fs/captured_config.toml"
        sid="aaaa1111000000000000000000000002"
      else
        seq_file="$repo_fs/seq"
        seq=$(cat "$seq_file" 2>/dev/null || echo 1)
        seq=$((seq + 1))
        echo "$seq" > "$seq_file"
        sid=$(printf "aaaa11110000000000000000000000%02x" "$seq")
        snap_dir="$repo_fs/snaps/$sid"
        mkdir -p "$snap_dir"
        for src in "$@"; do
          case "$src" in
            /*)
              if [ -e "$src" ]; then
                mkdir -p "$snap_dir$(dirname "$src")"
                cp -R "$src" "$snap_dir$src"
              fi
              ;;
          esac
        done
      fi
      echo "{\"message_type\":\"summary\",\"snapshot_id\":\"$sid\",\"total_bytes_processed\":4096,\"data_added\":2048,\"total_files_processed\":3}"
    fi
    ;;
  snapshots)
    echo "[{\"id\":\"43deae25000000000000000000000001\",\"short_id\":\"43deae25\",\"time\":\"2026-11-18T02:00:05Z\",\"hostname\":\"vm\",\"paths\":[\"/pg.fc\"],\"tags\":[\"opsbox-job:job1\",\"opsbox-kind:database\",\"opsbox-what:pg\"],\"summary\":{\"total_bytes_processed\":1024}}]"
    ;;
  forget)
    if [ "${RESTIC_FAIL_FORGET:-0}" = "1" ]; then
      echo "AccessDenied: s3:DeleteObject denied" >&2
      exit 1
    fi
    echo "[{\"keep\":[{\"id\":\"aaaa1111\",\"short_id\":\"aaaa1111\"}],\"remove\":[{\"id\":\"bbbb2222\",\"short_id\":\"bbbb2222\"}]}]"
    ;;
  check)
    if [ "${RESTIC_FAIL_CHECK:-0}" = "1" ]; then
      echo "{\"message_type\":\"summary\",\"num_errors\":1,\"broken_packs\":[\"pack123\"]}"
      exit 1
    fi
    echo "{\"message_type\":\"summary\",\"num_errors\":0,\"broken_packs\":[]}"
    ;;
  dump)
    if [ -f "$repo_fs/last_stdin_payload" ]; then
      cat "$repo_fs/last_stdin_payload"
    else
      echo "missing payload" >&2
      exit 1
    fi
    ;;
  restore)
    mkdir -p "$restore_target"
    if [ -n "$restore_snap" ] && [ -d "$repo_fs/snaps/$restore_snap" ]; then
      cp -R "$repo_fs/snaps/$restore_snap/." "$restore_target/"
    fi
    if [ -f "$repo_fs/captured_opsbox.db" ]; then
      cp "$repo_fs/captured_opsbox.db" "$restore_target/opsbox.db"
      mkdir -p "$restore_target/etc-opsbox"
      cp "$repo_fs/captured_config.toml" "$restore_target/etc-opsbox/config.toml"
    fi
    ;;
esac
"#;
        fs::write(&script, body).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    fn make_env(tmp: &Path, restic_bin: Option<PathBuf>) -> Env {
        let etc = tmp.join("etc-opsbox");
        let var = tmp.join("var-lib");
        fs::create_dir_all(&etc).unwrap();
        fs::create_dir_all(&var).unwrap();
        let master_key = etc.join("master.key");
        fs::write(&master_key, [9u8; 32]).unwrap();
        fs::set_permissions(&master_key, fs::Permissions::from_mode(0o600)).unwrap();
        let config_path = etc.join("config.toml");
        fs::write(&config_path, "listen = \"127.0.0.1:8443\"\n").unwrap();
        let db_path = var.join("opsbox.db");
        let store = Store::open(&db_path).unwrap();
        store
            .set_setting("backup_test_probe", &serde_json::json!("alive"))
            .unwrap();
        drop(store);

        let apps_dir = var.join("apps");
        fs::create_dir_all(&apps_dir).unwrap();
        let backup_dir = tmp.join("backups");
        let cache_dir = tmp.join("cache");
        let xfer = var.join("xfer");
        let trash = var.join("trash");
        fs::create_dir_all(&xfer).unwrap();
        fs::create_dir_all(&trash).unwrap();

        Env {
            capacity: Default::default(),
            capacity_profile: None,
            capacity_image_size_mb: None,
            secrets: Secrets::new(&master_key),
            master_key,
            config_path,
            files: files::Dirs { xfer, trash },
            apps_dir,
            docker: DockerClient::new("/nonexistent/docker.sock"),
            docker_bin: None,
            db_path,
            backup_dir,
            backup_cache_dir: cache_dir,
            restic_bin,
            allow_test_git: false,
            network_proc_root: "/proc".into(),
            network_systemctl: None,
        }
    }

    #[test]
    fn opsbox_self_backup_uses_sqlite_online_backup_and_strictly_excludes_master_key() {
        let tmp = TempDir::new().unwrap();
        let restic = write_mock_restic(tmp.path());
        let env = make_env(tmp.path(), Some(restic));

        let info = local_info_with(&env, &[]).unwrap();
        assert!(info.restic_installed);
        assert_eq!(info.restic_version.as_deref(), Some("0.19.1"));
        assert!(info.same_disk);
        assert!(
            info.same_disk_paths
                .iter()
                .any(|p| p.ends_with("opsbox.db"))
        );

        let res = run_backup_with(
            &env,
            BackupRunSpec {
                destination_id: "local".into(),
                repo: BackupRepoTarget::Local,
                job_id: "opsbox-self".into(),
                job_name: "opsbox config & state".into(),
                target: BackupRunTarget::Opsbox,
                retain_daily: 14,
                retain_weekly: 8,
                run_prune: true,
                run_check: true,
            },
        )
        .unwrap();
        assert_eq!(res.short_id, "aaaa1111");
        assert_eq!(res.pruned_count, 1);
        assert_eq!(res.check_state, BackupCheckState::ChecksumOk);

        let captured_db = env.backup_dir.join("captured_opsbox.db");
        assert!(captured_db.is_file());
        let ro = ReadOnlyStore::open(&captured_db).unwrap();
        assert_eq!(
            ro.get_setting("backup_test_probe").unwrap(),
            Some(serde_json::json!("alive"))
        );
        assert!(env.backup_dir.join("captured_config.toml").is_file());

        // Ephemeral password dir must be cleaned up.
        let leftover: Vec<_> = fs::read_dir(&env.backup_cache_dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with("opsbox-"))
            .collect();
        assert!(leftover.is_empty());

        let snaps = list_snapshots_with(
            &env,
            BackupSnapshotsRequest {
                destination_id: "local".into(),
                repo: BackupRepoTarget::Local,
            },
        )
        .unwrap();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].job_id.as_deref(), Some("job1"));

        let chk = check_with(
            &env,
            BackupCheckRequest {
                destination_id: "local".into(),
                repo: BackupRepoTarget::Local,
                read_data_subset: Some("10%".into()),
            },
        )
        .unwrap();
        assert!(chk.ok);

        // Opsbox self-copy restore & SQLite integrity verify.
        let copy_target = tmp.path().join("opsbox-copy-restore");
        let self_rst = restore_with(
            &env,
            BackupRestoreRequest {
                destination_id: "local".into(),
                repo: BackupRepoTarget::Local,
                restic_id: "aaaa1111".into(),
                mode: BackupRestoreMode::Copy,
                confirm_replace: None,
                target: BackupRestoreTarget::Opsbox {
                    copy_target: Some(copy_target.display().to_string()),
                },
                job_id: Some("opsbox-self".into()),
            },
        )
        .unwrap();
        assert!(self_rst.ok);
        assert!(copy_target.join("opsbox.db").is_file());
        assert!(!contains_master_key_file(&copy_target));

        // Opsbox replace-in-place must ALWAYS be refused.
        let self_replace_err = restore_with(
            &env,
            BackupRestoreRequest {
                destination_id: "local".into(),
                repo: BackupRepoTarget::Local,
                restic_id: "aaaa1111".into(),
                mode: BackupRestoreMode::Replace,
                confirm_replace: Some("replace".into()),
                target: BackupRestoreTarget::Opsbox { copy_target: None },
                job_id: None,
            },
        )
        .unwrap_err();
        assert_eq!(self_replace_err.code, PrivErrorCode::Refused);

        // Automated restore test on opsbox.db verifies SQLite PRAGMA integrity_check.
        let rtest = restore_test_with(
            &env,
            BackupRestoreTestRequest {
                destination_id: "local".into(),
                repo: BackupRepoTarget::Local,
                restic_id: "aaaa1111".into(),
                target: BackupRestoreTestTarget::Files {
                    is_opsbox_self: true,
                },
            },
        )
        .unwrap();
        assert!(rtest.ok);
    }

    #[test]
    fn refuses_master_key_dotdot_and_missing_remote_repo_config() {
        let tmp = TempDir::new().unwrap();
        let restic = write_mock_restic(tmp.path());
        let env = make_env(tmp.path(), Some(restic));

        let err_mk = run_backup_with(
            &env,
            BackupRunSpec {
                destination_id: "local".into(),
                repo: BackupRepoTarget::Local,
                job_id: "bad1".into(),
                job_name: "bad".into(),
                target: BackupRunTarget::Path {
                    paths: vec![env.master_key.display().to_string()],
                },
                retain_daily: 7,
                retain_weekly: 4,
                run_prune: false,
                run_check: false,
            },
        )
        .unwrap_err();
        assert_eq!(err_mk.code, PrivErrorCode::Invalid);

        let err_rel = run_backup_with(
            &env,
            BackupRunSpec {
                destination_id: "local".into(),
                repo: BackupRepoTarget::Local,
                job_id: "bad2".into(),
                job_name: "bad".into(),
                target: BackupRunTarget::Path {
                    paths: vec!["/etc/../etc/opsbox".into()],
                },
                retain_daily: 7,
                retain_weekly: 4,
                run_prune: false,
                run_check: false,
            },
        )
        .unwrap_err();
        assert_eq!(err_rel.code, PrivErrorCode::Invalid);

        // Remote destination ID without remote BackupRepoTarget is rejected cleanly.
        let err_remote = run_backup_with(
            &env,
            BackupRunSpec {
                destination_id: "s3-prod".into(),
                repo: BackupRepoTarget::Local,
                job_id: "bad3".into(),
                job_name: "bad".into(),
                target: BackupRunTarget::Opsbox,
                retain_daily: 7,
                retain_weekly: 4,
                run_prune: false,
                run_check: false,
            },
        )
        .unwrap_err();
        assert_eq!(err_remote.code, PrivErrorCode::Invalid);
    }

    #[test]
    fn s3_destination_test_sftp_keygen_recovery_key_and_restore_safety() {
        let tmp = TempDir::new().unwrap();
        let restic = write_mock_restic(tmp.path());
        let env = make_env(tmp.path(), Some(restic));

        // 1. Pre-save S3 test passes credentials via env vars, never argv.
        let test_res = test_destination_with(
            &env,
            BackupDestinationTestRequest {
                provider: opsbox_proto::BackupProviderId::R2,
                target: BackupDestinationTestTarget::Restic {
                    repo: BackupRepoTarget::S3 {
                        destination_id: "probe".into(),
                        endpoint: "https://acct1.r2.cloudflarestorage.com".into(),
                        bucket: "opsbox-bk".into(),
                        repo_path: "srv-1".into(),
                        region: Some("auto".into()),
                        credentials: BackupS3Credentials::Plaintext {
                            access_key_id: Secret::new("AKIA_TEST_123".to_owned()),
                            secret_access_key: Secret::new("SK_SUPER_SECRET_XYZ".to_owned()),
                        },
                    },
                },
            },
        )
        .unwrap();
        assert!(test_res.ok);
        assert!(test_res.verified);
        assert_eq!(test_res.steps.len(), 3);
        assert!(
            test_res
                .steps
                .iter()
                .all(|s| s.status == BackupTestStepStatus::Ok)
        );

        // 2. Sealed credentials work in S3 snapshot/backup ops & keep secrets off argv.
        let sealed_s3 = sealed::seal(
            &env.secrets,
            "backup:dest:dest-1",
            &Secret::new(
                r#"{"accessKeyId":"AKIA_SEALED","secretAccessKey":"SK_SEALED_XYZ"}"#.to_owned(),
            ),
        )
        .unwrap();
        let opsbox_proto::PrivReply::Sealed { sealed: value_enc } = sealed_s3 else {
            panic!("expected Sealed");
        };
        let repo = BackupRepoTarget::S3 {
            destination_id: "dest-1".into(),
            endpoint: "https://acct1.r2.cloudflarestorage.com".into(),
            bucket: "opsbox-bk".into(),
            repo_path: "srv-1".into(),
            region: Some("auto".into()),
            credentials: BackupS3Credentials::Sealed {
                seal_context: "backup:dest:dest-1".into(),
                value_enc,
            },
        };
        let data_dir = tmp.path().join("appdata");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("hello.txt"), b"v1-original").unwrap();

        let bk = run_backup_with(
            &env,
            BackupRunSpec {
                destination_id: "dest-1".into(),
                repo: repo.clone(),
                job_id: "pathjob".into(),
                job_name: "app data".into(),
                target: BackupRunTarget::Path {
                    paths: vec![data_dir.display().to_string()],
                },
                retain_daily: 7,
                retain_weekly: 4,
                run_prune: false,
                run_check: false,
            },
        )
        .unwrap();
        assert_eq!(bk.short_id, "aaaa1111");

        // 3. Recovery key matches deterministic derive_pw().
        let rec = recovery_key_with(&env).unwrap();
        assert_eq!(
            rec.recovery_key.expose(),
            derive_pw(&env.secrets).unwrap().as_str()
        );

        // 4. SFTP key generation seals private key and exposes only public key.
        let sftp_key = generate_sftp_key_with(
            &env,
            &BackupGenerateSftpKeyRequest {
                destination_id: "sftp-1".into(),
            },
        )
        .unwrap();
        assert!(sftp_key.public_key.starts_with("ssh-ed25519 "));
        assert!(sftp_key.fingerprint.starts_with("SHA256:"));
        assert!(!sftp_key.private_key_enc.contains("OPENSSH PRIVATE KEY"));

        // 5. Copy restore writes to new folder without touching live data_dir.
        fs::write(data_dir.join("hello.txt"), b"v2-modified").unwrap();
        let copy_res = restore_with(
            &env,
            BackupRestoreRequest {
                destination_id: "dest-1".into(),
                repo: repo.clone(),
                restic_id: bk.snapshot_id.clone(),
                mode: BackupRestoreMode::Copy,
                confirm_replace: None,
                target: BackupRestoreTarget::Path {
                    snapshot_paths: vec![data_dir.display().to_string()],
                    copy_target: None,
                },
                job_id: Some("pathjob".into()),
            },
        )
        .unwrap();
        assert!(copy_res.ok);
        assert!(copy_res.safety_snapshot_id.is_none());
        assert_eq!(
            fs::read(data_dir.join("hello.txt")).unwrap(),
            b"v2-modified"
        );
        let restored_copy = PathBuf::from(&copy_res.target_summary);
        assert_eq!(
            fs::read(restored_copy.join("hello.txt")).unwrap(),
            b"v1-original"
        );

        // 6. Replace restore refuses without exact typed "replace" confirmation.
        let missing_confirm = restore_with(
            &env,
            BackupRestoreRequest {
                destination_id: "dest-1".into(),
                repo: repo.clone(),
                restic_id: bk.snapshot_id.clone(),
                mode: BackupRestoreMode::Replace,
                confirm_replace: Some("REPLACE".into()),
                target: BackupRestoreTarget::Path {
                    snapshot_paths: vec![data_dir.display().to_string()],
                    copy_target: None,
                },
                job_id: Some("pathjob".into()),
            },
        )
        .unwrap_err();
        assert_eq!(missing_confirm.code, PrivErrorCode::ConfirmRequired);
        // Live data untouched on refusal!
        assert_eq!(
            fs::read(data_dir.join("hello.txt")).unwrap(),
            b"v2-modified"
        );

        // 7. Replace restore with exact "replace" takes a safety snapshot FIRST, then restores.
        let rep_res = restore_with(
            &env,
            BackupRestoreRequest {
                destination_id: "dest-1".into(),
                repo,
                restic_id: bk.snapshot_id,
                mode: BackupRestoreMode::Replace,
                confirm_replace: Some("replace".into()),
                target: BackupRestoreTarget::Path {
                    snapshot_paths: vec![data_dir.display().to_string()],
                    copy_target: None,
                },
                job_id: Some("pathjob".into()),
            },
        )
        .unwrap();
        assert!(rep_res.ok);
        assert!(rep_res.safety_snapshot_id.is_some());
        assert_eq!(
            fs::read(data_dir.join("hello.txt")).unwrap(),
            b"v1-original"
        );
    }
}
