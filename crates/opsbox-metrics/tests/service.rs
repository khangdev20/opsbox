//! The sampler against a real privileged helper (Linux reads /proc; other systems use
//! the sysinfo fallback).

use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::time::Duration;

use opsbox_metrics::Metrics;
use opsbox_priv::{PrivClient, PrivConfig, PrivServer};
use opsbox_store::Store;
use tokio::sync::oneshot;

#[tokio::test]
async fn samples_every_second_publishes_and_keeps_a_window() {
    let dir = tempfile::tempdir().unwrap();
    let probe = dir.path().join("probe");
    std::fs::write(&probe, b"").unwrap();
    let uid = std::fs::metadata(&probe).unwrap().uid();
    let socket = dir.path().join("p.sock");
    let mut cfg = PrivConfig::new(&socket, vec![uid]);
    cfg.master_key_path = dir.path().join("k");
    let server = PrivServer::bind(cfg).unwrap();
    let (stop, rx) = oneshot::channel::<()>();
    tokio::spawn(server.run(async {
        let _ = rx.await;
    }));

    let store = Arc::new(Store::open_in_memory().unwrap());
    let metrics = Metrics::spawn(PrivClient::new(&socket), store);
    let mut sub = metrics.subscribe();

    // The first sample only primes the counters, so the first snapshot is ~2 s away.
    let first = tokio::time::timeout(Duration::from_secs(6), sub.recv())
        .await
        .expect("a snapshot within 6 s")
        .unwrap();
    assert!(first.point.mem_total > 0 && first.point.mem_used <= first.point.mem_total);
    assert!(
        (0.0..=100.0).contains(&first.point.cpu),
        "{}",
        first.point.cpu
    );
    assert!(!first.point.cores.is_empty());
    assert!(first.disks.iter().any(|d| d.total > 0));

    let second = tokio::time::timeout(Duration::from_secs(4), sub.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(second.point.ts >= first.point.ts);
    assert!(metrics.latest().is_some());
    assert!(metrics.window(10).len() >= 2);
    assert!(metrics.last_error().is_none());
    let _ = stop.send(());
}

#[tokio::test]
async fn without_a_helper_there_is_no_data_and_a_reason() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let metrics = Metrics::spawn(PrivClient::new("/nonexistent/priv.sock"), store);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(metrics.latest().is_none());
    assert!(metrics.window(10).is_empty());
    let why = metrics.last_error().expect("a reason");
    assert!(why.contains("cannot reach the privileged helper"), "{why}");
    assert!(
        metrics.attention().is_empty(),
        "no facts and no disks: nothing to report"
    );
}
