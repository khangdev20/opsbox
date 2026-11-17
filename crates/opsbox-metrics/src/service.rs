//! The sampler: once a second ask `priv` for counters, keep the live ring, publish
//! to subscribers and write minute/hour rollups.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use opsbox_priv::PrivClient;
use opsbox_proto::SystemFacts;
use opsbox_store::Store;
use tokio::sync::broadcast;
use tracing::{debug, warn};

use crate::attention::{self, Attention};
use crate::engine::{Engine, Point, Snapshot};
use crate::rollup::Rollup;

/// How often `SystemFacts` is refreshed (it can run `apt-check`, which is slow).
const FACTS_EVERY: Duration = Duration::from_secs(300);
/// Every n-th sample also scans the processes.
const PROCS_EVERY: u64 = 2;

struct State {
    engine: Engine,
    rollup: Rollup,
    latest: Option<Arc<Snapshot>>,
    facts: SystemFacts,
    facts_ok: bool,
    /// Whether the last sample worked, for the "unavailable" answer.
    last_error: Option<String>,
}

#[derive(Clone)]
pub struct Metrics {
    state: Arc<Mutex<State>>,
    tx: broadcast::Sender<Arc<Snapshot>>,
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
                last_error: None,
            })),
            tx,
        };
        tokio::spawn(me.clone().run(priv_client.clone(), store));
        tokio::spawn(me.clone().refresh_facts(priv_client));
        me
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

    /// The "needs attention" items right now.
    pub fn attention(&self) -> Vec<Attention> {
        let s = self.lock();
        let disks = s
            .latest
            .as_ref()
            .map(|l| l.disks.as_slice())
            .unwrap_or_default();
        attention::compute(&s.facts, disks)
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
                let Some(snap) = s.engine.ingest(raw, now) else {
                    continue;
                };
                let rows = s.rollup.push(&snap.point, &snap.disks);
                let snap = Arc::new(snap);
                s.latest = Some(snap.clone());
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
                }
                other => debug!(?other, "could not refresh system facts"),
            }
            tokio::time::sleep(FACTS_EVERY).await;
        }
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
