//! The sampler: once a second ask `priv` for counters, keep the live ring, publish
//! to subscribers and write minute/hour rollups.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use opsbox_priv::{ClientError, PrivClient};
use opsbox_proto::{DockerContainerStat, SystemFacts};
use opsbox_store::Store;
use tokio::sync::{Notify, broadcast};
use tracing::{debug, warn};

use crate::attention::{self, Attention};
use crate::engine::{Engine, Point, Snapshot};
use crate::rollup::Rollup;

/// How often `SystemFacts` is refreshed (it can run `apt-check`, which is slow).
const FACTS_EVERY: Duration = Duration::from_secs(300);
/// Every n-th sample also scans the processes.
const PROCS_EVERY: u64 = 2;
/// Poll running containers every 2 s when Docker is active.
const DOCKER_ACTIVE_EVERY: Duration = Duration::from_secs(2);
/// Back off polling when Docker is not installed or stopped.
const DOCKER_IDLE_EVERY: Duration = Duration::from_secs(15);
const MAX_CONTAINER_STATS: usize = 8;

struct State {
    engine: Engine,
    rollup: Rollup,
    latest: Option<Arc<Snapshot>>,
    facts: SystemFacts,
    facts_ok: bool,
    facts_at: i64,
    down_containers: Option<(i64, Vec<String>)>,
    containers: Vec<DockerContainerStat>,
    restarting_containers: Vec<String>,
    restarting_known: bool,
    attention_since: HashMap<String, i64>,
    /// Whether the last sample worked, for the "unavailable" answer.
    last_error: Option<String>,
}

#[derive(Clone)]
pub struct Metrics {
    state: Arc<Mutex<State>>,
    tx: broadcast::Sender<Arc<Snapshot>>,
    docker_wake: Arc<Notify>,
}

impl Metrics {
    /// Start sampling in the background. The task ends when the runtime does.
    pub fn spawn(priv_client: PrivClient, store: Arc<Store>) -> Self {
        let (tx, _) = broadcast::channel(16);
        let me = Self {
            state: Arc::new(Mutex::new(State {
                engine: Engine::new(),
                rollup: Rollup::default(),
                latest: None,
                facts: SystemFacts::default(),
                facts_ok: false,
                facts_at: 0,
                down_containers: None,
                containers: Vec::new(),
                restarting_containers: Vec::new(),
                restarting_known: false,
                attention_since: HashMap::new(),
                last_error: None,
            })),
            tx,
            docker_wake: Arc::new(Notify::new()),
        };
        tokio::spawn(me.clone().run(priv_client.clone(), store));
        tokio::spawn(me.clone().refresh_facts(priv_client.clone()));
        tokio::spawn(me.clone().refresh_docker(priv_client));
        me
    }

