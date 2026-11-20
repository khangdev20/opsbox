//! `opsbox-watchdog`: independent supervisor, health probe loop, auto-rollback
//! policy, and standalone HTTPS Safe Mode server on `:8444` (spec 7.16).
//!
//! Intentionally shares ZERO code with `serve` (`opsbox-api`), `opsbox-auth`,
//! `opsbox-update`, or `opsbox-proxy` beyond `opsbox-proto` types and read-only
//! `opsbox-store` queries so a regression in `serve` cannot take down recovery.

use std::fs;
use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use opsbox_proto::{WatchdogCheck, WatchdogStatus};
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tracing::{info, warn};

pub mod actions;
pub mod auth;
pub mod cleanup;
mod exec;
pub mod policy;
pub mod probes;
pub mod server;
pub mod slots;

use auth::SafeAuth;
use policy::{PolicyAction, TickInput, WatchdogPolicy};
use probes::{DbHealth, DiskHealth, ServiceHealth, human_bytes};
use server::SafeModeState;
use slots::SlotManager;

pub const DEFAULT_PROBE_INTERVAL: Duration = Duration::from_secs(10);
pub const STATUS_FILE: &str = "status.json";

#[derive(Debug, Error)]
pub enum WatchdogError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("{0}")]
    Config(String),
}

#[derive(Debug, Clone)]
pub struct WatchdogConfig {
    pub listen: SocketAddr,
    pub allow_ips: Vec<IpAddr>,
    pub serve_listen: SocketAddr,
    pub priv_socket: PathBuf,
    pub proxy_socket: PathBuf,
    pub db_path: PathBuf,
    pub master_key_path: PathBuf,
    pub runtime_dir: PathBuf,
    pub slots_dir: PathBuf,
    pub binary_symlink: PathBuf,
    pub update_state_file: PathBuf,
    pub xfer_dir: PathBuf,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub poll_interval: Duration,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([0, 0, 0, 0], 8444)),
            allow_ips: Vec::new(),
            serve_listen: SocketAddr::from(([127, 0, 0, 1], 8443)),
            priv_socket: PathBuf::from("/run/opsbox/priv.sock"),
            proxy_socket: PathBuf::from("/run/opsbox-proxy/proxy.sock"),
            db_path: PathBuf::from("/var/lib/opsbox/opsbox.db"),
            master_key_path: PathBuf::from("/etc/opsbox/master.key"),
            runtime_dir: PathBuf::from("/run/opsbox-watchdog"),
            slots_dir: PathBuf::from("/var/lib/opsbox/bin"),
            binary_symlink: PathBuf::from("/usr/local/bin/opsbox"),
            update_state_file: PathBuf::from("/var/lib/opsbox/bin/update-state.json"),
            xfer_dir: PathBuf::from("/var/lib/opsbox/xfer"),
            cert_path: PathBuf::from("/etc/opsbox/watchdog/selfsigned.crt"),
            key_path: PathBuf::from("/etc/opsbox/watchdog/selfsigned.key"),
            poll_interval: DEFAULT_PROBE_INTERVAL,
        }
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

fn ensure_runtime_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn write_status_file(runtime_dir: &Path, status: &WatchdogStatus) {
    let path = runtime_dir.join(STATUS_FILE);
    let tmp = runtime_dir.join("status.json.tmp");
    let Ok(bytes) = serde_json::to_vec(status) else {
        return;
    };
    if fs::write(&tmp, bytes).is_ok() {
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
        let _ = fs::rename(&tmp, &path);
    }
}

fn service_check(name: &str, health: &ServiceHealth) -> WatchdogCheck {
    match health {
        ServiceHealth::Ok { version, millis } => WatchdogCheck {
            name: name.to_owned(),
            status: "ok".into(),
            state: "healthy".into(),
            note: if version.is_empty() {
                format!("{millis} ms")
            } else {
                format!("v{version} · {millis} ms")
            },
        },
        ServiceHealth::PortConflict { .. } => WatchdogCheck {
            name: name.to_owned(),
            status: "warn".into(),
            state: "port_conflict".into(),
            note: "Proxy running on other ports. Open Settings > Network.".into(),
        },
        ServiceHealth::Down(err) => WatchdogCheck {
            name: name.to_owned(),
            status: "fail".into(),
            state: "down".into(),
            note: err.clone(),
        },
        ServiceHealth::Off => WatchdogCheck {
            name: name.to_owned(),
            status: "off".into(),
            state: "off".into(),
            note: "Not installed".into(),
        },
    }
}

