//! Uninstall opsbox while keeping apps and (by default) data.
//!
//! The CLI (`opsbox uninstall`) runs the steps as root. The panel calls
//! [`PrivOp::Uninstall`], which schedules the same CLI via a detached
//! `systemd-run` unit so the panel can stop itself cleanly.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use opsbox_proto::{
    PrivError, PrivErrorCode, PrivOp, PrivReply, UninstallAdopted, UninstallPlan,
};
use serde::Deserialize;

use super::Env;

const PREFIX_BIN: &str = "/usr/local/bin/opsbox";
const STATE_DIR: &str = "/var/lib/opsbox";
const CONF_DIR: &str = "/etc/opsbox";
const UNIT_DIR: &str = "/etc/systemd/system";
const SLOT_DIR: &str = "/var/lib/opsbox/bin";
const BACKUP_DIR: &str = "/var/backups/opsbox";
const BACKUP_CACHE_DIR: &str = "/var/cache/opsbox/restic";
const SVC_USER: &str = "opsbox";
const DB_PATH: &str = "/var/lib/opsbox/opsbox.db";

/// Core services and the cleanup timer pair.
const CORE_UNITS: &[&str] = &[
    "opsbox-watchdog.service",
    "opsbox.service",
    "opsbox-proxy.service",
    "opsbox-priv.service",
    "opsbox-cleanup.timer",
    "opsbox-cleanup.service",
];

fn err(code: PrivErrorCode, message: impl Into<String>) -> PrivError {
    PrivError {
        code,
        message: message.into(),
    }
}

/// Install layout. Production uses absolute Linux paths; tests pass a temp root.
#[derive(Debug, Clone)]
pub struct Layout {
    pub prefix_bin: PathBuf,
    pub state_dir: PathBuf,
    pub conf_dir: PathBuf,
    pub unit_dir: PathBuf,
    pub slot_dir: PathBuf,
    pub backup_dir: PathBuf,
    pub backup_cache_dir: PathBuf,
    pub db_path: PathBuf,
}

impl Layout {
    pub fn production() -> Self {
        Self {
            prefix_bin: PREFIX_BIN.into(),
            state_dir: STATE_DIR.into(),
            conf_dir: CONF_DIR.into(),
            unit_dir: UNIT_DIR.into(),
            slot_dir: SLOT_DIR.into(),
            backup_dir: BACKUP_DIR.into(),
            backup_cache_dir: BACKUP_CACHE_DIR.into(),
            db_path: DB_PATH.into(),
        }
    }

    /// Map production paths under `root` for dry-run / unit tests.
    pub fn under(root: &Path) -> Self {
        let strip = |p: &str| -> PathBuf {
            let rel = p.trim_start_matches('/');
            root.join(rel)
        };
        Self {
            prefix_bin: strip(PREFIX_BIN),
            state_dir: strip(STATE_DIR),
            conf_dir: strip(CONF_DIR),
            unit_dir: strip(UNIT_DIR),
            slot_dir: strip(SLOT_DIR),
            backup_dir: strip(BACKUP_DIR),
            backup_cache_dir: strip(BACKUP_CACHE_DIR),
            db_path: strip(DB_PATH),
        }
    }
}

/// Inputs that shape the plan beyond the fixed path list.
#[derive(Debug, Clone, Default)]
pub struct PlanContext {
    pub purge: bool,
    pub domains: Vec<String>,
    pub adopted: Option<UninstallAdopted>,
    pub containers: Vec<String>,
    /// Extra timer unit names (e.g. `opsbox-job-abcd1234.timer`) discovered on disk.
    pub extra_timers: Vec<String>,
}

