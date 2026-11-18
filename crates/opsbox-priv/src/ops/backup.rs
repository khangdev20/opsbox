//! Privileged backup engine: restic invocation, password derivation, database dump
//! streaming, Docker volume snapshots, opsbox self-backup (SQLite online backup +
//! `/etc/opsbox` excluding `master.key`), and local-disk inspection
//! (docs/BACKEND_SPEC.md 7.12).

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use opsbox_backup::BackupError;
use opsbox_backup::dumps::build_dump_spec;
use opsbox_backup::restic::{
    backup_paths_args, backup_stdin_args, check_args, forget_args, forget_single_snapshot_args,
    init_args, job_tags, parse_backup_summary, parse_check_output, parse_forget_output,
    parse_snapshots, parse_version, snapshots_args,
};
use opsbox_backup::validation::{
    validate_abs_path, validate_path_list, validate_read_data_subset, validate_retention,
    validate_volume_list,
};
use opsbox_deploy::pipeline::scrub_secrets;
use opsbox_proto::{
    BackupCheckRequest, BackupCheckResult, BackupCheckState, BackupForgetRequest,
    BackupForgetResult, BackupLocalInfo, BackupRunResult, BackupRunSpec, BackupRunTarget,
    BackupSnapshotsRequest, BackupTargetKind, DockerExecRequest, PrivError, PrivErrorCode,
    ResticSnapshotInfo, valid_backup_id, valid_docker_ref, valid_service_name,
};
use opsbox_store::ReadOnlyStore;
use tokio::io::AsyncReadExt as _;
use tokio::process::Command;
use zeroize::Zeroizing;

use super::Env;
use super::deploy::{err, find_binary, open_deploy_vars};
use super::docker::{block_on_fut, map_docker_err};

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

