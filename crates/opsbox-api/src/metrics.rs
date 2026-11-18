//! `GET /api/v1/metrics`, `GET /api/v1/attention` and `WS /api/v1/stream/metrics`.
//!
//! The JSON matches what the mock in `web/js/api.js` returns, so the Overview page and
//! the status bar need no changes: percentages for the history arrays, GB for memory
//! and disk, MB/s for the network.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::response::Response;
use opsbox_metrics::{Metrics, Point, Severity, Snapshot};
use opsbox_proto::ErrorCode;
use opsbox_store::AttentionStateRecord;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;

use crate::json::ApiJson;
use crate::session::{Client, Viewer, origin_matches_host};
use crate::{AppState, HttpError};

const GB: f64 = (1u64 << 30) as f64;
const MB: f64 = (1u64 << 20) as f64;
const DEFAULT_WINDOW_SECS: usize = 48;
const MAX_WINDOW_SECS: usize = 3600;
/// Long windows are averaged into at most this many points.
const MAX_POINTS: usize = 120;
const MIN_STEP_SECS: usize = 2;

#[derive(Deserialize)]
pub(crate) struct WindowQuery {
    window: Option<String>,
}

/// `48s`, `5m`, `1h` (or a bare number of seconds), at most one hour.
pub fn parse_window(text: Option<&str>) -> Result<usize, HttpError> {
    let bad = || {
        HttpError::new(
            ErrorCode::Validation,
            "The window must look like 48s, 5m or 1h (at most 1h).",
        )
    };
    let Some(t) = text else {
        return Ok(DEFAULT_WINDOW_SECS);
    };
    let (digits, mult) = match t.chars().last() {
        Some('s') => (&t[..t.len() - 1], 1),
        Some('m') => (&t[..t.len() - 1], 60),
        Some('h') => (&t[..t.len() - 1], 3600),
        Some(c) if c.is_ascii_digit() => (t, 1),
        _ => return Err(bad()),
    };
    let n: usize = digits.parse().map_err(|_| bad())?;
    let secs = n.checked_mul(mult).ok_or_else(bad)?;
    if (1..=MAX_WINDOW_SECS).contains(&secs) {
        Ok(secs)
    } else {
        Err(bad())
    }
}

/// Average `points` into buckets of `step` points each, newest bucket last. Buckets are
/// aligned to the newest point so the latest value is always included.
fn buckets(points: &[Point], step: usize) -> Vec<&[Point]> {
    let mut out: Vec<&[Point]> = points.rchunks(step).collect();
    out.reverse();
    out
}

fn mean(points: &[Point], f: impl Fn(&Point) -> f64) -> f64 {
    if points.is_empty() {
        0.0
    } else {
        points.iter().map(f).sum::<f64>() / points.len() as f64
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn proc_json(q: &opsbox_metrics::ProcView) -> Value {
    let mut v = json!({
        "name": q.name,
        "pid": q.pid,
        "cpu": round1(f64::from(q.cpu)),
        "mem": (q.rss as f64 / MB).round(),
        "self": q.is_self,
        "kind": q.kind,
    });
    if let Some(c) = &q.container {
        let mut cjson = json!({ "id": c.id, "name": c.name });
        if let Some(svc) = &c.service {
            cjson["service"] = json!(svc);
        }
        v["container"] = cjson;
    }
    if let Some(unit) = &q.unit {
        v["unit"] = json!(unit);
    }
    v
}

/// The full metrics object for the last `window_secs` seconds.
pub fn view(snap: &Snapshot, history: &[Point], window_secs: usize) -> Value {
    let step = (window_secs / MAX_POINTS).max(MIN_STEP_SECS);
    let b = buckets(history, step);
    let p = &snap.point;
    // The disk the Overview card shows: "/" if present, else the first real filesystem.
    let root = snap
        .disks
        .iter()
        .find(|d| d.mount == "/")
        .or(snap.disks.first());
    let procs: Vec<Value> = snap.procs.iter().map(proc_json).collect();
    let containers: Vec<Value> = snap
        .containers
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "name": c.name,
                "image": c.image,
                "cpu": round1(c.cpu_pct),
                "mem": (c.mem_used_bytes as f64 / MB).round(),
                "state": c.state,
                "status": c.status,
                "managed": c.managed,
                "service": c.service,
            })
        })
        .collect();
    json!({
        "ts": p.ts,
        "cpu": b.iter().map(|c| mean(c, |x| f64::from(x.cpu)).round()).collect::<Vec<_>>(),
        "mem": b.iter().map(|c| mean(c, |x| f64::from(x.mem_pct())).round()).collect::<Vec<_>>(),
        "cpuNow": round1(f64::from(p.cpu)),
        "cores": p.cores,
        "memUsed": p.mem_used as f64 / GB,
        // Totals are printed as they come ("/ 48 GB"), so they are rounded here.
        "memTotal": round1(p.mem_total as f64 / GB),
        "cache": p.cache as f64 / GB,
        "swapUsed": p.swap_used as f64 / GB,
        "swapTotal": p.swap_total as f64 / GB,
        "disk": root.map_or(0.0, |d| d.used as f64 / GB),
        "diskTotal": round1(root.map_or(0.0, |d| d.total as f64 / GB)),
        "disks": snap.disks.iter().map(|d| json!({ "mount": d.mount, "fstype": d.fstype, "used": d.used as f64 / GB, "total": d.total as f64 / GB })).collect::<Vec<_>>(),
        "netIn": p.net_in / MB,
        "netOut": p.net_out / MB,
        "diskRead": p.disk_read / MB,
        "diskWrite": p.disk_write / MB,
        "conns": p.conns,
        "load": p.load,
        "procs": procs,
        "containers": containers,
    })
}