fn disk_check(label: &str, disk: &DiskHealth) -> WatchdogCheck {
    if let Some(ref err) = disk.error {
        return WatchdogCheck {
            name: label.to_owned(),
            status: "warn".into(),
            state: "unknown".into(),
            note: err.clone(),
        };
    }
    let status = if disk.critical {
        "fail"
    } else if disk.warn {
        "warn"
    } else {
        "ok"
    };
    WatchdogCheck {
        name: label.to_owned(),
        status: status.into(),
        state: format!("{} free", human_bytes(disk.avail_bytes)),
        note: format!("{:.0}% used", disk.used_pct),
    }
}

fn db_check(health: &DbHealth) -> WatchdogCheck {
    match health {
        DbHealth::Ok => WatchdogCheck {
            name: "sqlite integrity".into(),
            status: "ok".into(),
            state: "ok".into(),
            note: "Verified on start".into(),
        },
        DbHealth::Missing => WatchdogCheck {
            name: "sqlite integrity".into(),
            status: "warn".into(),
            state: "pending".into(),
            note: "Database not initialized yet".into(),
        },
        DbHealth::Failed(err) => WatchdogCheck {
            name: "sqlite integrity".into(),
            status: "fail".into(),
            state: "error".into(),
            note: err.clone(),
        },
    }
}

struct SnapshotParams<'a> {
    now: i64,
    listen: SocketAddr,
    serve: &'a ServiceHealth,
    priv_h: &'a ServiceHealth,
    proxy: &'a ServiceHealth,
    disk_root: &'a DiskHealth,
    disk_data: Option<(&'a Path, &'a DiskHealth)>,
    db: &'a DbHealth,
    policy: &'a WatchdogPolicy,
    slots: &'a SlotManager,
    ssh_open_until: Option<i64>,
}

fn build_status(p: SnapshotParams<'_>) -> WatchdogStatus {
    let mut checks = vec![
        service_check("opsbox.service", p.serve),
        service_check("opsbox-priv.service", p.priv_h),
        service_check("opsbox-proxy.service", p.proxy),
        disk_check("disk /", p.disk_root),
    ];
    if let Some((path, d)) = p.disk_data {
        checks.push(disk_check(&format!("disk {}", path.display()), d));
    }
    checks.push(db_check(p.db));

    let failed = p.serve.is_failing() || p.priv_h.is_failing() || p.proxy.is_failing();
    let degraded = failed
        || matches!(p.proxy, ServiceHealth::PortConflict { .. })
        || p.policy.safe_mode_active
        || p.disk_root.warn
        || p.disk_data.is_some_and(|(_, d)| d.warn)
        || matches!(p.db, DbHealth::Failed(_));

    let update_state = p.slots.read_state();
    let prev = p.slots.available_previous_slot();
    let cur = p
        .slots
        .current_slot_version()
        .or(update_state.current_version)
        .unwrap_or_else(|| opsbox_version::VERSION.to_owned());

    WatchdogStatus {
        running: true,
        failed,
        degraded,
        updated_at: p.now,
        current_version: cur,
        previous_version: prev.clone(),
        can_rollback: prev.is_some(),
        update_message: update_state.last_outcome.map(|o| o.message),
        checks,
        recent_events: p.policy.events_vec(),
        safe_mode_listen: Some(p.listen.to_string()),
        ssh_open_until: p.ssh_open_until.filter(|u| *u > p.now),
    }
}

