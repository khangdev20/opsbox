//! Real sockets: busy primary and fallback, HTTP forwarding, status, and live promotion.
use opsbox_priv::{PrivConfig, PrivServer};
use opsbox_proxy::{ProxyClient, ProxyConfig, ProxyServer, Route, UpstreamTls};
use std::{os::unix::fs::MetadataExt, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};

async fn get(addr: std::net::SocketAddr) -> String {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: app.example.com\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).await.unwrap();
    raw
}
#[tokio::test]
async fn busy_ports_serve_and_promote_without_dropping_inflight_requests() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let primary = occupied.local_addr().unwrap();
    let occupied_tls = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let primary_tls = occupied_tls.local_addr().unwrap();
    let occupied_fallback = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proc = root.join("proc");
    std::fs::create_dir_all(proc.join("net")).unwrap();
    std::fs::create_dir_all(proc.join("123/fd")).unwrap();
    std::fs::write(
        proc.join("net/tcp"),
        format!(
            "0: 0100007F:{:04X} 0:0 0A 0:0 0:0 0 0 0 1234 1",
            primary.port()
        ),
    )
    .unwrap();
    std::fs::write(proc.join("123/comm"), "nginx\n").unwrap();
    std::fs::write(proc.join("123/cgroup"), "0::/system.slice/nginx.service").unwrap();
    std::os::unix::fs::symlink("socket:[1234]", proc.join("123/fd/3")).unwrap();
    let mut priv_cfg = PrivConfig::new(
        root.join("priv.sock"),
        vec![std::fs::metadata(root).unwrap().uid()],
    );
    priv_cfg.network_proc_root = proc;
    let server = PrivServer::bind(priv_cfg).unwrap();
    let (priv_stop, rx) = oneshot::channel();
    let priv_task = tokio::spawn(server.run(async {
        let _ = rx.await;
    }));
    let mut config = ProxyConfig::new(root.join("priv.sock"));
    config.listen_http = primary;
    config.listen_https = primary_tls;
    config.fallback_http = occupied_fallback.local_addr().unwrap().port();
    config.socket_path = root.join("proxy.sock");
    config.state_dir = root.join("state");
    config.retry_interval = Duration::from_millis(100);
    let server = ProxyServer::bind(config).await.unwrap();
    let fallback = server.http_addr();
    assert_ne!(fallback, primary);
    assert!((8080..=8099).contains(&fallback.port()));
    assert_ne!(server.https_addr(), primary_tls);
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    server.routes().set(vec![Route {
        host: "app.example.com".into(),
        upstream: upstream.local_addr().unwrap().to_string(),
        upstream_tls: UpstreamTls::Plain,
        redirect_www: false,
        redirect_to: None,
        redirect_https: false,
        hsts: false,
        max_body_bytes: 0,
        is_panel: false,
        forward_secret: None,
    }]);
    let (request_seen, seen) = oneshot::channel();
    let (finish, finish_rx) = oneshot::channel();
    let upstream_task = tokio::spawn(async move {
        let (mut stream, _) = upstream.accept().await.unwrap();
        let mut buf = [0; 4096];
        assert!(stream.read(&mut buf).await.unwrap() > 0);
        request_seen.send(()).unwrap();
        let _ = finish_rx.await;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
        let (mut stream, _) = upstream.accept().await.unwrap();
        assert!(stream.read(&mut buf).await.unwrap() > 0);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    });
    let (stop, rx) = oneshot::channel();
    let task = tokio::spawn(server.run(async {
        let _ = rx.await;
    }));
    let client = ProxyClient::new(root.join("proxy.sock"));
    let status = client.status("conflict").await.unwrap();
    assert!(status.network.running && status.network.port_conflict);
    assert_eq!(status.network.holders[0].process, "nginx");
    assert_eq!(status.network.holders[0].pid, 123);
    let in_flight = tokio::spawn(get(fallback));
    let _ = seen.await;
    drop(occupied);
    drop(occupied_tls);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = client.status("promoted").await.unwrap();
            if !status.network.port_conflict {
                assert_eq!(status.network.listen_http, Some(primary));
                assert_eq!(status.network.listen_https, Some(primary_tls));
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    finish.send(()).unwrap();
    assert!(in_flight.await.unwrap().ends_with("ok"));
    assert!(get(primary).await.ends_with("ok"));
    assert!(TcpStream::connect(fallback).await.is_err());
    upstream_task.await.unwrap();
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    priv_stop.send(()).unwrap();
    priv_task.await.unwrap();
}