/// The first sample arrives about two seconds after start-up; callers that come in
/// earlier wait for it rather than getting an error.
async fn latest_or_wait(m: &Metrics) -> Result<Arc<Snapshot>, HttpError> {
    for _ in 0..40 {
        if let Some(s) = m.latest() {
            return Ok(s);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let why = m.last_error();
    tracing::warn!(error = ?why, "metrics requested but there is no sample yet");
    Err(HttpError::new(
        ErrorCode::Unavailable,
        if why.is_some() {
            "Machine metrics are unavailable: the privileged helper could not be read. Check that opsbox-priv is running."
        } else {
            "Machine metrics are still starting up. Try again in a few seconds."
        },
    ))
}

pub(crate) async fn get_metrics(
    State(state): State<AppState>,
    Viewer(_): Viewer,
    Query(q): Query<WindowQuery>,
) -> Result<Json<Value>, HttpError> {
    let window = parse_window(q.window.as_deref())?;
    let snap = latest_or_wait(&state.metrics).await?;
    Ok(Json(view(&snap, &state.metrics.window(window), window)))
}

const MAX_SNOOZE_MINUTES: i64 = 7 * 24 * 60;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AttentionEntry {
    pub id: String,
    pub kind: String,
    pub severity: Severity,
    pub since: i64,
    pub text: String,
    pub link: (String, String),
    #[serde(rename = "alertId", skip_serializing_if = "Option::is_none")]
    pub alert_id: Option<String>,
    pub read: bool,
    pub snoozed: bool,
    #[serde(rename = "snoozedUntil", skip_serializing_if = "Option::is_none")]
    pub snoozed_until: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AttentionResponse {
    pub items: Vec<AttentionEntry>,
    pub unread: usize,
    pub critical: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SnoozeBody {
    minutes: i64,
}

fn unix_now() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    )
    .unwrap_or(0)
    .max(1)
}

fn validate_attention_id(id: &str) -> Result<(), HttpError> {
    if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
        return Err(HttpError::new(
            ErrorCode::Validation,
            "Invalid notification ID.",
        ));
    }
    Ok(())
}

fn cert_since(
    state: &AppState,
    key: &str,
    cert: &opsbox_proxy::CertInfo,
    now: i64,
    active_cert_keys: &mut HashSet<String>,
) -> i64 {
    if cert.not_after.is_some_and(|exp| exp <= now) {
        return cert.not_after.unwrap_or(now).max(1);
    }
    active_cert_keys.insert(key.to_owned());
    let mut map = state
        .attention_seen
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *map.entry(key.to_owned()).or_insert(now)
}

async fn collect_viewer_attention(
    state: &AppState,
    who: &opsbox_auth::Principal,
    now: i64,
) -> Vec<AttentionEntry> {
    let mut items = state.metrics.attention();
    if let Some((ver, since)) = state.updater.latest_available_with_since() {
        items.push(opsbox_metrics::Attention {
            id: format!("update:{ver}"),
            kind: "update".into(),
            severity: Severity::Info,
            since: since.max(1),
            text: format!("opsbox {ver} is available"),
            link: ("Review in Settings →".to_owned(), "#/settings".to_owned()),
        });
    }
    if let Ok(failed) = state.store.list_failed_latest_deploys() {
        for (svc, dep) in failed {
            if !who.allows_service(&svc.name) {
                continue;
            }
            if who.user.role == opsbox_proto::Role::Deployer
                && !who.via_token
                && svc.owner_id.as_deref() != Some(who.user.id.as_str())
            {
                continue;
            }
            let reason = dep.error_message.as_deref().unwrap_or("Deploy failed");
            let since = dep.finished_at.unwrap_or(dep.started_at).max(1);
            items.push(opsbox_metrics::Attention {
                id: format!("deploy:{}", svc.name),
                kind: "deploy".into(),
                severity: Severity::Critical,
                since,
                text: format!("Deploy failed for {}: {reason}", svc.name),
                link: ("View deploy →".to_owned(), format!("#/apps?s={}", svc.name)),
            });
        }
    }
    if let Some(ref proxy) = state.proxy
        && let Ok(st) = proxy
            .clone()
            .with_timeout(Duration::from_millis(500))
            .status("attention-certs")
            .await
    {
        let mut active_cert_keys = HashSet::new();
        let cert_map: HashMap<String, opsbox_proxy::CertInfo> = st
            .certs
            .into_iter()
            .map(|c| (c.domain.to_ascii_lowercase(), c))
            .collect();
        if who.user.role == opsbox_proto::Role::Admin
            && let Some(panel_dom) = crate::panel::active_panel_domain(state)
            && let Some(cert) = cert_map.get(&panel_dom.to_ascii_lowercase())
            && crate::domains::cert_needs_attention(cert)
        {
            let reason = cert
                .last_error
                .as_deref()
                .unwrap_or("Certificate is expired or unreadable");
            let id = format!("cert:{panel_dom}");
            let severity = if cert.not_after.is_some_and(|exp| exp <= now) {
                Severity::Critical
            } else {
                Severity::Warning
            };
            let since = cert_since(state, &id, cert, now, &mut active_cert_keys);
            items.push(opsbox_metrics::Attention {
                id,
                kind: "tls".into(),
                severity,
                since,
                text: format!("TLS certificate renewal failed for {panel_dom}: {reason}"),
                link: ("Review in Settings →".to_owned(), "#/settings".to_owned()),
            });
        }
        if let Ok(dom_svcs) = state.store.list_all_service_domains_with_services() {
            for (dom, svc) in dom_svcs {
                if !who.allows_service(&svc.name) {
                    continue;
                }
                if who.user.role == opsbox_proto::Role::Deployer
                    && !who.via_token
                    && svc.owner_id.as_deref() != Some(who.user.id.as_str())
                {
                    continue;
                }
                if let Some(cert) = cert_map.get(&dom.domain.to_ascii_lowercase())
                    && crate::domains::cert_needs_attention(cert)
                {
                    let reason = cert
                        .last_error
                        .as_deref()
                        .unwrap_or("Certificate is expired or unreadable");
                    let id = format!("cert:{}", dom.domain);
                    let severity = if cert.not_after.is_some_and(|exp| exp <= now) {
                        Severity::Critical
                    } else {
                        Severity::Warning
                    };
                    let since = cert_since(state, &id, cert, now, &mut active_cert_keys);
                    items.push(opsbox_metrics::Attention {
                        id,
                        kind: "tls".into(),
                        severity,
                        since,
                        text: format!(
                            "TLS certificate renewal failed for {} ({}): {reason}",
                            dom.domain, svc.name
                        ),
                        link: (
                            "View domains →".to_owned(),
                            format!("#/apps?s={}&tab=domains", svc.name),
                        ),
                    });
                }
            }
        }
        // Only prune cert first-seen entries when all certs were visible (admin).
        if who.user.role == opsbox_proto::Role::Admin && who.scoped_service.is_none() {
            let mut map = state
                .attention_seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            map.retain(|k, _| !k.starts_with("cert:") || active_cert_keys.contains(k));
        }
    }
    items.extend(crate::backups::collect_attention_items(state, who));
    crate::alerts::merge_alert_attention(state, who, items).await
}

fn apply_attention_state(
    entries: Vec<AttentionEntry>,
    states: &HashMap<String, AttentionStateRecord>,
    now: i64,
) -> AttentionResponse {
    let mut items = Vec::with_capacity(entries.len());
    for mut item in entries {
        if let Some(st) = states.get(&item.id) {
            item.read = st.read_since.is_some_and(|rs| item.since <= rs)
                && st
                    .read_severity
                    .is_some_and(|rsev| item.severity.rank() <= rsev);
            let active_snooze = st.snooze_until.filter(|&u| u > now).filter(|_| {
                st.snooze_severity
                    .is_none_or(|ssev| item.severity.rank() <= ssev)
            });
            if active_snooze.is_some() && item.severity != Severity::Critical {
                continue;
            }
            item.snoozed = active_snooze.is_some();
            item.snoozed_until = active_snooze;
        }
        items.push(item);
    }
    items.sort_by_key(|a| {
        (
            a.severity != Severity::Critical,
            a.snoozed,
            Reverse(a.since),
            a.id.clone(),
        )
    });
    let unread = items.iter().filter(|a| !a.read).count();
    let critical = items
        .iter()
        .filter(|a| a.severity == Severity::Critical && !a.read)
        .count();
    AttentionResponse {
        items,
        unread,
        critical,
    }
}

async fn load_user_attention_states(
    state: &AppState,
    user_id: &str,
) -> Result<HashMap<String, AttentionStateRecord>, HttpError> {
    let store = state.store.clone();
    let uid = user_id.to_owned();
    tokio::task::spawn_blocking(move || store.list_attention_state(&uid))
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(|_| HttpError::internal())
}

pub(crate) async fn get_attention(
    State(state): State<AppState>,
    Viewer(who): Viewer,
) -> Result<Json<AttentionResponse>, HttpError> {
    let now = unix_now();
    let entries = collect_viewer_attention(&state, &who, now).await;
    let states = load_user_attention_states(&state, &who.user.id).await?;
    Ok(Json(apply_attention_state(entries, &states, now)))
}

pub(crate) async fn read_attention(
    State(state): State<AppState>,
    Viewer(who): Viewer,
    Path(id): Path<String>,
) -> Result<Json<Value>, HttpError> {
    validate_attention_id(&id)?;
    let now = unix_now();
    let entries = collect_viewer_attention(&state, &who, now).await;
    let target = entries
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "No such notification."))?
        .clone();
    let store = state.store.clone();
    let uid = who.user.id.clone();
    let item_id = target.id.clone();
    tokio::task::spawn_blocking(move || {
        store.mark_attention_read(&uid, &item_id, target.since, target.severity.rank(), now)
    })
    .await
    .map_err(|_| HttpError::internal())?
    .map_err(|_| HttpError::internal())?;
    let states = load_user_attention_states(&state, &who.user.id).await?;
    let resp = apply_attention_state(entries, &states, now);
    Ok(Json(
        json!({ "ok": true, "items": resp.items, "unread": resp.unread, "critical": resp.critical }),
    ))
}