/// Pure plan builder: what is removed, what stays, warnings.
pub fn build_plan(layout: &Layout, ctx: &PlanContext) -> UninstallPlan {
    let mut removed = vec![
        "opsbox, opsbox-priv, opsbox-proxy and opsbox-watchdog services".into(),
        "opsbox-cleanup.timer (weekly cleanup)".into(),
        format!("{} (symlink and release slots)", layout.prefix_bin.display()),
        format!("system user `{SVC_USER}`"),
    ];
    for t in &ctx.extra_timers {
        removed.push(format!("timer {t}"));
    }
    if ctx.purge {
        removed.push(format!("{} (database, audit log, apps metadata)", layout.state_dir.display()));
        removed.push(format!("{} (config and master key)", layout.conf_dir.display()));
        removed.push(format!("{} (local backups)", layout.backup_dir.display()));
        removed.push(format!("{} (restic cache)", layout.backup_cache_dir.display()));
    }

    let mut kept = Vec::new();
    if !ctx.containers.is_empty() {
        kept.push(format!(
            "{} app container(s), restart policy set to unless-stopped",
            ctx.containers.len()
        ));
    } else {
        kept.push("App containers (restart policy set to unless-stopped)".into());
    }
    if !ctx.purge {
        kept.push(format!(
            "{} (database, audit log, proxy state)",
            layout.state_dir.display()
        ));
        kept.push(format!("{} (config.toml, master key)", layout.conf_dir.display()));
        kept.push(format!("{} (local restic backups)", layout.backup_dir.display()));
    }
    if let Some(a) = &ctx.adopted
        && a.will_restore
    {
        kept.push(format!(
            "Adopted web server {} restored to ports 80/443",
            a.holder
        ));
    }

    let mut warnings = Vec::new();
    if !ctx.domains.is_empty() {
        warnings.push(
            "These domains stop answering on ports 80/443 until another proxy takes over."
                .into(),
        );
    }
    if ctx.purge {
        warnings.push(
            "Purge permanently deletes /var/lib/opsbox and /etc/opsbox (and local backups)."
                .into(),
        );
    }

    UninstallPlan {
        purge: ctx.purge,
        removed,
        kept,
        domains: ctx.domains.clone(),
        adopted: ctx.adopted.clone(),
        containers: ctx.containers.clone(),
        warnings,
    }
}

/// Discover `opsbox-*.timer` unit files (cleanup + job / backup-style timers).
pub fn discover_timers(unit_dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(unit_dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(s) = name.to_str() else { continue };
        if s.starts_with("opsbox-") && s.ends_with(".timer") && s != "opsbox-cleanup.timer" {
            out.push(s.to_owned());
        }
    }
    out.sort();
    out
}

/// Paths that file-removal steps delete. Used by dry-run tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathRemovals {
    /// Always removed (binary symlink, unit files, slots directory contents handled separately).
    pub always: Vec<PathBuf>,
    /// Removed only with `--purge`.
    pub purge_only: Vec<PathBuf>,
}

pub fn path_removals(layout: &Layout, extra_timers: &[String]) -> PathRemovals {
    let mut always = vec![layout.prefix_bin.clone()];
    for u in CORE_UNITS {
        always.push(layout.unit_dir.join(u));
    }
    for t in extra_timers {
        always.push(layout.unit_dir.join(t));
        if let Some(svc) = t.strip_suffix(".timer") {
            always.push(layout.unit_dir.join(format!("{svc}.service")));
        }
    }
    // Release slots directory (binaries under /var/lib/opsbox/bin).
    always.push(layout.slot_dir.clone());

    let purge_only = vec![
        layout.state_dir.clone(),
        layout.conf_dir.clone(),
        layout.backup_dir.clone(),
        layout.backup_cache_dir.clone(),
    ];
    PathRemovals { always, purge_only }
}

/// Apply path removals against `layout`. When `dry_run` is true, only report what would go.
/// Returns the list of paths that were (or would be) removed.
pub fn apply_path_removals(
    layout: &Layout,
    purge: bool,
    extra_timers: &[String],
    dry_run: bool,
) -> io::Result<Vec<PathBuf>> {
    let plan = path_removals(layout, extra_timers);
    let mut done = Vec::new();
    for p in &plan.always {
        if p.exists() || p.is_symlink() {
            done.push(p.clone());
            if !dry_run {
                remove_path(p)?;
            }
        }
    }
    if purge {
        for p in &plan.purge_only {
            if p.exists() {
                done.push(p.clone());
                if !dry_run {
                    remove_path(p)?;
                }
            }
        }
    }
    Ok(done)
}