    /// Wake the Docker background poller immediately after an install or container action.
    pub fn notify_docker(&self) {
        self.docker_wake.notify_one();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The newest sample, if there has been one.
    pub fn latest(&self) -> Option<Arc<Snapshot>> {
        self.lock().latest.clone()
    }

    /// Why there is no data, when sampling is failing.
    pub fn last_error(&self) -> Option<String> {
        let s = self.lock();
        s.latest.is_none().then(|| s.last_error.clone()).flatten()
    }

    /// The last `secs` seconds, oldest first.
    pub fn window(&self, secs: usize) -> Vec<Point> {
        self.lock().engine.window(secs)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Snapshot>> {
        self.tx.subscribe()
    }

    /// The "needs attention" items right now, with stable first-seen timestamps.
    pub fn attention(&self) -> Vec<Attention> {
        let mut s = self.lock();
        reconcile_attention_since(&mut s, unix_now())
    }

    /// Cached successful source groups for alerts. Expired samples are unknown.
    pub fn alert_sources(&self, now: i64) -> (Option<SystemFacts>, Option<Vec<String>>) {
        let s = self.lock();
        let facts = (s.facts_ok && now.saturating_sub(s.facts_at) < 600).then(|| s.facts.clone());
        let down = s
            .down_containers
            .as_ref()
            .filter(|(at, _)| now.saturating_sub(*at) < 30)
            .map(|(_, names)| names.clone());
        (facts, down)
    }

    async fn run(self, priv_client: PrivClient, store: Arc<Store>) {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut n: u64 = 0;
        let mut last_warn = std::time::Instant::now() - Duration::from_secs(3600);
        let mut last_purge = 0i64;
        loop {
            tick.tick().await;
            n += 1;
            let request_id = format!("metrics-{n}");
            let raw = tokio::time::timeout(
                Duration::from_secs(3),
                priv_client.metrics_sample(&request_id, n.is_multiple_of(PROCS_EVERY)),
            )
            .await;
            let raw = match raw {
                Ok(Ok(raw)) => raw,
                other => {
                    let why = match other {
                        Ok(Err(e)) => e.to_string(),
                        _ => "the privileged helper did not answer in time".to_owned(),
                    };
                    self.lock().last_error = Some(why.clone());
                    // One warning a minute, not one a second.
                    if last_warn.elapsed() > Duration::from_secs(60) {
                        warn!(error = %why, "metrics sampling failed");
                        last_warn = std::time::Instant::now();
                    }
                    continue;
                }
            };
            let now = unix_now();
            let (snapshot, rows) = {
                let mut s = self.lock();
                s.last_error = None;
                let Some(mut snap) = s.engine.ingest(raw, now) else {
                    continue;
                };
                snap.containers = s.containers.clone();
                let rows = s.rollup.push(&snap.point, &snap.disks);
                let snap = Arc::new(snap);
                s.latest = Some(snap.clone());
                reconcile_attention_since(&mut s, now);
                (snap, rows)
            };
            // No subscribers is the normal case; the send error just says so.
            let _ = self.tx.send(snapshot);
            if !rows.is_empty() || now - last_purge > 3600 {
                let store = store.clone();
                let purge = now - last_purge > 3600;
                if purge {
                    last_purge = now;
                }
                tokio::task::spawn_blocking(move || {
                    for (g, row) in rows {
                        if let Err(e) = store.put_metric(g, &row) {
                            warn!(error = %e, "could not store a metric rollup");
                        }
                    }
                    if purge {
                        let _ = store.purge_metrics(now);
                    }
                });
            }
        }
    }

    async fn refresh_facts(self, priv_client: PrivClient) {
        loop {
            match tokio::time::timeout(Duration::from_secs(60), priv_client.system_facts("facts"))
                .await
            {
                Ok(Ok(f)) => {
                    let mut s = self.lock();
                    s.facts = f;
                    s.facts_ok = true;
                    s.facts_at = unix_now();
                    let at = s.facts_at;
                    reconcile_attention_since(&mut s, at);
                }
                other => debug!(?other, "could not refresh system facts"),
            }
            tokio::time::sleep(FACTS_EVERY).await;
        }
    }

    async fn refresh_docker(self, priv_client: PrivClient) {
        loop {
            let delay = match tokio::time::timeout(
                Duration::from_secs(4),
                priv_client.docker_list_containers("docker-watch"),
            )
            .await
            {
                Ok(Ok(list)) => {
                    let restarting: Vec<String> = list
                        .iter()
                        .filter(|c| c.state == "restarting")
                        .map(|c| c.name.clone())
                        .collect();
                    let stats = if list.iter().any(|c| c.state == "running") {
                        match tokio::time::timeout(
                            Duration::from_secs(4),
                            priv_client.docker_stats("docker-stats"),
                        )
                        .await
                        {
                            Ok(Ok(mut st)) => {
                                st.sort_by(|a, b| b.cpu_pct.total_cmp(&a.cpu_pct));
                                st.truncate(MAX_CONTAINER_STATS);
                                Some(st)
                            }
                            _ => None,
                        }
                    } else {
                        Some(Vec::new())
                    };
                    let mut s = self.lock();
                    s.down_containers = Some((
                        unix_now(),
                        list.iter()
                            .filter(|c| {
                                matches!(c.state.as_str(), "exited" | "dead" | "restarting")
                            })
                            .map(|c| c.name.clone())
                            .collect(),
                    ));
                    s.restarting_containers = restarting;
                    s.restarting_known = true;
                    if let Some(st) = stats {
                        s.containers = st;
                    }
                    reconcile_attention_since(&mut s, unix_now());
                    DOCKER_ACTIVE_EVERY
                }
                Ok(Err(ClientError::Remote(_))) => {
                    let mut s = self.lock();
                    s.containers.clear();
                    s.restarting_containers.clear();
                    s.restarting_known = true;
                    reconcile_attention_since(&mut s, unix_now());
                    DOCKER_IDLE_EVERY
                }
                other => {
                    debug!(?other, "docker metrics refresh skipped");
                    Duration::from_secs(5)
                }
            };
            tokio::select! {
                () = tokio::time::sleep(delay) => {}
                () = self.docker_wake.notified() => {}
            }
        }
    }
}

fn reconcile_attention_since(s: &mut State, now: i64) -> Vec<Attention> {
    let disks = s
        .latest
        .as_ref()
        .map(|l| l.disks.as_slice())
        .unwrap_or_default();
    let mut facts = s.facts.clone();
    if s.restarting_known {
        facts.restarting_containers = s.restarting_containers.clone();
    }
    let mut items = attention::compute_at(&facts, disks, now);
    let mut next_since = HashMap::with_capacity(items.len());
    for item in &mut items {
        let first = s.attention_since.get(&item.id).copied().unwrap_or(now);
        item.since = first;
        next_since.insert(item.id.clone(), first);
    }
    s.attention_since = next_since;
    items
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