async fn apply_policy_actions(
    actions_list: Vec<PolicyAction>,
    cfg: &WatchdogConfig,
    slots: &SlotManager,
    policy: &mut WatchdogPolicy,
    now: i64,
) {
    for action in actions_list {
        match action {
            PolicyAction::RestartUnit(unit) => {
                let name = unit.unit_name();
                if !actions::systemd_available() {
                    policy.push_event(format!(
                        "{name} failed health check (systemd unavailable; restart skipped)."
                    ));
                    continue;
                }
                match tokio::task::spawn_blocking(move || actions::restart_unit(name)).await {
                    Ok(Ok(())) => policy.push_event(format!("Restarted {name}.")),
                    Ok(Err(e)) => {
                        warn!(unit = name, error = %e, "watchdog unit restart failed");
                        policy.push_event(e);
                    }
                    Err(_) => policy.push_event(format!("Restart task for {name} panicked.")),
                }
            }
            PolicyAction::FreeSpaceAndRestart => {
                let s = slots.clone();
                let x = cfg.xfer_dir.clone();
                let db = cfg.db_path.clone();
                let res = tokio::task::spawn_blocking(move || {
                    let report = cleanup::free_opsbox_space(&s, &x, &db, true);
                    let restart = if actions::systemd_available() {
                        actions::restart_all_opsbox()
                    } else {
                        Ok(())
                    };
                    (report.summary(), restart)
                })
                .await;
                match res {
                    Ok((summary, Ok(()))) => {
                        policy
                            .push_event(format!("Auto disk recovery: {summary} Restarted opsbox."));
                    }
                    Ok((summary, Err(e))) => {
                        policy.push_event(format!("Auto disk recovery: {summary} ({e})"));
                    }
                    Err(_) => policy.push_event("Auto disk recovery task panicked."),
                }
            }
            PolicyAction::RollbackToPrevious { to_version } => {
                let s = slots.clone();
                let db = cfg.db_path.clone();
                let res = tokio::task::spawn_blocking(move || {
                    let rolled = s.rollback_to_previous(&db, now)?;
                    if actions::systemd_available() {
                        actions::restart_all_opsbox()?;
                    }
                    Ok::<String, String>(rolled)
                })
                .await;
                match res {
                    Ok(Ok(ver)) => {
                        policy.push_event(format!(
                            "Rolled back automatically to {ver} and restarted opsbox."
                        ));
                    }
                    Ok(Err(e)) => {
                        policy.safe_mode_active = true;
                        warn!(target = %to_version, error = %e, "automatic rollback failed");
                        policy
                            .push_event(format!("Automatic rollback to {to_version} failed: {e}"));
                    }
                    Err(_) => {
                        policy.safe_mode_active = true;
                        policy.push_event("Automatic rollback task panicked.");
                    }
                }
            }
            PolicyAction::MarkUpdateVerified {
                to_version,
                from_version,
            } => {
                let s = slots.clone();
                let tv = to_version.clone();
                let fv = from_version.clone();
                let _ = tokio::task::spawn_blocking(move || s.mark_verified(&tv, &fv, now)).await;
                info!(version = %to_version, "post-update health check passed");
            }
        }
    }
}

pub async fn run(
    cfg: WatchdogConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), WatchdogError> {
    let listener = TcpListener::bind(cfg.listen).await?;
    run_with_listener(cfg, listener, shutdown).await
}