fn remove_path(p: &Path) -> io::Result<()> {
    let meta = match fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if meta.is_dir() && !meta.file_type().is_symlink() {
        fs::remove_dir_all(p)
    } else {
        fs::remove_file(p)
    }
}

fn dump_audit_text(db: &Path) -> Result<String, String> {
    // Read-only: uninstall must not migrate or write the live database.
    let store = opsbox_store::ReadOnlyStore::open(db).map_err(|e| e.to_string())?;
    let entries = store.recent_audit(10_000).map_err(|e| e.to_string())?;
    let mut body = String::new();
    for e in entries.into_iter().rev() {
        body.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            e.ts,
            e.actor,
            e.action,
            e.target.as_deref().unwrap_or("-"),
            e.result,
            e.ip.as_deref().unwrap_or("-"),
            e.detail
        ));
    }
    Ok(body)
}

fn today() -> String {
    // Prefer local date via `date`; fall back to UTC day from SystemTime.
    if let Ok(out) = Command::new("date").args(["+%Y-%m-%d"]).output()
        && out.status.success()
    {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if !s.is_empty() {
            return s;
        }
    }
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    // 1970-01-01 + days — good enough for a log file name in tests.
    format!("day-{days}")
}

/// Export the audit chain to `state_dir/uninstall-<date>.log` (and copy aside before purge).
pub fn export_audit_log(layout: &Layout, purge: bool) -> io::Result<PathBuf> {
    let date = today();
    let dest = layout
        .state_dir
        .join(format!("uninstall-{date}.log"));
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut body = String::new();
    body.push_str(&format!("opsbox uninstall audit export ({date})\n"));
    body.push_str(&format!("purge={purge}\n\n"));

    if layout.db_path.exists() {
        match dump_audit_text(&layout.db_path) {
            Ok(text) => body.push_str(&text),
            Err(e) => body.push_str(&format!("(could not read audit log: {e})\n")),
        }
    } else {
        body.push_str("(no database present)\n");
    }

    fs::write(&dest, body.as_bytes())?;

    if purge {
        // Keep a copy outside the purged tree.
        let aside = PathBuf::from(format!("/var/log/opsbox-uninstall-{date}.log"));
        if let Some(parent) = aside.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if fs::copy(&dest, &aside).is_ok() {
            return Ok(aside);
        }
    }
    Ok(dest)
}