pub(crate) async fn read_all_attention(
    State(state): State<AppState>,
    Viewer(who): Viewer,
) -> Result<Json<Value>, HttpError> {
    let now = unix_now();
    let entries = collect_viewer_attention(&state, &who, now).await;
    let states = load_user_attention_states(&state, &who.user.id).await?;
    let visible = apply_attention_state(entries.clone(), &states, now);
    let pairs: Vec<(String, i64, i64)> = visible
        .items
        .iter()
        .map(|item| (item.id.clone(), item.since, item.severity.rank()))
        .collect();
    if !pairs.is_empty() {
        let store = state.store.clone();
        let uid = who.user.id.clone();
        tokio::task::spawn_blocking(move || store.mark_all_attention_read(&uid, &pairs, now))
            .await
            .map_err(|_| HttpError::internal())?
            .map_err(|_| HttpError::internal())?;
    }
    let next_states = load_user_attention_states(&state, &who.user.id).await?;
    let resp = apply_attention_state(entries, &next_states, now);
    Ok(Json(
        json!({ "ok": true, "items": resp.items, "unread": resp.unread, "critical": resp.critical }),
    ))
}

pub(crate) async fn snooze_attention(
    State(state): State<AppState>,
    Viewer(who): Viewer,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<SnoozeBody>,
) -> Result<Json<Value>, HttpError> {
    validate_attention_id(&id)?;
    if !(1..=MAX_SNOOZE_MINUTES).contains(&body.minutes) {
        return Err(HttpError::new(
            ErrorCode::Validation,
            "Snooze duration must be between 1 minute and 7 days.",
        ));
    }
    let now = unix_now();
    let entries = collect_viewer_attention(&state, &who, now).await;
    let target = entries
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "No such notification."))?
        .clone();
    let until = now.saturating_add(body.minutes.saturating_mul(60));
    let store = state.store.clone();
    let uid = who.user.id.clone();
    let item_id = target.id.clone();
    tokio::task::spawn_blocking(move || {
        store.snooze_attention(&uid, &item_id, until, target.severity.rank(), now)
    })
    .await
    .map_err(|_| HttpError::internal())?
    .map_err(|_| HttpError::internal())?;
    let states = load_user_attention_states(&state, &who.user.id).await?;
    let resp = apply_attention_state(entries, &states, now);
    Ok(Json(
        json!({ "ok": true, "items": resp.items, "unread": resp.unread, "critical": resp.critical }),
    ))
}

