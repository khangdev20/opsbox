//! `GET /cleanup/scan`, `POST /cleanup/run`, `PUT /cleanup/auto` (docs/BACKEND_SPEC.md 7.11).

use axum::Json;
use axum::extract::{Extension, State};
use opsbox_proto::{CleanupFreed, CleanupItem, ErrorCode};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::json::ApiJson;
use crate::session::{Admin, Client};
use crate::system::{OnBody, audit, patient};
use crate::tasks::Outcome;
use crate::{AppState, HttpError, RequestId};

const GB: f64 = (1u64 << 30) as f64;

/// Bytes shown as the page's "GB" (the same unit the Overview uses).
fn gb(bytes: u64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let b = bytes as f64;
    (b / GB * 100.0).round() / 100.0
}

/// The bar's colour for a scanner (a name the stylesheet knows).
fn colour(id: &str) -> &'static str {
    match id {
        "journal" => "text",
        "pkg-cache" => "warn",
        "tmp" | "coredumps" => "muted",
        id if id.starts_with("docker-") && id != "docker-volumes" => "accent",
        _ => "ls",
    }
}

fn item_json(i: &CleanupItem) -> Value {
    json!({
        "id": i.id, "title": i.title, "desc": i.desc, "bytes": i.bytes, "gb": gb(i.bytes),
        // Selected by default: everything safe to run unattended.
        "on": i.safe && !i.warn, "safe": i.safe, "warn": i.warn, "color": colour(&i.id),
    })
}

/// `GET /cleanup/scan`
pub(crate) async fn scan(
    State(state): State<AppState>,
    Admin(_): Admin,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    let s = patient(&state, 120).cleanup_scan(&rid).await?;
    // The disk the Overview shows: "/" if present, else the first real filesystem.
    let disk = state.metrics.latest().and_then(|snap| {
        snap.disks
            .iter()
            .find(|d| d.mount == "/")
            .or(snap.disks.first())
            .map(|d| (d.used, d.total))
    });
    let (used, total) = disk.unwrap_or((0, 0));
    Ok(Json(json!({
        "items": s.items.iter().map(item_json).collect::<Vec<_>>(),
        "disk": gb(used), "total": gb(total).round(), "system": gb(s.system_bytes),
        "auto": s.auto,
        "scanned": clock_now(),
    })))
}

fn clock_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let day = secs % 86400;
    format!("{:02}:{:02} UTC", day / 3600, day % 3600 / 60)
}

#[derive(Deserialize)]
pub(crate) struct RunBody {
    ids: Vec<String>,
}

/// `POST /cleanup/run` starts a task; the result is `{ freed, items: [{id, bytes, error}] }`.
pub(crate) async fn run(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<RunBody>,
) -> Result<(axum::http::StatusCode, Json<Value>), HttpError> {
    if body.ids.is_empty() {
        return Err(HttpError::new(
            ErrorCode::Validation,
            "Choose what to clean.",
        ));
    }
    if body.ids.len() > 20 {
        return Err(HttpError::new(ErrorCode::Validation, "Too many items."));
    }
    let (pc, name, ip, s) = (
        patient(&state, 30 * 60),
        who.user.name.clone(),
        client.ip.clone(),
        state.clone(),
    );
    let ids = body.ids.clone();
    let task = state.tasks.start("cleanup.run", async move {
        let freed: Vec<CleanupFreed> = pc.cleanup_run(&rid, &name, &ids).await?;
        let total: u64 = freed.iter().map(|f| f.bytes).sum();
        let failed: Vec<&str> = freed
            .iter()
            .filter(|f| f.error.is_some())
            .map(|f| f.id.as_str())
            .collect();
        audit(
            &s,
            &who,
            &ip,
            "cleanup.run",
            Some(&ids.join(",")),
            json!({ "freed_bytes": total, "failed": failed }),
        );
        let log = freed
            .iter()
            .map(|f| match &f.error {
                None => format!("{}: freed {:.2} GB", f.id, gb(f.bytes)),
                Some(e) => format!("{}: failed: {e}", f.id),
            })
            .collect();
        Ok(Outcome {
            log,
            result: json!({
                "freed": gb(total), "bytes": total,
                "items": freed.iter().map(|f| json!({ "id": f.id, "bytes": f.bytes, "error": f.error })).collect::<Vec<_>>(),
            }),
        })
    })?;
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "task": task })),
    ))
}

/// `PUT /cleanup/auto`
pub(crate) async fn set_auto(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<OnBody>,
) -> Result<Json<Value>, HttpError> {
    patient(&state, 30)
        .cleanup_auto_set(&rid, &who.user.name, body.on)
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        "cleanup.auto",
        None,
        json!({ "on": body.on }),
    );
    Ok(Json(json!({ "ok": true })))
}