fn systemctl() -> Option<PathBuf> {
    ["/usr/bin/systemctl", "/bin/systemctl"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

fn docker_bin() -> Option<PathBuf> {
    ["/usr/bin/docker", "/usr/local/bin/docker"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

fn run_cmd(bin: &Path, args: &[&str]) -> io::Result<std::process::Output> {
    Command::new(bin).args(args).output()
}

/// Stop and disable every opsbox unit and discovered timer.
pub fn stop_and_disable_units(extra_timers: &[String]) {
    let Some(ctl) = systemctl() else {
        return;
    };
    let mut names: Vec<&str> = CORE_UNITS.to_vec();
    let owned: Vec<String> = extra_timers
        .iter()
        .flat_map(|t| {
            let mut v = vec![t.clone()];
            if let Some(svc) = t.strip_suffix(".timer") {
                v.push(format!("{svc}.service"));
            }
            v
        })
        .collect();
    for s in &owned {
        names.push(s.as_str());
    }
    // Also clear transient helpers from updates.
    let _ = run_cmd(
        &ctl,
        &[
            "stop",
            "opsbox-restart.service",
            "opsbox-rollback-check.timer",
            "opsbox-rollback-check.service",
            "opsbox-uninstall.service",
        ],
    );
    let mut stop_args = vec!["stop"];
    stop_args.extend(names.iter().copied());
    let _ = run_cmd(&ctl, &stop_args);
    let mut dis_args = vec!["disable", "--now"];
    dis_args.extend(names.iter().copied());
    let _ = run_cmd(&ctl, &dis_args);
    let _ = run_cmd(&ctl, &["daemon-reload"]);
    let _ = run_cmd(&ctl, &["reset-failed"]);
}

/// Switch managed containers to `unless-stopped` so they survive without opsbox.
pub fn keep_containers_running() -> Vec<String> {
    let Some(docker) = docker_bin() else {
        return Vec::new();
    };
    let out = match run_cmd(
        &docker,
        &[
            "ps",
            "-aq",
            "--filter",
            "label=opsbox.managed=true",
            "--format",
            "{{.Names}}",
        ],
    ) {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => return Vec::new(),
    };
    let mut names = Vec::new();
    for name in out.lines().map(str::trim).filter(|s| !s.is_empty()) {
        let _ = run_cmd(&docker, &["update", "--restart=unless-stopped", name]);
        names.push(name.to_owned());
    }
    names
}

fn list_managed_containers() -> Vec<String> {
    let Some(docker) = docker_bin() else {
        return Vec::new();
    };
    let out = match run_cmd(
        &docker,
        &[
            "ps",
            "-aq",
            "--filter",
            "label=opsbox.managed=true",
            "--format",
            "{{.Names}}",
        ],
    ) {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => return Vec::new(),
    };
    out.lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

#[derive(Deserialize)]
struct ReceiptPublic {
    public: opsbox_proto::WebMove,
}

/// Active adoption receipts under `/etc/opsbox/web-moves`.
fn active_adopted(conf_dir: &Path) -> Option<UninstallAdopted> {
    let dir = conf_dir.join("web-moves");
    let entries = fs::read_dir(&dir).ok()?;
    for e in entries.flatten() {
        let Ok(raw) = fs::read(e.path()) else { continue };
        let Ok(r) = serde_json::from_slice::<ReceiptPublic>(&raw) else {
            continue;
        };
        if r.public.active {
            return Some(UninstallAdopted {
                holder: r.public.holder,
                domains: Vec::new(), // filled by the API from routes when known
                will_restore: true,
            });
        }
    }
    None
}

fn remove_opsbox_user() {
    let _ = Command::new("userdel").args([SVC_USER]).output();
    // Leave the group if other members exist; ignore failures.
    let _ = Command::new("groupdel").args([SVC_USER]).output();
}

/// Full uninstall used by the CLI. `live` runs systemctl/docker/userdel; tests pass false
/// and only exercise path removals via [`apply_path_removals`].
pub fn execute(layout: &Layout, purge: bool, live: bool) -> Result<UninstallPlan, String> {
    let extra = discover_timers(&layout.unit_dir);
    let containers = if live {
        keep_containers_running()
    } else {
        Vec::new()
    };
    let adopted = active_adopted(&layout.conf_dir);
    let ctx = PlanContext {
        purge,
        domains: Vec::new(),
        adopted: adopted.clone(),
        containers: containers.clone(),
        extra_timers: extra.clone(),
    };
    let plan = build_plan(layout, &ctx);

    if live {
        // Restore adopted web before stopping the proxy, so old sites keep working.
        if adopted.as_ref().is_some_and(|a| a.will_restore)
            && let Err(e) = restore_adopted_via_cli(&layout.conf_dir)
        {
            eprintln!("opsbox uninstall: warning: could not restore adopted web server: {e}");
        }

        match export_audit_log(layout, purge) {
            Ok(path) => println!("Audit log exported to {}", path.display()),
            Err(e) => eprintln!("opsbox uninstall: warning: could not export audit log: {e}"),
        }

        stop_and_disable_units(&extra);
        if let Err(e) = apply_path_removals(layout, purge, &extra, false) {
            return Err(format!("removing files failed: {e}"));
        }
        remove_opsbox_user();
        if let Some(ctl) = systemctl() {
            let _ = run_cmd(&ctl, &["daemon-reload"]);
        }
    }

    Ok(plan)
}

/// Best-effort: for each active receipt, call the same undo path the panel uses by
/// shelling `systemctl` / `docker` through a short helper script is too fragile; instead
/// invoke `opsbox` is unavailable mid-uninstall. We re-apply backed-up config files and
/// reload the unit / container from the receipt JSON.
fn restore_adopted_via_cli(conf_dir: &Path) -> Result<(), String> {
    let dir = conf_dir.join("web-moves");
    let entries = fs::read_dir(&dir).map_err(|e| e.to_string())?;
    #[derive(Deserialize)]
    struct FullReceipt {
        public: opsbox_proto::WebMove,
        files: Vec<SavedFile>,
        plan: opsbox_proto::WebPlan,
        #[serde(default)]
        old_restart: String,
        mutated: bool,
    }
    #[derive(Deserialize)]
    struct SavedFile {
        path: String,
        before: Option<String>,
        mode: u32,
    }

    for e in entries.flatten() {
        let raw = fs::read(e.path()).map_err(|e| e.to_string())?;
        let r: FullReceipt = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
        if !r.public.active || !r.mutated {
            continue;
        }
        for f in &r.files {
            if let Some(before) = &f.before {
                write_file_mode(Path::new(&f.path), before.as_bytes(), f.mode)
                    .map_err(|e| e.to_string())?;
            }
        }
        match &r.plan.target {
            opsbox_proto::PortTarget::Unit(unit) => {
                if let Some(ctl) = systemctl() {
                    let _ = run_cmd(&ctl, &["reload-or-restart", unit]);
                }
            }
            opsbox_proto::PortTarget::Container(id) => {
                if let Some(docker) = docker_bin() {
                    let policy = if r.old_restart.is_empty() {
                        "unless-stopped"
                    } else {
                        r.old_restart.as_str()
                    };
                    let _ = run_cmd(&docker, &["update", &format!("--restart={policy}"), id]);
                    let _ = run_cmd(&docker, &["start", id]);
                }
            }
        }
        // Mark inactive so a later reinstall does not undo again.
        if let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(&raw) {
            if let Some(p) = v.get_mut("public") {
                p["active"] = serde_json::json!(false);
            }
            let _ = fs::write(e.path(), serde_json::to_vec_pretty(&v).unwrap_or(raw));
        }
        println!(
            "Restored adopted web server {} (ports 80/443).",
            r.public.holder
        );
    }
    Ok(())
}

fn write_file_mode(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("opsbox-uninstall-tmp");
    fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Schedule `opsbox uninstall --yes [--purge]` in a transient unit and return.
pub fn schedule(purge: bool) -> Result<(), PrivError> {
    let bin = if Path::new(PREFIX_BIN).exists() {
        PREFIX_BIN.to_owned()
    } else {
        // Dev / unusual prefix: use argv0-equivalent from PATH.
        which_opsbox().ok_or_else(|| {
            err(
                PrivErrorCode::NotFound,
                "Cannot find the opsbox binary to schedule uninstall.",
            )
        })?
    };
    let _ = Command::new("systemctl")
        .args(["reset-failed", "opsbox-uninstall.service"])
        .output();
    let mut script = format!("sleep 1; {bin} uninstall --yes");
    if purge {
        script.push_str(" --purge");
    }
    let out = Command::new("systemd-run")
        .args([
            "--unit=opsbox-uninstall",
            "--no-block",
            "--description=Uninstall opsbox (panel requested)",
            "/bin/sh",
            "-c",
            &script,
        ])
        .output()
        .map_err(|e| err(PrivErrorCode::Internal, format!("systemd-run failed: {e}")))?;
    if !out.status.success() {
        return Err(err(
            PrivErrorCode::Internal,
            format!(
                "Cannot schedule uninstall: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        ));
    }
    Ok(())
}

fn which_opsbox() -> Option<String> {
    let out = Command::new("sh")
        .args(["-c", "command -v opsbox"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if s.is_empty() { None } else { Some(s) }
}

/// Live plan for the panel: discover timers, containers, adopted receipts.
pub fn live_plan(purge: bool, domains: Vec<String>) -> UninstallPlan {
    let layout = Layout::production();
    let extra = discover_timers(&layout.unit_dir);
    let mut adopted = active_adopted(&layout.conf_dir);
    if let Some(a) = adopted.as_mut() {
        // Prefer domain list from the panel when the receipt has none.
        if a.domains.is_empty() {
            a.domains = domains.clone();
        }
    }
    let ctx = PlanContext {
        purge,
        domains,
        adopted,
        containers: list_managed_containers(),
        extra_timers: extra,
    };
    build_plan(&layout, &ctx)
}

pub(super) fn dispatch(op: &PrivOp, _env: &Env) -> Result<PrivReply, PrivError> {
    match op {
        PrivOp::UninstallPlan { purge, domains } => Ok(PrivReply::UninstallPlan(Box::new(
            live_plan(*purge, domains.clone()),
        ))),
        PrivOp::Uninstall { purge } => schedule(*purge).map(|()| PrivReply::Done),
        _ => Err(err(
            PrivErrorCode::BadRequest,
            "Not an uninstall operation.",
        )),
    }
}

/// CLI entry: confirmations, then [`execute`].
pub fn run_cli(purge: bool, yes: bool) -> Result<(), i32> {
    use nix::unistd::geteuid;
    if !geteuid().is_root() {
        eprintln!("opsbox uninstall must run as root (try: sudo opsbox uninstall).");
        return Err(1);
    }

    let layout = Layout::production();
    let extra = discover_timers(&layout.unit_dir);
    let domains: Vec<String> = Vec::new(); // CLI has no store; warnings still cover proxy.
    let ctx = PlanContext {
        purge,
        domains,
        adopted: active_adopted(&layout.conf_dir),
        containers: list_managed_containers(),
        extra_timers: extra,
    };
    let plan = build_plan(&layout, &ctx);

    println!("This will remove opsbox from this server.");
    println!();
    println!("Removed:");
    for r in &plan.removed {
        println!("  - {r}");
    }
    println!("Kept:");
    for k in &plan.kept {
        println!("  - {k}");
    }
    if !plan.warnings.is_empty() {
        println!("Warnings:");
        for w in &plan.warnings {
            println!("  - {w}");
        }
    }
    println!();

    if !yes {
        eprint!("Type uninstall to continue: ");
        let _ = io::stderr().flush();
        let mut line = String::new();
        if io::stdin().read_line(&mut line).is_err() || line.trim() != "uninstall" {
            eprintln!("Cancelled.");
            return Err(1);
        }
        if purge {
            eprint!("Type purge to permanently delete /var/lib/opsbox and /etc/opsbox: ");
            let _ = io::stderr().flush();
            line.clear();
            if io::stdin().read_line(&mut line).is_err() || line.trim() != "purge" {
                eprintln!("Cancelled.");
                return Err(1);
            }
        }
    }

    match execute(&layout, purge, true) {
        Ok(_) => {
            println!();
            println!("opsbox has been uninstalled.");
            if !purge {
                println!(
                    "Data kept at {} and {}.",
                    layout.state_dir.display(),
                    layout.conf_dir.display()
                );
                println!("Re-running install.sh reuses that data when present.");
            }
            Ok(())
        }
        Err(e) => {
            eprintln!("opsbox uninstall failed: {e}");
            Err(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn seed_layout(root: &Path) -> Layout {
        let layout = Layout::under(root);
        fs::create_dir_all(&layout.unit_dir).unwrap();
        fs::create_dir_all(&layout.slot_dir).unwrap();
        fs::create_dir_all(&layout.state_dir).unwrap();
        fs::create_dir_all(&layout.conf_dir).unwrap();
        fs::create_dir_all(&layout.backup_dir).unwrap();
        fs::create_dir_all(layout.prefix_bin.parent().unwrap()).unwrap();
        fs::write(&layout.prefix_bin, b"fake").unwrap();
        fs::write(layout.slot_dir.join("opsbox-0.1.0"), b"bin").unwrap();
        for u in CORE_UNITS {
            fs::write(layout.unit_dir.join(u), b"unit").unwrap();
        }
        fs::write(layout.unit_dir.join("opsbox-job-deadbeef.timer"), b"t").unwrap();
        fs::write(layout.unit_dir.join("opsbox-job-deadbeef.service"), b"s").unwrap();
        fs::write(layout.conf_dir.join("config.toml"), b"# cfg").unwrap();
        fs::write(layout.state_dir.join("opsbox.db"), b"db").unwrap();
        layout
    }

    #[test]
    fn plan_no_proxy_domains() {
        let layout = Layout::production();
        let plan = build_plan(
            &layout,
            &PlanContext {
                purge: false,
                ..Default::default()
            },
        );
        assert!(plan.domains.is_empty());
        assert!(plan.warnings.iter().all(|w| !w.contains("domains")));
        assert!(plan.kept.iter().any(|k| k.contains("/var/lib/opsbox")));
        assert!(plan.removed.iter().any(|r| r.contains("opsbox-priv")));
        assert!(!plan.purge);
    }

    #[test]
    fn plan_with_domains_warns() {
        let layout = Layout::production();
        let plan = build_plan(
            &layout,
            &PlanContext {
                purge: false,
                domains: vec!["app.example.com".into(), "panel.example.com".into()],
                ..Default::default()
            },
        );
        assert_eq!(plan.domains.len(), 2);
        assert!(plan.warnings.iter().any(|w| w.contains("80/443")));
    }

    #[test]
    fn plan_with_adopted_web_server() {
        let layout = Layout::production();
        let plan = build_plan(
            &layout,
            &PlanContext {
                purge: false,
                domains: vec!["old.example.com".into()],
                adopted: Some(UninstallAdopted {
                    holder: "nginx.service".into(),
                    domains: vec!["old.example.com".into()],
                    will_restore: true,
                }),
                ..Default::default()
            },
        );
        assert!(
            plan.kept
                .iter()
                .any(|k| k.contains("nginx.service") && k.contains("restored"))
        );
        assert_eq!(plan.adopted.as_ref().unwrap().holder, "nginx.service");
    }

    #[test]
    fn dry_run_keep_data_removes_units_keeps_state() {
        let tmp = tempdir().unwrap();
        let layout = seed_layout(tmp.path());
        let extra = discover_timers(&layout.unit_dir);
        assert_eq!(extra, vec!["opsbox-job-deadbeef.timer".to_owned()]);

        let removed = apply_path_removals(&layout, false, &extra, false).unwrap();
        assert!(removed.iter().any(|p| p == &layout.prefix_bin));
        assert!(removed.iter().any(|p| p.ends_with("opsbox.service")));
        assert!(removed.iter().any(|p| p.ends_with("opsbox-job-deadbeef.timer")));
        assert!(layout.state_dir.exists(), "keep-data leaves state dir");
        assert!(layout.conf_dir.exists(), "keep-data leaves conf dir");
        assert!(layout.backup_dir.exists());
        assert!(!layout.prefix_bin.exists());
        assert!(!layout.unit_dir.join("opsbox.service").exists());
        assert!(!layout.slot_dir.exists());
    }

    #[test]
    fn dry_run_purge_removes_data_dirs() {
        let tmp = tempdir().unwrap();
        let layout = seed_layout(tmp.path());
        let extra = discover_timers(&layout.unit_dir);
        apply_path_removals(&layout, true, &extra, false).unwrap();
        assert!(!layout.state_dir.exists());
        assert!(!layout.conf_dir.exists());
        assert!(!layout.backup_dir.exists());
    }

    #[test]
    fn dry_run_flag_lists_without_deleting() {
        let tmp = tempdir().unwrap();
        let layout = seed_layout(tmp.path());
        let extra = discover_timers(&layout.unit_dir);
        let listed = apply_path_removals(&layout, true, &extra, true).unwrap();
        assert!(!listed.is_empty());
        assert!(layout.prefix_bin.exists());
        assert!(layout.state_dir.exists());
    }
}