pub async fn run_with_listener(
    cfg: WatchdogConfig,
    listener: TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), WatchdogError> {
    ensure_runtime_dir(&cfg.runtime_dir)?;
    let local_listen = listener.local_addr()?;

    let db_check_path = cfg.db_path.clone();
    let db_initial =
        tokio::task::spawn_blocking(move || probes::check_db_integrity(&db_check_path))
            .await
            .unwrap_or_else(|_| DbHealth::Failed("SQLite startup check panicked.".into()));

    let (acceptor, active_cert) =
        server::load_or_create_tls_acceptor(&cfg.cert_path, &cfg.key_path)?;

    let slots = SlotManager {
        slots_dir: cfg.slots_dir.clone(),
        symlink: cfg.binary_symlink.clone(),
        state_file: cfg.update_state_file.clone(),
    };
    let auth = Arc::new(SafeAuth::new(
        cfg.db_path.clone(),
        cfg.master_key_path.clone(),
        cfg.runtime_dir.clone(),
    ));

    let mut policy = WatchdogPolicy::default();
    if let DbHealth::Failed(ref err) = db_initial {
        policy.safe_mode_active = true;
        policy.push_event(format!("Startup SQLite integrity check failed: {err}"));
    } else {
        policy.push_event("Watchdog started.");
    }

    let snapshot = Arc::new(RwLock::new(WatchdogStatus {
        running: true,
        failed: false,
        degraded: matches!(db_initial, DbHealth::Failed(_)),
        updated_at: now_secs(),
        current_version: slots
            .current_slot_version()
            .unwrap_or_else(|| opsbox_version::VERSION.to_owned()),
        previous_version: slots.available_previous_slot(),
        can_rollback: slots.available_previous_slot().is_some(),
        update_message: None,
        checks: vec![db_check(&db_initial)],
        recent_events: policy.events_vec(),
        safe_mode_listen: Some(local_listen.to_string()),
        ssh_open_until: None,
    }));
    {
        let st = snapshot
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        write_status_file(&cfg.runtime_dir, &st);
    }

    info!(
        listen = %local_listen,
        serve = %cfg.serve_listen,
        cert = %active_cert.display(),
        runtime_dir = %cfg.runtime_dir.display(),
        "opsbox watchdog started"
    );

    let wake_loop = Arc::new(Notify::new());
    let reset_failures = Arc::new(AtomicBool::new(false));

    let server_state = SafeModeState {
        auth,
        snapshot: snapshot.clone(),
        slots: slots.clone(),
        priv_socket: cfg.priv_socket.clone(),
        xfer_dir: cfg.xfer_dir.clone(),
        db_path: cfg.db_path.clone(),
        panel_port: cfg.serve_listen.port(),
        wake_loop: wake_loop.clone(),
        reset_failures: reset_failures.clone(),
    };

    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let srv_handle = tokio::spawn(server::serve(
        listener,
        acceptor,
        server_state,
        cfg.allow_ips.clone(),
        async move {
            let _ = stop_rx.await;
        },
    ));

    let data_dir = cfg
        .db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty() && *p != Path::new("/"))
        .map(Path::to_path_buf);

    let poll = cfg.poll_interval.max(Duration::from_millis(50));
    tokio::pin!(shutdown);

    loop {
        if reset_failures.swap(false, Ordering::AcqRel) {
            policy.clear_failures();
        }

        let proxy_expected = actions::proxy_unit_installed();
        let (serve_h, priv_h, proxy_h) = tokio::join!(
            probes::probe_serve(cfg.serve_listen),
            probes::probe_priv(&cfg.priv_socket),
            probes::probe_proxy(&cfg.proxy_socket, proxy_expected),
        );

        let disk_root_path = PathBuf::from("/");
        let dd = data_dir.clone();
        let (disk_root, disk_data, fb_ssh, restart_counts) =
            tokio::task::spawn_blocking(move || {
                let root = probes::probe_disk(&disk_root_path);
                let data = dd
                    .as_deref()
                    .map(|p| (p.to_path_buf(), probes::probe_disk(p)));
                let fb = actions::expire_fallback_ssh();
                (root, data, fb, probes::probe_restart_counts())
            })
            .await
            .unwrap_or_else(|_| (probes::probe_disk(Path::new("/")), None, None, [None; 3]));

        let now = now_secs();
        let disk_critical =
            disk_root.critical || disk_data.as_ref().is_some_and(|(_, d)| d.critical);
        let update_state = slots.read_state();
        let prev_slot = slots.available_previous_slot();

        let actions_to_run = policy.evaluate(&TickInput {
            now,
            restart_counts,
            serve_ok: serve_h.is_ok(),
            priv_ok: priv_h.is_ok(),
            proxy_ok: proxy_h.is_ok(),
            proxy_port_conflict: matches!(proxy_h, ServiceHealth::PortConflict { .. }),
            disk_critical,
            update_state,
            previous_slot_available: prev_slot,
        });

        apply_policy_actions(actions_to_run, &cfg, &slots, &mut policy, now).await;

        let cur_ssh_until = {
            let guard = snapshot
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match (fb_ssh, guard.ssh_open_until) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => (b > now).then_some(b),
                (None, None) => None,
            }
        };

        let next_status = build_status(SnapshotParams {
            now,
            listen: local_listen,
            serve: &serve_h,
            priv_h: &priv_h,
            proxy: &proxy_h,
            disk_root: &disk_root,
            disk_data: disk_data.as_ref().map(|(p, d)| (p.as_path(), d)),
            db: &db_initial,
            policy: &policy,
            slots: &slots,
            ssh_open_until: cur_ssh_until,
        });
        {
            let mut guard = snapshot
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *guard = next_status.clone();
        }
        write_status_file(&cfg.runtime_dir, &next_status);

        tokio::select! {
            () = &mut shutdown => break,
            () = wake_loop.notified() => {},
            () = tokio::time::sleep(poll) => {},
        }
    }

    let _ = stop_tx.send(());
    let _ = srv_handle.await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use opsbox_proto::{PrivOp, PrivReply, PrivRequest, PrivResponse, PrivResult, frame};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;

    #[tokio::test]
    async fn watchdog_loop_writes_status_and_tracks_probe_transitions() {
        let dir = tempfile::tempdir().unwrap();
        let rt = dir.path().join("run");
        let db = dir.path().join("opsbox.db");
        {
            let _store = opsbox_store::Store::open(&db).unwrap();
        }

        // Mock healthy HTTP `serve` /healthz listener.
        let serve_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let serve_addr = serve_listener.local_addr().unwrap();
        let (drop_serve_tx, mut drop_serve_rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut drop_serve_rx => break,
                    acc = serve_listener.accept() => {
                        if let Ok((mut s, _)) = acc {
                            tokio::spawn(async move {
                                let mut buf = [0u8; 512];
                                let _ = s.read(&mut buf).await;
                                let body = r#"{"status":"ok","version":"0.1.5"}"#;
                                let resp = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                                    body.len()
                                );
                                let _ = s.write_all(resp.as_bytes()).await;
                            });
                        }
                    }
                }
            }
        });

        // Mock healthy `priv` Unix socket answering Ping.
        let priv_sock = dir.path().join("priv.sock");
        let u_listener = UnixListener::bind(&priv_sock).unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = u_listener.accept().await {
                tokio::spawn(async move {
                    let mut hdr = [0u8; 4];
                    if stream.read_exact(&mut hdr).await.is_err() {
                        return;
                    }
                    let len = frame::body_len(hdr).unwrap();
                    let mut buf = vec![0u8; len];
                    stream.read_exact(&mut buf).await.unwrap();
                    let req: PrivRequest = frame::decode(&buf).unwrap();
                    assert!(matches!(req.op, PrivOp::Ping));
                    let resp = PrivResponse {
                        id: req.id,
                        result: PrivResult::Success {
                            reply: PrivReply::Pong {
                                version: "0.1.5".into(),
                            },
                        },
                    };
                    let out = frame::encode(&resp).unwrap();
                    let _ = stream.write_all(&out).await;
                });
            }
        });

        let wd_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cfg = WatchdogConfig {
            listen: wd_listener.local_addr().unwrap(),
            allow_ips: Vec::new(),
            serve_listen: serve_addr,
            priv_socket: priv_sock,
            proxy_socket: dir.path().join("missing-proxy.sock"),
            db_path: db,
            master_key_path: dir.path().join("master.key"),
            runtime_dir: rt.clone(),
            slots_dir: dir.path().join("bin"),
            binary_symlink: dir.path().join("opsbox"),
            update_state_file: dir.path().join("update-state.json"),
            xfer_dir: dir.path().join("xfer"),
            cert_path: dir.path().join("panel.crt"),
            key_path: dir.path().join("panel.key"),
            poll_interval: Duration::from_millis(75),
        };

        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            run_with_listener(cfg, wd_listener, async {
                let _ = stop_rx.await;
            })
            .await
            .unwrap();
        });

        // Wait briefly for initial healthy status.json write.
        let status_file = rt.join(STATUS_FILE);
        for _ in 0..30 {
            if status_file.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let st: WatchdogStatus =
            serde_json::from_str(&fs::read_to_string(&status_file).unwrap()).unwrap();
        assert!(st.running);
        assert!(!st.failed, "checks: {:?}", st.checks);

        // Shut down mock `serve` and observe failure status transition.
        let _ = drop_serve_tx.send(());
        let mut saw_fail = false;
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if let Ok(text) = fs::read_to_string(&status_file)
                && let Ok(s) = serde_json::from_str::<WatchdogStatus>(&text)
                && s.failed
            {
                saw_fail = true;
                break;
            }
        }
        assert!(saw_fail, "watchdog should report serve failure");

        let _ = stop_tx.send(());
        handle.await.unwrap();
    }
}