/// No host listeners: exercise the same retry path with a fake port table.
#[tokio::test(start_paused = true)]
async fn fake_port_table_reports_stable_ports_busy_until_slow_retry() {
    use opsbox_proto::{PortHolder, ProxyNetwork};
    use opsbox_proxy::server::{
        bind_or_fallback_with, busy_retry_interval, port_state, retry_busy_ports,
    };
    use std::{collections::BTreeMap, io, net::SocketAddr};
    for port in [80, 443] {
        let config = ProxyConfig::new("unused");
        assert_eq!(config.retry_interval, Duration::from_secs(30));
        let primary: SocketAddr = format!("0.0.0.0:{port}").parse().unwrap();
        let fallback: SocketAddr = format!("0.0.0.0:{}", if port == 80 { 8080 } else { 8445 })
            .parse()
            .unwrap();
        let holder = PortHolder {
            port,
            pid: 321,
            process: "caddy".into(),
            unit: None,
            container: None,
        };
        let mut table = BTreeMap::from([(primary, holder.clone())]);
        let (listener, conflict) = bind_or_fallback_with(
            primary,
            fallback.port(),
            fallback.port()..=fallback.port(),
            |addr| {
                std::future::ready(if table.contains_key(&addr) {
                    Err(io::Error::from(io::ErrorKind::AddrInUse))
                } else {
                    Ok(addr)
                })
            },
        )
        .await
        .expect("busy startup must stay alive on a fallback");
        assert!(conflict);
        assert_eq!(listener, fallback);
        let mut network = ProxyNetwork {
            running: true,
            port_conflict: true,
            primary_http: Some(config.listen_http),
            primary_https: Some(config.listen_https),
            listen_http: Some(if port == 80 {
                fallback
            } else {
                config.listen_http
            }),
            listen_https: Some(if port == 443 {
                fallback
            } else {
                config.listen_https
            }),
            conflict_ports: vec![port],
            holders: vec![holder],
        };
        let busy = network.clone();
        let mut retry = busy_retry_interval(config.retry_interval);
        let before = tokio::time::Instant::now();
        for _ in 0..2 {
            assert_eq!(port_state(&network), "ports_busy");
            assert_eq!(network.holders[0].process, "caddy");
            assert_eq!(network.holders[0].pid, 321);
            tokio::select! {
                biased;
                _ = retry.tick() => panic!("retried before 30 seconds"),
                _ = tokio::time::sleep(Duration::from_secs(29)) => {}
            }
            retry.tick().await;
            let mut attempts = vec![];
            let bound = retry_busy_ports(&mut network, &config, |addr| {
                attempts.push(addr);
                std::future::ready(if table.contains_key(&addr) {
                    Err(io::Error::from(io::ErrorKind::AddrInUse))
                } else {
                    Ok(addr)
                })
            })
            .await
            .unwrap();
            assert_eq!(network, busy);
            assert!(bound.is_empty());
            assert_eq!(attempts, vec![primary]);
            assert!(network.running);
            assert_eq!(network.conflict_ports, vec![port]);
        }
        assert_eq!(before.elapsed(), Duration::from_secs(60));
        table.clear();
        retry.tick().await;
        let bound = retry_busy_ports(&mut network, &config, |addr| std::future::ready(Ok(addr)))
            .await
            .unwrap();
        assert_eq!(bound, vec![(port == 443, primary)]);
        assert_eq!(port_state(&network), "running");
        assert!(!network.port_conflict);
        assert!(network.holders.is_empty());
        // An injected interval uses the identical path without a wall-clock wait.
        let mut fast = busy_retry_interval(Duration::from_millis(10));
        let before = tokio::time::Instant::now();
        fast.tick().await;
        assert_eq!(before.elapsed(), Duration::from_millis(10));
    }
}