fn repo_lock(repo_dir: &Path) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<std::sync::Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> =
        OnceLock::new();
    let map = LOCKS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = map
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Arc::clone(
        guard
            .entry(repo_dir.to_path_buf())
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

fn resolve_local_repo_dir(env: &Env, destination_id: &str) -> Result<PathBuf, PrivError> {
    if !valid_backup_id(destination_id) {
        return Err(err(
            PrivErrorCode::Invalid,
            "Invalid backup destination identifier.",
        ));
    }
    if destination_id != "local" {
        return Err(err(
            PrivErrorCode::Unsupported,
            "Remote backup destinations are not enabled yet; use the local repository.",
        ));
    }
    Ok(env.backup_dir.clone())
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

fn clean_restic_command(restic_bin: &Path, cache_dir: &Path, args: &[String]) -> Command {
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
    cmd
}

async fn run_restic(
    restic_bin: &Path,
    cache_dir: &Path,
    current_dir: Option<&Path>,
    args: &[String],
    timeout: Duration,
    scrub: &[&str],
) -> Result<ResticCmdOutput, PrivError> {
    let mut cmd = clean_restic_command(restic_bin, cache_dir, args);
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

async fn ensure_repo_initialized(
    restic_bin: &Path,
    repo_dir: &Path,
    cache_dir: &Path,
    pw: &PasswordFileGuard,
) -> Result<bool, PrivError> {
    ensure_private_dir(repo_dir)?;
    if repo_dir.join("config").is_file() {
        return Ok(false);
    }
    let args = init_args(repo_dir, pw.path());
    let scrub = [pw.secret()];
    let out = run_restic(
        restic_bin,
        cache_dir,
        None,
        &args,
        RESTIC_META_TIMEOUT,
        &scrub,
    )
    .await?;
    if out.code != Some(0) {
        if repo_dir.join("config").is_file() {
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
    repo_dir: &Path,
    cache_dir: &Path,
    pw: &PasswordFileGuard,
    stdout: &str,
) {
    if let Ok(sum) = parse_backup_summary(stdout) {
        let args = forget_single_snapshot_args(repo_dir, pw.path(), &sum.snapshot_id);
        let scrub = [pw.secret()];
        let _ = run_restic(
            restic_bin,
            cache_dir,
            None,
            &args,
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
    let repo_dir = resolve_local_repo_dir(env, &req.destination_id)?;
    let secret = derive_pw(&env.secrets)?;

    let started = Instant::now();
    let repo_mutex = repo_lock(&repo_dir);

    block_on_fut(async move {
        let _repo_guard = repo_mutex.lock().await;
        let pw = PasswordFileGuard::create(&env.backup_cache_dir, secret)?;
        let mut log = Vec::new();

        if ensure_repo_initialized(&restic_bin, &repo_dir, &env.backup_cache_dir, &pw).await? {
            log.push(format!(
                "Initialized encrypted local restic repository at {}.",
                repo_dir.display()
            ));
        }

        let summary = match req.target {
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
                let kv: Vec<(String, String)> =
                    opened.into_iter().map(|v| (v.key, v.value)).collect();
                let dump_spec =
                    build_dump_spec(template, &service_name, &kv).map_err(map_backup_err)?;

                let mut scrub_owned = deploy_scrub;
                scrub_owned.extend(dump_spec.scrub_secrets);
                scrub_owned.push(pw.secret().to_owned());
                let scrub_refs: Vec<&str> = scrub_owned.iter().map(String::as_str).collect();

                log.push(format!(
                    "Streaming {} dump from container {} ({}) directly into restic.",
                    template.method_name(),
                    service_name,
                    container_id
                ));

                let tags = job_tags(&req.job_id, BackupTargetKind::Database, &service_name);
                let args =
                    backup_stdin_args(&repo_dir, pw.path(), &dump_spec.stdin_filename, &tags);
                let mut cmd = clean_restic_command(&restic_bin, &env.backup_cache_dir, &args);
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

                let exec_fut =
                    env.docker
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
                            &restic_bin,
                            &repo_dir,
                            &env.backup_cache_dir,
                            &pw,
                            &stdout,
                        )
                        .await;
                        return Err(map_docker_err(e, env.docker.socket_path()));
                    }
                };
                if exec_status.exit_code != 0 || exec_status.bytes_written == 0 {
                    discard_partial_snapshot(
                        &restic_bin,
                        &repo_dir,
                        &env.backup_cache_dir,
                        &pw,
                        &stdout,
                    )
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
                parse_backup_summary(&stdout).map_err(map_backup_err)?
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
                let tags = job_tags(&req.job_id, BackupTargetKind::Volume, &what);
                let excludes = backup_excludes(env);
                let args = backup_paths_args(&repo_dir, pw.path(), &mount_paths, &excludes, &tags);
                let scrub = [pw.secret()];
                let run_res = run_restic(
                    &restic_bin,
                    &env.backup_cache_dir,
                    None,
                    &args,
                    RESTIC_RUN_TIMEOUT,
                    &scrub,
                )
                .await;

                for cid in paused.iter().rev() {
                    let _ = env.docker.unpause_container(cid).await;
                }

                let out = run_res?;
                if out.code != Some(0) {
                    return Err(err(
                        PrivErrorCode::Internal,
                        restic_failure_msg("restic volume backup", &out, &scrub),
                    ));
                }
                parse_backup_summary(&out.stdout).map_err(map_backup_err)?
            }
            BackupRunTarget::Path { paths } => {
                let clean = validate_path_list(&paths).map_err(map_backup_err)?;
                let mut resolved = Vec::with_capacity(clean.len());
                for p in &clean {
                    let canon = validate_live_path(p, env)?;
                    resolved.push(canon.to_string_lossy().into_owned());
                }
                let what = clean.join(", ");
                let tags = job_tags(&req.job_id, BackupTargetKind::Path, &what);
                let excludes = backup_excludes(env);
                let args = backup_paths_args(&repo_dir, pw.path(), &resolved, &excludes, &tags);
                let scrub = [pw.secret()];
                log.push(format!("Backing up {} path(s).", resolved.len()));
                let out = run_restic(
                    &restic_bin,
                    &env.backup_cache_dir,
                    None,
                    &args,
                    RESTIC_RUN_TIMEOUT,
                    &scrub,
                )
                .await?;
                if out.code != Some(0) {
                    return Err(err(
                        PrivErrorCode::Internal,
                        restic_failure_msg("restic path backup", &out, &scrub),
                    ));
                }
                parse_backup_summary(&out.stdout).map_err(map_backup_err)?
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

                log.push(
                    "Staged online SQLite backup and /etc/opsbox (excluding master.key).".into(),
                );
                let tags = job_tags(
                    &req.job_id,
                    BackupTargetKind::Opsbox,
                    "opsbox (SQLite + /etc/opsbox)",
                );
                let mut excludes = backup_excludes(env);
                excludes.push("master.key".to_owned());
                excludes.push("*/master.key".to_owned());
                let args = backup_paths_args(&repo_dir, pw.path(), &rel_targets, &excludes, &tags);
                let scrub = [pw.secret()];
                let out = run_restic(
                    &restic_bin,
                    &env.backup_cache_dir,
                    Some(stage_dir.path()),
                    &args,
                    RESTIC_RUN_TIMEOUT,
                    &scrub,
                )
                .await?;
                drop(stage_dir);
                if out.code != Some(0) {
                    return Err(err(
                        PrivErrorCode::Internal,
                        restic_failure_msg("restic opsbox self-backup", &out, &scrub),
                    ));
                }
                parse_backup_summary(&out.stdout).map_err(map_backup_err)?
            }
        };

        log.push(format!(
            "Created snapshot {} ({} bytes processed, {} bytes added).",
            summary.short_id, summary.total_bytes_processed, summary.data_added
        ));

        let mut pruned_count = 0u32;
        if req.run_prune {
            let fg_args = forget_args(
                &repo_dir,
                pw.path(),
                Some(&req.job_id),
                req.retain_daily,
                req.retain_weekly,
            );
            let scrub = [pw.secret()];
            let fg_out = run_restic(
                &restic_bin,
                &env.backup_cache_dir,
                None,
                &fg_args,
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
            let chk_args = check_args(&repo_dir, pw.path(), Some(DEFAULT_SUBSET));
            let scrub = [pw.secret()];
            let chk_out = run_restic(
                &restic_bin,
                &env.backup_cache_dir,
                None,
                &chk_args,
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
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            pruned_count,
            check_state,
            check_error,
            log,
        })
    })?
}

/// List snapshots stored in the encrypted local restic repository.
pub fn list_snapshots_with(
    env: &Env,
    req: BackupSnapshotsRequest,
) -> Result<Vec<ResticSnapshotInfo>, PrivError> {
    let repo_dir = resolve_local_repo_dir(env, &req.destination_id)?;
    if !repo_dir.join("config").is_file() {
        return Ok(Vec::new());
    }
    let restic_bin = find_restic_bin(env)?;
    let secret = derive_pw(&env.secrets)?;
    let repo_mutex = repo_lock(&repo_dir);

    block_on_fut(async move {
        let _guard = repo_mutex.lock().await;
        if !repo_dir.join("config").is_file() {
            return Ok(Vec::new());
        }
        let pw = PasswordFileGuard::create(&env.backup_cache_dir, secret)?;
        let args = snapshots_args(&repo_dir, pw.path());
        let scrub = [pw.secret()];
        let out = run_restic(
            &restic_bin,
            &env.backup_cache_dir,
            None,
            &args,
            RESTIC_META_TIMEOUT,
            &scrub,
        )
        .await?;
        if out.code != Some(0) {
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
    let repo_dir = resolve_local_repo_dir(env, &req.destination_id)?;
    if !repo_dir.join("config").is_file() {
        return Ok(BackupForgetResult {
            removed_snapshot_ids: Vec::new(),
            kept_count: 0,
            removed_count: 0,
            log: vec!["Local backup repository has no snapshots yet.".into()],
        });
    }
    let restic_bin = find_restic_bin(env)?;
    let secret = derive_pw(&env.secrets)?;
    let repo_mutex = repo_lock(&repo_dir);

    block_on_fut(async move {
        let _guard = repo_mutex.lock().await;
        let pw = PasswordFileGuard::create(&env.backup_cache_dir, secret)?;
        let args = forget_args(
            &repo_dir,
            pw.path(),
            req.job_id.as_deref(),
            req.retain_daily,
            req.retain_weekly,
        );
        let scrub = [pw.secret()];
        let out = run_restic(
            &restic_bin,
            &env.backup_cache_dir,
            None,
            &args,
            RESTIC_RUN_TIMEOUT,
            &scrub,
        )
        .await?;
        if out.code != Some(0) {
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

/// Run `restic check` (optionally `--read-data-subset`) on the local repository.
pub fn check_with(env: &Env, req: BackupCheckRequest) -> Result<BackupCheckResult, PrivError> {
    let subset = match req.read_data_subset {
        Some(ref s) => Some(validate_read_data_subset(s).map_err(map_backup_err)?),
        None => Some(DEFAULT_SUBSET.to_owned()),
    };
    let repo_dir = resolve_local_repo_dir(env, &req.destination_id)?;
    if !repo_dir.join("config").is_file() {
        return Err(err(
            PrivErrorCode::NotFound,
            "No local backup repository exists yet — run a backup first.",
        ));
    }
    let restic_bin = find_restic_bin(env)?;
    let secret = derive_pw(&env.secrets)?;
    let repo_mutex = repo_lock(&repo_dir);

    block_on_fut(async move {
        let _guard = repo_mutex.lock().await;
        let pw = PasswordFileGuard::create(&env.backup_cache_dir, secret)?;
        let args = check_args(&repo_dir, pw.path(), subset.as_deref());
        let scrub = [pw.secret()];
        let out = run_restic(
            &restic_bin,
            &env.backup_cache_dir,
            None,
            &args,
            RESTIC_RUN_TIMEOUT,
            &scrub,
        )
        .await?;
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
snapshot_file=""
args_log="${RESTIC_TEST_LOG:-}"
prev=""
for a in "$@"; do
  case "$prev" in
    -r) repo="$a"; prev=""; continue ;;
    --password-file) pw="$a"; prev=""; continue ;;
    --stdin-filename) stdin_name="$a"; prev=""; continue ;;
  esac
  case "$a" in
    -r|--password-file|--stdin-filename|--tag|--exclude|--group-by|--keep-daily|--keep-weekly)
      prev="$a"
      ;;
    --stdin) stdin_mode=1 ;;
    version) sub="version" ;;
    init) sub="init" ;;
    backup) sub="backup" ;;
    snapshots) sub="snapshots" ;;
    forget) sub="forget" ;;
    check) sub="check" ;;
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

case "$sub" in
  init)
    mkdir -p "$repo"
    echo "{}" > "$repo/config"
    echo "{\"message_type\":\"initialized\",\"id\":\"testrepo\",\"repository\":\"$repo\"}"
    ;;
  backup)
    mkdir -p "$repo"
    if [ ! -f "$repo/config" ]; then
      echo "not initialized" >&2
      exit 1
    fi
    if [ "$stdin_mode" -eq 1 ]; then
      bytes=$(wc -c | tr -d ' ')
      printf "%s:%s\n" "$stdin_name" "$bytes" > "$repo/last_stdin"
      echo "{\"message_type\":\"summary\",\"snapshot_id\":\"43deae25000000000000000000000001\",\"total_bytes_processed\":$bytes,\"data_added\":$bytes,\"total_files_processed\":1}"
    else
      if [ -f "opsbox.db" ] && [ -f "etc-opsbox/config.toml" ]; then
        cp "opsbox.db" "$repo/captured_opsbox.db"
        if [ -f "etc-opsbox/master.key" ]; then
          echo "MASTER_KEY_LEAKED" >&2
          exit 9
        fi
        cp "etc-opsbox/config.toml" "$repo/captured_config.toml"
      fi
      echo "{\"message_type\":\"summary\",\"snapshot_id\":\"aaaa1111000000000000000000000002\",\"total_bytes_processed\":4096,\"data_added\":2048,\"total_files_processed\":3}"
    fi
    ;;
  snapshots)
    echo "[{\"id\":\"43deae25000000000000000000000001\",\"short_id\":\"43deae25\",\"time\":\"2026-11-18T02:00:05Z\",\"hostname\":\"vm\",\"paths\":[\"/pg.fc\"],\"tags\":[\"opsbox-job:job1\",\"opsbox-kind:database\",\"opsbox-what:pg\"],\"summary\":{\"total_bytes_processed\":1024}}]"
    ;;
  forget)
    echo "[{\"keep\":[{\"id\":\"aaaa1111\",\"short_id\":\"aaaa1111\"}],\"remove\":[{\"id\":\"bbbb2222\",\"short_id\":\"bbbb2222\"}]}]"
    ;;
  check)
    if [ "${RESTIC_FAIL_CHECK:-0}" = "1" ]; then
      echo "{\"message_type\":\"summary\",\"num_errors\":1,\"broken_packs\":[\"pack123\"]}"
      exit 1
    fi
    echo "{\"message_type\":\"summary\",\"num_errors\":0,\"broken_packs\":[]}"
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
            },
        )
        .unwrap();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].job_id.as_deref(), Some("job1"));

        let chk = check_with(
            &env,
            BackupCheckRequest {
                destination_id: "local".into(),
                read_data_subset: Some("10%".into()),
            },
        )
        .unwrap();
        assert!(chk.ok);
    }

    #[test]
    fn refuses_master_key_dotdot_and_remote_destinations() {
        let tmp = TempDir::new().unwrap();
        let restic = write_mock_restic(tmp.path());
        let env = make_env(tmp.path(), Some(restic));

        let err_mk = run_backup_with(
            &env,
            BackupRunSpec {
                destination_id: "local".into(),
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

        let err_remote = run_backup_with(
            &env,
            BackupRunSpec {
                destination_id: "s3-prod".into(),
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
        assert_eq!(err_remote.code, PrivErrorCode::Unsupported);
    }
}
