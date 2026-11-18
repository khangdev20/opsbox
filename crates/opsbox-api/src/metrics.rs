//! `GET /api/v1/metrics`, `GET /api/v1/attention` and `WS /api/v1/stream/metrics`.
//!
//! The JSON matches what the mock in `web/js/api.js` returns, so the Overview page and
//! the status bar need no changes: percentages for the history arrays, GB for memory
//! and disk, MB/s for the network.

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use opsbox_metrics::{Metrics, Point, Snapshot};
use opsbox_proto::ErrorCode;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;

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
    let procs: Vec<Value> = snap
        .procs
        .iter()
        .map(|q| json!({ "name": q.name, "pid": q.pid, "cpu": round1(f64::from(q.cpu)), "mem": (q.rss as f64 / MB).round(), "self": q.is_self }))
        .collect();
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

pub(crate) async fn get_attention(
    State(state): State<AppState>,
    Viewer(who): Viewer,
) -> Json<Value> {
    let mut items = state.metrics.attention();
    if let Some(ver) = state.updater.latest_available() {
        items.push(opsbox_metrics::Attention {
            kind: "update",
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
            items.push(opsbox_metrics::Attention {
                kind: "deploy",
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
        let cert_map: std::collections::HashMap<String, opsbox_proxy::CertInfo> = st
            .certs
            .into_iter()
            .map(|c| (c.domain.to_ascii_lowercase(), c))
            .collect();
        if who.user.role == opsbox_proto::Role::Admin
            && let Some(panel_dom) = crate::panel::active_panel_domain(&state)
            && let Some(cert) = cert_map.get(&panel_dom.to_ascii_lowercase())
            && crate::domains::cert_needs_attention(cert)
        {
            let reason = cert
                .last_error
                .as_deref()
                .unwrap_or("Certificate is expired or unreadable");
            items.push(opsbox_metrics::Attention {
                kind: "tls",
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
                    items.push(opsbox_metrics::Attention {
                        kind: "tls",
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
    }
    items.extend(crate::backups::collect_attention_items(&state, &who));
    Json(json!(items))
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
            }],
            containers: vec![],
        };
        let v = view(&snap, std::slice::from_ref(&snap.point), 48);
        assert_eq!(
            v["procs"][0],
            json!({ "name": "opsbox", "pid": 7, "cpu": 0.1, "mem": 14.0, "self": true })
        );
    }
}
