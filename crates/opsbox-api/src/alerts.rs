//! Alert orchestration. System inputs come from priv-backed services; secret
//! configuration is sealed/opened through priv. serve never accesses master.key.
use crate::{
    AppState, HttpError, RequestId,
    json::ApiJson,
    secrets::{self, ReauthBody, begin_reveal, no_store_headers},
    session::{Admin, Client, deny_token},
    system::{audit, invalid},
};
use axum::{
    Json,
    extract::{Extension, Path, State},
    http::HeaderMap,
};
use opsbox_alerts::{
    Channel, Engine, Incident, Observation, Quiet, State as AlertState,
    channels::{self, Settings},
};
use opsbox_proto::{ErrorCode, Role, Secret};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

const KEY: &str = "alerts.engine.v1";
#[derive(Clone, Default)]
pub struct Alerts {
    lock: Arc<Mutex<()>>,
    started: Arc<AtomicBool>,
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
fn load(s: &AppState) -> Result<Engine, HttpError> {
    s.store
        .get_setting(KEY)
        .map_err(|_| HttpError::internal())?
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| HttpError::internal())
        .map(Option::unwrap_or_default)
}
fn save(s: &AppState, e: &Engine) -> Result<(), HttpError> {
    s.store
        .set_setting(
            KEY,
            &serde_json::to_value(e).map_err(|_| HttpError::internal())?,
        )
        .map_err(|_| HttpError::internal())
}
fn context(id: &str) -> String {
    format!("alerts:channel:{id}")
}
fn valid_id(id: &str) -> Result<(), HttpError> {
    if ["c1", "c2", "c3", "c4", "c5"].contains(&id) {
        Ok(())
    } else {
        Err(HttpError::new(
            ErrorCode::NotFound,
            "No such notification channel.",
        ))
    }
}
fn payload(s: &AppState, i: &Incident) -> Value {
    let base = crate::panel::active_panel_domain(s)
        .map(|domain| format!("https://{domain}/"))
        .unwrap_or_default();
    json!({"id":format!("{}:{}",i.id,i.revision),"name":i.name,"rule":i.rule,"target":i.target,"state":i.state,"level":i.level,"message":if i.state==AlertState::Resolved {format!("Recovered: {}",i.msg)} else {i.msg.clone()},"link":format!("{base}{}",i.link),"alertsLink":format!("{base}#/alerts?id={}",encode(&i.id))})
}
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
async fn deliver(s: &AppState, c: &Channel, mut p: Value) -> Result<(), HttpError> {
    let secret = s
        .priv_client
        .secret_open("alert-delivery", None, &context(&c.id), &c.sealed)
        .await?;
    let config: Settings =
        serde_json::from_str(secret.expose()).map_err(|_| HttpError::internal())?;
    if matches!(config, Settings::Discord { .. }) {
        p["host"] = json!(crate::panel::active_panel_domain(s).unwrap_or_else(|| "opsbox".into()));
    }
    tokio::task::spawn_blocking(move || channels::send(config, p, now()))
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(|msg| HttpError::new(ErrorCode::Unavailable, msg))
}
impl Alerts {
    pub fn spawn(&self, s: AppState) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let delivery_state = s.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(15));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            tick.tick().await;
            loop {
                tick.tick().await;
                let (obs, known) = collect(&s, now()).await;
                let _guard = s.alerts.lock.lock().await;
                let result = load(&s).and_then(|mut e| {
                    e.evaluate(obs, &known, now());
                    save(&s, &e)
                });
                if result.is_err() {
                    tracing::warn!("Alert evaluation could not complete; it will retry.");
                }
            }
        });
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if delivery_cycle(&delivery_state).await.is_err() {
                    tracing::warn!("Alert delivery could not complete; it will retry.");
                }
            }
        });
    }
}
async fn delivery_cycle(s: &AppState) -> Result<(), HttpError> {
    let prepared = {
        let _guard = s.alerts.lock.lock().await;
        let mut e = load(s)?;
        let candidate = e
            .channels
            .values()
            .flat_map(|c| {
                e.incidents
                    .values()
                    .filter(|i| e.due(&i.id, &c.id, now()))
                    .map(|i| {
                        let at = i
                            .deliveries
                            .get(&c.id)
                            .map_or(0, |d| d.attempt.unwrap_or(d.at));
                        (at, i.id.clone(), c.clone())
                    })
            })
            .min_by_key(|(at, _, _)| *at)
            .map(|(_, id, c)| (id, c));
        let Some((id, c)) = candidate else {
            return Ok(());
        };
        let i = &e.incidents[&id];
        let revision = i.revision;
        let firing = i.state == AlertState::Firing;
        let p = payload(s, i);
        e.attempted(&id, &c.id, now());
        save(s, &e)?;
        (id, c, revision, firing, p)
    };
    let (id, c, revision, firing, p) = prepared;
    let result = deliver(s, &c, p).await;
    let _guard = s.alerts.lock.lock().await;
    let mut e = load(s)?;
    // Configuration and incident state can change during network IO. A successful
    // firing must still qualify for a later resolved delivery, even if it recovered
    // in flight; never mark an unseen revision as delivered.
    if e.channels
        .get(&c.id)
        .is_some_and(|current| current.sealed == c.sealed)
    {
        if result.is_ok()
            && let Some(i) = e.incidents.get_mut(&id)
        {
            let d = i.deliveries.entry(c.id.clone()).or_default();
            d.revision = revision;
            d.at = now();
            d.attempt = None;
            d.firing_sent = firing;
        }
        if let Some(channel) = e.channels.get_mut(&c.id) {
            channel.verified = result.is_ok();
            channel.error = result
                .err()
                .map(|_| "Delivery failed. Check this channel and send a test.".into());
        }
        save(s, &e)?;
    }
    Ok(())
}