/// `WS /api/v1/stream/metrics`: one text frame a second, `{ "type": "metrics", "ts", "data" }`
/// where `data` is the same object `GET /metrics` returns for the default window.
pub(crate) async fn stream(
    // Extractors run in this order: authenticate before looking at the upgrade headers.
    Viewer(who): Viewer,
    client: Client,
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> Result<Response, HttpError> {
    // A page on another site must not be able to open this socket with the visitor's
    // cookies (cross-site WebSocket hijacking). Browsers always send Origin here.
    if !origin_matches_host(&client.headers) {
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "This request came from another site and was refused.",
        ));
    }
    let slot = crate::session::stream_slot(&state)?;
    Ok(
        crate::session::bounded_socket(ws).on_upgrade(move |socket| async move {
            let _slot = slot;
            tokio::select! {
                () = crate::session::stream_ended(state.clone(), who, client.ip) => {},
                () = pump(socket, state.metrics) => {},
            }
        }),
    )
}

fn frame(snap: &Snapshot, m: &Metrics) -> Message {
    let data = view(snap, &m.window(DEFAULT_WINDOW_SECS), DEFAULT_WINDOW_SECS);
    Message::text(json!({ "type": "metrics", "ts": snap.point.ts, "data": data }).to_string())
}

async fn pump(mut socket: WebSocket, metrics: Metrics) {
    let mut rx = metrics.subscribe();
    // Do not make the page wait a second for its first frame.
    if let Some(s) = metrics.latest()
        && socket.send(frame(&s, &metrics)).await.is_err()
    {
        return;
    }
    loop {
        tokio::select! {
            snap = rx.recv() => match snap {
                Ok(s) => if socket.send(frame(&s, &metrics)).await.is_err() { return },
                // A slow client missed some frames: carry on from the newest.
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            },
            incoming = socket.recv() => match incoming {
                None | Some(Err(_) | Ok(Message::Close(_))) => return,
                Some(Ok(_)) => {} // pings are answered by the library; anything else is ignored
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opsbox_metrics::DiskView;

    fn p(ts: i64, cpu: f32) -> Point {
        Point {
            ts,
            cpu,
            cores: vec![1],
            mem_total: 8 << 30,
            mem_used: 2 << 30,
            cache: 1 << 30,
            swap_total: 0,
            swap_used: 0,
            load: [0.5, 0.4, 0.3],
            net_in: 2.0 * MB,
            net_out: MB,
            disk_read: 0.0,
            disk_write: 0.0,
            conns: 3,
        }
    }

    #[test]
    fn windows_parse_and_are_bounded() {
        assert_eq!(parse_window(None).unwrap(), 48);
        assert_eq!(parse_window(Some("48s")).unwrap(), 48);
        assert_eq!(parse_window(Some("5m")).unwrap(), 300);
        assert_eq!(parse_window(Some("1h")).unwrap(), 3600);
        assert_eq!(parse_window(Some("90")).unwrap(), 90);
        for bad in [
            "",
            "0s",
            "2h",
            "3601",
            "s",
            "-5s",
            "1.5m",
            "5x",
            "99999999999999999999h",
        ] {
            assert!(parse_window(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_default_window_gives_24_points_like_the_mock() {
        let history: Vec<Point> = (0..60).map(|i| p(i, i as f32)).collect();
        let snap = Snapshot {
            point: history.last().unwrap().clone(),
            disks: vec![],
            procs: vec![],
            containers: vec![],
        };
        let v = view(&snap, &history[60 - 48..], 48);
        assert_eq!(v["cpu"].as_array().unwrap().len(), 24);
        // The newest bucket averages the two newest points, seconds 58 and 59.
        assert_eq!(v["cpu"][23], 59.0);
    }

    #[test]
    fn long_windows_are_averaged_into_at_most_120_points() {
        let history: Vec<Point> = (0..3600).map(|i| p(i, 10.0)).collect();
        let snap = Snapshot {
            point: history[3599].clone(),
            disks: vec![],
            procs: vec![],
            containers: vec![],
        };
        let v = view(&snap, &history, 3600);
        assert_eq!(v["cpu"].as_array().unwrap().len(), 120);
        assert!(v["cpu"].as_array().unwrap().iter().all(|x| x == 10.0));
    }

    #[test]
    fn units_match_the_mock_gb_and_mb_per_second() {
        let snap = Snapshot {
            point: p(5, 12.34),
            disks: vec![
                DiskView {
                    mount: "/boot".into(),
                    fstype: "ext4".into(),
                    used: 1 << 30,
                    total: 2 << 30,
                },
                DiskView {
                    mount: "/".into(),
                    fstype: "ext4".into(),
                    used: 30 << 30,
                    total: 100 << 30,
                },
            ],
            procs: vec![],
            containers: vec![],
        };
        let v = view(&snap, std::slice::from_ref(&snap.point), 48);
        assert_eq!(
            (
                v["memUsed"].clone(),
                v["memTotal"].clone(),
                v["cache"].clone()
            ),
            (json!(2.0), json!(8.0), json!(1.0))
        );
        assert_eq!(
            (v["netIn"].clone(), v["netOut"].clone()),
            (json!(2.0), json!(1.0))
        );
        assert_eq!(
            (v["disk"].clone(), v["diskTotal"].clone()),
            (json!(30.0), json!(100.0)),
            "the card shows /, not the first disk"
        );
        assert_eq!(v["cpuNow"], 12.3);
        assert_eq!(v["mem"][0], 25.0);
        assert_eq!(v["conns"], 3);
    }

    #[test]
    fn processes_are_in_the_mocks_shape() {
        let snap = Snapshot {
            point: p(1, 1.0),
            disks: vec![],
            procs: vec![opsbox_metrics::ProcView {
                pid: 7,
                name: "opsbox".into(),
                cpu: 0.123,
                rss: 14 << 20,
                is_self: true,
                kind: opsbox_metrics::ProcKind::System,
                container: None,
                unit: Some("opsbox.service".into()),
            }],
            containers: vec![],
        };
        let v = view(&snap, std::slice::from_ref(&snap.point), 48);
        assert_eq!(
            v["procs"][0],
            json!({
                "name": "opsbox",
                "pid": 7,
                "cpu": 0.1,
                "mem": 14.0,
                "self": true,
                "kind": "system",
                "unit": "opsbox.service",
            })
        );
    }

    #[test]
    fn process_container_kind_includes_name_and_service() {
        let snap = Snapshot {
            point: p(1, 1.0),
            disks: vec![],
            procs: vec![opsbox_metrics::ProcView {
                pid: 1204,
                name: "postgres".into(),
                cpu: 8.1,
                rss: 612 << 20,
                is_self: false,
                kind: opsbox_metrics::ProcKind::Container,
                container: Some(opsbox_metrics::ProcContainer {
                    id: "abc123def456".into(),
                    name: "web-app".into(),
                    service: Some("web".into()),
                }),
                unit: None,
            }],
            containers: vec![],
        };
        let v = view(&snap, std::slice::from_ref(&snap.point), 48);
        assert_eq!(
            v["procs"][0],
            json!({
                "name": "postgres",
                "pid": 1204,
                "cpu": 8.1,
                "mem": 612.0,
                "self": false,
                "kind": "container",
                "container": { "id": "abc123def456", "name": "web-app", "service": "web" },
            })
        );
    }

    fn item(id: &str, severity: Severity, since: i64) -> AttentionEntry {
        AttentionEntry {
            id: id.into(),
            kind: id.split(':').next().unwrap_or("info").into(),
            severity,
            since,
            text: format!("text for {id}"),
            link: ("View →".into(), "#/overview".into()),
            alert_id: None,
            read: false,
            snoozed: false,
            snoozed_until: None,
        }
    }

    #[test]
    fn apply_attention_state_sorts_counts_and_respects_snooze_and_reunread_invariants() {
        let now = 2_000;
        let entries = vec![
            item("update:0.2.0", Severity::Info, 1_950),
            item("security:host", Severity::Warning, 1_900),
            item("backup:opsbox", Severity::Warning, 1_800),
            item("disk:/", Severity::Critical, 1_700),
            item("deploy:web", Severity::Critical, 1_850),
        ];
        let mut states = HashMap::new();
        // Read security:host at matching since/severity -> stays read.
        states.insert(
            "security:host".into(),
            AttentionStateRecord {
                item_id: "security:host".into(),
                read_since: Some(1_900),
                read_severity: Some(Severity::Warning.rank()),
                read_at: Some(1_910),
                snooze_until: None,
                snooze_severity: None,
                updated_at: 1_910,
            },
        );
        // Snoozed warning backup:opsbox -> hidden while now < snooze_until.
        states.insert(
            "backup:opsbox".into(),
            AttentionStateRecord {
                item_id: "backup:opsbox".into(),
                read_since: None,
                read_severity: None,
                read_at: None,
                snooze_until: Some(now + 3600),
                snooze_severity: Some(Severity::Warning.rank()),
                updated_at: now,
            },
        );
        // Snoozed critical deploy:web -> stays in list (after unsnoozed criticals), dimmed, AND still counted!
        states.insert(
            "deploy:web".into(),
            AttentionStateRecord {
                item_id: "deploy:web".into(),
                read_since: Some(1_850),
                read_severity: Some(Severity::Warning.rank()), // escalated from warning to critical -> unread again!
                read_at: Some(1_860),
                snooze_until: Some(now + 3600),
                snooze_severity: Some(Severity::Critical.rank()),
                updated_at: now,
            },
        );
        // Read update:0.2.0 at older since (1900 < 1950) -> unread again!
        states.insert(
            "update:0.2.0".into(),
            AttentionStateRecord {
                item_id: "update:0.2.0".into(),
                read_since: Some(1_900),
                read_severity: Some(Severity::Info.rank()),
                read_at: Some(1_910),
                snooze_until: None,
                snooze_severity: None,
                updated_at: 1_910,
            },
        );

        let res = apply_attention_state(entries, &states, now);
        let ids: Vec<_> = res.items.iter().map(|i| i.id.as_str()).collect();
        // Order: unsnoozed critical (disk:/), snoozed critical (deploy:web), non-critical newest-first.
        assert_eq!(
            ids,
            ["disk:/", "deploy:web", "update:0.2.0", "security:host"]
        );
        assert!(!res.items[0].snoozed && !res.items[0].read);
        assert!(res.items[1].snoozed && !res.items[1].read);
        assert!(!res.items[2].read);
        assert!(res.items[3].read);
        assert_eq!(res.unread, 3);
        assert_eq!(res.critical, 2);

        // Once snooze expires, backup:opsbox returns to the list & unread count.
        let later = apply_attention_state(
            vec![item("backup:opsbox", Severity::Warning, 1_800)],
            &states,
            now + 3601,
        );
        assert_eq!(later.items.len(), 1);
        assert_eq!(later.unread, 1);
    }
}