async fn collect(s: &AppState, at: i64) -> (Vec<Observation>, BTreeSet<String>) {
    let mut out = Vec::new();
    let mut known = BTreeSet::new();
    known.insert("health".into());
    if let Some(snap) = s
        .metrics
        .latest()
        .filter(|snap| at.saturating_sub(snap.point.ts) < 30)
    {
        for k in ["disk", "cpu", "memory"] {
            known.insert(k.into());
        }
        for d in &snap.disks {
            if d.pct() > 75.0 {
                out.push(Observation::new(
                    "disk",
                    &d.mount,
                    format!(
                        "Disk {} is at {:.0}%. Free space or expand the disk.",
                        d.mount,
                        d.pct()
                    ),
                    "#/cleanup",
                    d.pct() >= 95.0,
                ));
            }
        }
        if snap.point.cpu > 90.0 {
            out.push(Observation::new(
                "cpu",
                "host",
                format!("CPU is at {:.0}%. Review busy processes.", snap.point.cpu),
                "#/overview",
                snap.point.cpu >= 99.0,
            ));
        }
        if snap.point.mem_pct() > 90.0 {
            out.push(Observation::new(
                "memory",
                "host",
                format!(
                    "Memory is at {:.0}%. Review running services.",
                    snap.point.mem_pct()
                ),
                "#/overview",
                snap.point.mem_pct() >= 95.0,
            ));
        }
    } else {
        out.push(Observation::new(
            "health",
            "host",
            "Machine metrics are unavailable. Check that the privileged helper is running.".into(),
            "#/recovery",
            true,
        ));
    }
    let (facts, down) = s.metrics.alert_sources(at);
    if let Some(f) = facts {
        known.extend(["units".into(), "security".into()]);
        for name in f.failed_units {
            out.push(Observation::new(
                "units",
                &name,
                format!("System service {name} failed. Review its logs."),
                "#/system",
                true,
            ));
        }
        if f.updates_security.is_some_and(|n| n > 0) {
            out.push(Observation::new(
                "security",
                "host",
                "Security updates are waiting. Review and install them.".into(),
                "#/system",
                false,
            ));
        }
        if f.updates_security.is_none() {
            known.remove("security");
        }
    }
    if let Some(names) = down {
        known.insert("containers".into());
        for name in names {
            out.push(Observation::new(
                "containers",
                &name,
                format!("Container {name} is stopped or restarting. Review its logs."),
                "#/containers",
                true,
            ));
        }
    }
    if let Ok(failed) = s.store.list_failed_latest_deploys() {
        known.insert("deploy".into());
        for (svc, _) in failed {
            out.push(Observation::new(
                "deploy",
                &svc.id,
                format!(
                    "The latest deploy for {} failed. Review the build log.",
                    svc.name
                ),
                &format!("#/apps?s={}", encode(&svc.name)),
                true,
            ));
        }
    }
    if let Ok(jobs) = s.store.list_backup_jobs() {
        known.extend(["backup".into(), "missed".into()]);
        for j in jobs {
            if j.last_status.as_deref() == Some("failed") {
                out.push(Observation::new(
                    "backup",
                    &j.id,
                    format!("Backup {} failed. Review the backup and retry.", j.name),
                    "#/backups",
                    true,
                ));
            }
            if j.enabled
                && j.last_status.as_deref() != Some("running")
                && opsbox_backup::cron::is_run_missed(&j.cron, j.last_run_at, j.created_at, at, 900)
            {
                out.push(Observation::new(
                    "missed",
                    &j.id,
                    format!(
                        "Backup {} is overdue. Check its schedule and destination.",
                        j.name
                    ),
                    "#/backups",
                    false,
                ));
            }
        }
        if s.store
            .get_setting("capacity.backup_alert")
            .ok()
            .flatten()
            .and_then(|v| v["at"].as_i64())
            .is_some_and(|t| at - t < 86400)
        {
            out.push(Observation::new("backup","capacity","A backup waited 30 minutes for memory and is running at lowest priority. Stop unused workloads or upgrade the server.".into(),"#/backups",false));
        }
        match s.store.get_setting("backups.last_restore_test_error") {
            Ok(Some(v)) if v.as_str().is_some_and(|v| !v.is_empty()) => out.push(Observation::new(
                "backup",
                "restore-test",
                "A restore test failed. Review backups before relying on them.".into(),
                "#/backups",
                true,
            )),
            Err(_) => {
                known.remove("backup");
            }
            _ => {}
        }
    }
    match s.store.list_failed_backup_snapshots() {
        Ok(snaps) => {
            for snap in snaps
                .into_iter()
                .filter(|snap| snap.check_state == opsbox_proto::BackupCheckState::ChecksumFailed)
            {
                out.push(Observation::new(
                    "backup",
                    &format!("checksum-{}", snap.id),
                    "A backup checksum failed. Inspect the snapshot and make a new backup.".into(),
                    "#/backups",
                    true,
                ));
            }
        }
        Err(_) => {
            known.remove("backup");
        }
    }
    if let Some(proxy) = &s.proxy {
        match proxy
            .clone()
            .with_timeout(Duration::from_secs(2))
            .status("alerts-certs")
            .await
        {
            Ok(status) => {
                if status.network.port_conflict {
                    out.push(Observation::new("health", "proxy-ports", "The proxy is running on other ports because its usual ports are busy. Open Settings > Network.".into(), "#/settings?section=network", false));
                }
                // Keep certificate state unknown if its domain ownership cannot be read.
                if let Ok(domains) = s.store.list_all_service_domains_with_services() {
                    known.insert("tls".into());
                    for c in status.certs {
                        if c.not_after
                            .is_some_and(|exp| exp.saturating_sub(at) < 14 * 86400)
                            || c.last_error.is_some()
                        {
                            let link = domains
                                .iter()
                                .find(|(d, _)| d.domain.eq_ignore_ascii_case(&c.domain))
                                .map_or_else(
                                    || "#/settings".into(),
                                    |(_, svc)| {
                                        format!("#/apps?s={}&tab=domains", encode(&svc.name))
                                    },
                                );
                            out.push(Observation::new("tls",&c.domain,format!("Certificate for {} expires soon or could not renew. Review its domain settings.",c.domain),&link,c.not_after.is_some_and(|exp|exp<=at)));
                        }
                    }
                }
            }
            Err(_) => out.push(Observation::new(
                "health",
                "proxy",
                "The web proxy is unavailable. Review its service and logs.".into(),
                "#/recovery",
                true,
            )),
        }
    } else {
        known.insert("tls".into());
    }
    if let Ok(events) = s.store.alert_security_events(at - 3600) {
        known.extend(["ssh".into(), "signins".into()]);
        let mut failures = 0;
        for event in events {
            let detail: Value = serde_json::from_str(&event.detail).unwrap_or(Value::Null);
            if event.action == "system.ssh.emergency"
                && event.result == "ok"
                && detail["until"].as_i64().is_some_and(|until| until > at)
            {
                out.push(Observation::new(
                    "ssh",
                    event.target.as_deref().unwrap_or("host"),
                    "Emergency SSH access is open. Close it when finished.".into(),
                    "#/access",
                    true,
                ));
            }
            if event.action == "auth.login"
                && event.result != "ok"
                && time::OffsetDateTime::parse(
                    &event.ts,
                    &time::format_description::well_known::Rfc3339,
                )
                .is_ok_and(|ts| ts.unix_timestamp() >= at - 900)
            {
                failures += 1;
            }
        }
        if failures >= 5 {
            out.push(Observation::new(
                "signins",
                "host",
                format!("{failures} sign-ins failed in 15 minutes. Review access and allowed IPs."),
                "#/security",
                true,
            ));
        }
    }
    (out, known)
}

pub(crate) async fn get(
    State(s): State<AppState>,
    Admin(_): Admin,
) -> Result<Json<Value>, HttpError> {
    let _guard = s.alerts.lock.lock().await;
    let e = load(&s)?;
    let discord_display = if let Some(c) = e.channels.get("c5") {
        s.priv_client
            .secret_open("alert-list", None, &context("c5"), &c.sealed)
            .await
            .ok()
            .and_then(|opened| serde_json::from_str::<Settings>(opened.expose()).ok())
            .and_then(|settings| {
                if let Settings::Discord { url } = settings {
                    let token = url.expose().rsplit('/').next().unwrap_or("");
                    Some(format!(
                        "…/{}",
                        token
                            .chars()
                            .rev()
                            .take(4)
                            .collect::<String>()
                            .chars()
                            .rev()
                            .collect::<String>()
                    ))
                } else {
                    None
                }
            })
    } else {
        None
    };
    let channels: Vec<_> = [
        ("c1", "Email"),
        ("c2", "Slack"),
        ("c3", "Telegram"),
        ("c4", "Webhook"),
        ("c5", "Discord"),
    ]
    .into_iter()
    .map(|(id, name)| {
        let c = e.channels.get(id);
        json!({
            "id": id,
            "name": name,
            "on": c.is_some(),
            "verified": c.is_some_and(|c| c.verified),
            "error": c.and_then(|c| c.error.clone()),
            "to": if id == "c5" { discord_display.as_deref().unwrap_or(if c.is_some() { "Configured" } else { "" }) } else if c.is_some() { "Configured" } else { "" },
            "for": "all alerts",
            "hasValue": c.is_some(),
        })
    })
    .collect();
    let rules: Vec<_> = e
        .rules
        .iter()
        .map(|r| {
            let mut v = serde_json::to_value(r).unwrap_or(Value::Null);
            v["notify"] = json!("all channels");
            v
        })
        .collect();
    let incidents:Vec<_>=e.incidents.values().filter(|i|matches!(i.state,AlertState::Pending|AlertState::Firing|AlertState::Resolved)).map(|i|json!({"id":i.id,"name":i.name,"msg":i.msg,"state":i.state,"level":i.level,"since":i.since,"link":i.link,"silenceUntil":i.silence_until})).collect();
    let history: Vec<_> = e
        .history
        .iter()
        .rev()
        .map(|h| json!({"id":h.id,"name":h.name,"msg":h.msg,"state":h.state,"ts":h.ts}))
        .collect();
    Ok(Json(
        json!({"channels":channels,"rules":rules,"incidents":incidents,"history":history,"quiet":{"from":e.quiet.from,"to":e.quiet.to,"tz":"UTC"}}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuleBody {
    on: bool,
}
pub(crate) async fn set_rule(
    State(s): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Path(id): Path<String>,
    ApiJson(b): ApiJson<RuleBody>,
) -> Result<Json<Value>, HttpError> {
    let _guard = s.alerts.lock.lock().await;
    let mut e = load(&s)?;
    let r = e
        .rules
        .iter_mut()
        .find(|r| r.id == id)
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "No such alert rule."))?;
    r.on = b.on;
    save(&s, &e)?;
    audit(
        &s,
        &who,
        &client.ip,
        "alerts.rule",
        Some(&id),
        json!({"on":b.on}),
    );
    Ok(Json(json!({"ok":true})))
}
pub(crate) async fn connect(
    State(s): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
    ApiJson(b): ApiJson<Settings>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    valid_id(&id)?;
    if b.id() != id {
        return Err(invalid("kind", "Channel type does not match."));
    }
    b.validate().map_err(|msg| invalid("channel", msg))?;
    let plaintext = Secret::new(serde_json::to_string(&b).map_err(|_| HttpError::internal())?);
    let sealed = s
        .priv_client
        .secret_seal(&rid, Some(&who.user.name), &context(&id), plaintext)
        .await?;
    let _guard = s.alerts.lock.lock().await;
    let mut e = load(&s)?;
    e.channels.insert(
        id.clone(),
        Channel {
            id: id.clone(),
            sealed,
            verified: false,
            error: None,
        },
    );
    for i in e.incidents.values_mut() {
        i.deliveries.remove(&id);
    }
    save(&s, &e)?;
    audit(
        &s,
        &who,
        &client.ip,
        "alerts.channel.connect",
        Some(&id),
        json!({}),
    );
    Ok(Json(json!({"ok":true})))
}
pub(crate) async fn disconnect(
    State(s): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Path(id): Path<String>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    valid_id(&id)?;
    let _guard = s.alerts.lock.lock().await;
    let mut e = load(&s)?;
    e.channels.remove(&id);
    save(&s, &e)?;
    audit(
        &s,
        &who,
        &client.ip,
        "alerts.channel.disconnect",
        Some(&id),
        json!({}),
    );
    Ok(Json(json!({"ok":true})))
}

/// `POST /api/v1/alerts/channels/{id}/reveal`: open sealed channel settings after re-auth.
pub(crate) async fn reveal_channel(
    State(s): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ReauthBody>,
) -> Result<(HeaderMap, Json<Value>), HttpError> {
    deny_token(&who)?;
    valid_id(&id)?;
    if id == "c5" {
        return Err(invalid(
            "channel",
            "Discord webhook URLs cannot be revealed. Paste a new URL to replace it.",
        ));
    }
    if let Err(e) = begin_reveal(&s, &who, &rid, &client.ip, &body).await {
        secrets::audit_reveal_denied(
            &s,
            &who,
            &client.ip,
            "alerts.channel.reveal.denied",
            Some(&id),
            "reauth",
        );
        return Err(e);
    }
    let e = load(&s)?;
    let c = e.channels.get(&id).ok_or_else(|| {
        HttpError::new(
            ErrorCode::NotFound,
            "Connect this channel before revealing it.",
        )
    })?;
    let opened = s
        .priv_client
        .secret_open(&rid, Some(&who.user.name), &context(&id), &c.sealed)
        .await?;
    let settings: Settings =
        serde_json::from_str(opened.expose()).map_err(|_| HttpError::internal())?;
    // Re-serialize through serde so Secret fields become plaintext JSON for the reveal
    // response only; list endpoints never open the seal.
    let value = serde_json::to_value(&settings).map_err(|_| HttpError::internal())?;
    secrets::audit_reveal(
        &s,
        &who,
        Some(&client.ip),
        "alerts.channel.reveal",
        Some(&id),
        &format!("channel:{id}"),
        json!({}),
    );
    Ok((
        no_store_headers(),
        Json(json!({ "id": id, "settings": value })),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TestBody {
    channel: Option<String>,
}
pub(crate) async fn test(
    State(s): State<AppState>,
    Admin(who): Admin,
    client: Client,
    ApiJson(b): ApiJson<TestBody>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    let selected: Vec<_> = {
        let _guard = s.alerts.lock.lock().await;
        let e = load(&s)?;
        e.channels
            .values()
            .filter(|c| b.channel.as_ref().is_none_or(|id| id == &c.id))
            .cloned()
            .collect()
    };
    if selected.is_empty() {
        return Err(invalid("channel", "Connect a notification channel first."));
    }
    let results=futures_util::future::join_all(selected.iter().map(|c|deliver(&s,c,json!({"id":format!("test-{}",now()),"name":"Test alert","state":"test","level":"warning","message":"Your opsbox notification channel is working.","link":"#/alerts"})))).await;
    let failed = results.iter().any(Result::is_err);
    let _guard = s.alerts.lock.lock().await;
    let mut e = load(&s)?;
    for (configured, result) in selected.iter().zip(results) {
        if let Some(c) = e.channels.get_mut(&configured.id)
            && c.sealed == configured.sealed
        {
            c.verified = result.is_ok();
            c.error = result
                .err()
                .map(|_| "Delivery failed. Check the address and credentials.".into());
        }
    }
    save(&s, &e)?;
    audit(
        &s,
        &who,
        &client.ip,
        "alerts.test",
        b.channel.as_deref(),
        json!({"ok":!failed}),
    );
    if failed {
        return Err(HttpError::new(
            ErrorCode::Unavailable,
            "A test could not be delivered. Check the channel address and credentials.",
        ));
    }
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SilenceBody {
    id: String,
    seconds: i64,
}
pub(crate) async fn silence(
    State(s): State<AppState>,
    Admin(who): Admin,
    client: Client,
    ApiJson(b): ApiJson<SilenceBody>,
) -> Result<Json<Value>, HttpError> {
    if !(0..=86400 * 7).contains(&b.seconds) {
        return Err(invalid(
            "seconds",
            "Silence must be between zero and seven days.",
        ));
    }
    let _guard = s.alerts.lock.lock().await;
    let mut e = load(&s)?;
    let i = e
        .incidents
        .get_mut(&b.id)
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "No such alert."))?;
    i.silence_until = now() + b.seconds;
    save(&s, &e)?;
    audit(
        &s,
        &who,
        &client.ip,
        "alerts.silence",
        Some(&b.id),
        json!({"seconds":b.seconds}),
    );
    Ok(Json(json!({"ok":true})))
}
pub(crate) async fn quiet(
    State(s): State<AppState>,
    Admin(who): Admin,
    client: Client,
    ApiJson(b): ApiJson<Quiet>,
) -> Result<Json<Value>, HttpError> {
    if Quiet::minute(&b.from).is_none() || Quiet::minute(&b.to).is_none() {
        return Err(invalid(
            "from",
            "Use 24-hour HH:MM times in UTC. Equal times disable quiet hours.",
        ));
    }
    let _guard = s.alerts.lock.lock().await;
    let mut e = load(&s)?;
    e.quiet = b;
    save(&s, &e)?;
    audit(&s, &who, &client.ip, "alerts.quiet", None, json!({}));
    Ok(Json(json!({"ok":true})))
}

/// Map an alert rule + target to the canonical attention item `id` (`kind:subject`) so
/// built-in checks and fired alerts describing the same issue deduplicate cleanly.
pub(crate) fn canonical_attention_id(
    store: &opsbox_store::Store,
    rule: &str,
    target: &str,
) -> String {
    match (rule, target) {
        ("deploy", svc_id) => {
            let subject = store
                .get_service(svc_id)
                .ok()
                .flatten()
                .map_or_else(|| svc_id.to_owned(), |s| s.name);
            format!("deploy:{subject}")
        }
        ("tls", domain) => format!("cert:{domain}"),
        ("backup", "restore-test") => "backup:restore-test".to_owned(),
        ("backup", "capacity") => "backup:capacity".to_owned(),
        ("backup", t) if t.starts_with("checksum-") => "backup:checksum".to_owned(),
        ("backup" | "missed", job_id) => {
            if job_id == opsbox_store::OPSBOX_SELF_JOB_ID {
                "backup:opsbox".to_owned()
            } else {
                format!("backup:{job_id}")
            }
        }
        (r, t) => format!("{r}:{t}"),
    }
}

/// Only administrators can follow host-wide alerts. Other roles retain their
/// existing scoped attention links and cannot infer global alert targets.
pub(crate) async fn merge_alert_attention(
    s: &AppState,
    who: &opsbox_auth::Principal,
    items: Vec<opsbox_metrics::Attention>,
) -> Vec<crate::metrics::AttentionEntry> {
    use opsbox_metrics::Severity;
    use std::collections::BTreeMap;

    let mut map: BTreeMap<String, crate::metrics::AttentionEntry> = BTreeMap::new();
    for a in items {
        let entry = crate::metrics::AttentionEntry {
            id: a.id.clone(),
            kind: a.kind,
            severity: a.severity,
            since: a.since.max(1),
            text: a.text,
            link: a.link,
            alert_id: None,
            read: false,
            snoozed: false,
            snoozed_until: None,
        };
        if let Some(prev) = map.get_mut(&entry.id) {
            if entry.severity > prev.severity {
                *prev = entry;
            } else if entry.severity == prev.severity && entry.since < prev.since {
                prev.since = entry.since;
            }
        } else {
            map.insert(entry.id.clone(), entry);
        }
    }

    if who.user.role != Role::Admin || who.scoped_service.is_some() {
        return map.into_values().collect();
    }
    let _guard = s.alerts.lock.lock().await;
    let Ok(e) = load(s) else {
        return map.into_values().collect();
    };
    for i in e
        .incidents
        .values()
        .filter(|i| i.state == AlertState::Firing)
    {
        let id = canonical_attention_id(&s.store, &i.rule, &i.target);
        let alert_sev = Severity::parse(&i.level);
        let alert_since = i.since.max(1);
        let alert_link = (
            "View alert →".to_owned(),
            format!("#/alerts?id={}", encode(&i.id)),
        );
        if let Some(existing) = map.get_mut(&id) {
            if existing.alert_id.is_some() {
                if alert_sev > existing.severity
                    || (alert_sev == existing.severity && alert_since > existing.since)
                {
                    existing.severity = alert_sev;
                    existing.since = alert_since;
                    existing.text = i.msg.clone();
                    existing.link = alert_link;
                    existing.alert_id = Some(i.id.clone());
                }
            } else {
                if alert_sev > existing.severity {
                    existing.severity = alert_sev;
                    existing.since = alert_since;
                    existing.text = i.msg.clone();
                } else {
                    existing.since = existing.since.min(alert_since);
                }
                existing.link = alert_link;
                existing.alert_id = Some(i.id.clone());
            }
        } else {
            map.insert(
                id.clone(),
                crate::metrics::AttentionEntry {
                    id,
                    kind: i.rule.clone(),
                    severity: alert_sev,
                    since: alert_since,
                    text: i.msg.clone(),
                    link: alert_link,
                    alert_id: Some(i.id.clone()),
                    read: false,
                    snoozed: false,
                    snoozed_until: None,
                },
            );
        }
    }
    map.into_values().collect()
}
