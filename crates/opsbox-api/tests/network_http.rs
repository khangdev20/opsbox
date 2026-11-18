//! Real HTTP router + real priv, fixture procfs and scripted Docker/systemd.
mod common;
use axum::http::{Method, StatusCode};
use common::{Harness, Opts};
use opsbox_api::router;
use opsbox_priv::{PrivClient, PrivConfig, PrivServer};
use opsbox_proto::Role;
use serde_json::{Value, json};
use std::{
    os::unix::fs::{MetadataExt, PermissionsExt},
    sync::Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
    sync::{Mutex, oneshot},
};

async fn post(b: &mut common::Browser, path: &str, body: Value) -> common::Reply {
    b.send(Method::POST, path, Some(body), Opts::default())
        .await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn docker_and_systemd_takeover_undo_are_confirmed_revalidated_and_audited() {
    let h = Harness::new(false);
    let root = &h.root;
    std::fs::create_dir_all(root.join("proc/net")).unwrap();
    std::fs::create_dir_all(root.join("proc/123/fd")).unwrap();
    std::fs::write(
        root.join("proc/net/tcp"),
        "0: 00000000:0050 0:0 0A 0:0 0:0 0 0 0 1234 1",
    )
    .unwrap();
    std::fs::write(
        root.join("proc/net/tcp6"),
        "0: 00000000000000000000000000000000:01BB 0:0 0A 0:0 0:0 0 0 0 1235 1",
    )
    .unwrap();
    std::fs::write(root.join("proc/123/comm"), "docker-proxy\n").unwrap();
    std::os::unix::fs::symlink("socket:[1234]", root.join("proc/123/fd/3")).unwrap();
    std::os::unix::fs::symlink("socket:[1235]", root.join("proc/123/fd/4")).unwrap();
    let id = "a".repeat(64);
    std::fs::write(
        root.join("state.json"),
        json!({"running":true,"policy":"always","enabled":"enabled","id":id}).to_string(),
    )
    .unwrap();
    let fake = r#"#!/usr/bin/env python3
import json, pathlib, sys
root = pathlib.Path(__file__).parent
p = root / 'state.json'
s = json.loads(p.read_text())
a = sys.argv[1:]
with (root / 'commands').open('a') as f: f.write(json.dumps(a) + '\n')
if pathlib.Path(__file__).name == 'docker':
    if a[0] == 'ps':
        if s['running']: print(s['id'])
    elif a[0] == 'inspect':
        print(json.dumps([{'Id':s['id'],'Name':'/web','Image':'sha256:test','Config':{'Image':'nginx:1.27'},'State':{'Running':s['running']},'HostConfig':{'PortBindings':{'80/tcp':[{'HostIp':'0.0.0.0','HostPort':'80'}],'443/tcp':[{'HostIp':'0.0.0.0','HostPort':'443'}]},'RestartPolicy':{'Name':s['policy'],'MaximumRetryCount':0}},'NetworkSettings':{'Ports':{'80/tcp':[{'HostPort':'80'}],'443/tcp':[{'HostPort':'443'}]}}}]))
    elif a[0] == 'update': s['policy'] = a[1].split('=',1)[1]
    elif a[0] in ('start','stop'): s['running'] = a[0] == 'start'
    else: sys.exit(2)
else:
    if a[0] == 'show': print(s['enabled'])
    elif a[0] in ('enable','disable'): s['enabled'] = 'enabled' if a[0] == 'enable' else 'disabled'
    elif a[0] in ('start','stop'): s['running'] = a[0] == 'start'
    else: sys.exit(2)
p.write_text(json.dumps(s))
"#;
    for name in ["docker", "systemctl"] {
        let path = root.join(name);
        std::fs::write(&path, fake).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let sock = root.join("proxy.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let network = Arc::new(Mutex::new(
        json!({"running":true,"port_conflict":true,"listen_http":"0.0.0.0:8080","listen_https":"0.0.0.0:8445","primary_http":"0.0.0.0:80","primary_https":"0.0.0.0:443","conflict_ports":[80,443],"holders":[]}),
    ));
    let net = network.clone();
    let control_task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = [0; 4];
            stream.read_exact(&mut header).await.unwrap();
            let mut body = vec![0; opsbox_proto::frame::body_len(header).unwrap()];
            stream.read_exact(&mut body).await.unwrap();
            let req: Value = opsbox_proto::frame::decode(&body).unwrap();
            let reply = if req["op"]["type"] == "status" {
                json!({"type":"status","data":{"network":net.lock().await.clone()}})
            } else {
                if req["op"]["args"]["action"] == "keep_fallback" {
                    net.lock().await["port_conflict"] = json!(false);
                }
                json!({"type":"ok"})
            };
            stream
                .write_all(
                    &opsbox_proto::frame::encode(
                        &json!({"id":req["id"],"result":{"status":"ok","reply":reply}}),
                    )
                    .unwrap(),
                )
                .await
                .unwrap();
        }
    });
    let config_path = root.join("config.toml");
    std::fs::write(&config_path, format!("[api]\nlisten = '127.0.0.1:8443'\n[proxy]\nsocket = '{}'\nlisten_http = '0.0.0.0:80'\nlisten_https = '0.0.0.0:443'\n", sock.display())).unwrap();
    let mut cfg = PrivConfig::new(
        root.join("network-priv.sock"),
        vec![std::fs::metadata(root).unwrap().uid()],
    );
    cfg.config_path = config_path.clone();
    cfg.master_key_path = root.join("master.key");
    cfg.network_proc_root = root.join("proc");
    cfg.docker_bin = Some(root.join("docker"));
    cfg.network_systemctl = Some(root.join("systemctl"));
    let server = PrivServer::bind(cfg).unwrap();
    let (stop, rx) = oneshot::channel();
    let task = tokio::spawn(server.run(async {
        let _ = rx.await;
    }));
    let mut state = h.state.clone();
    state.priv_client = PrivClient::new(root.join("network-priv.sock"));
    let app = router(state);
    let mut admin = h.signed_in("boss", Role::Admin);
    admin.app = app.clone();
    let mut viewer = h.signed_in("reader", Role::Viewer);
    viewer.app = app.clone();
    assert_eq!(
        viewer.get("/api/v1/network/proxy").await.status,
        StatusCode::FORBIDDEN
    );
    // Adoption is interactive-admin only, and discovery is read-only.
    let discover_body = json!({"target":{"kind":"container","id":id}});
    for path in [
        "/api/v1/network/proxy/discover",
        "/api/v1/network/proxy/move",
        "/api/v1/network/proxy/undo-move",
        "/api/v1/domains/old.test/adopted-tls",
    ] {
        assert_eq!(
            post(&mut viewer, path, discover_body.clone()).await.status,
            StatusCode::FORBIDDEN
        );
    }
    let discovery = post(&mut admin, "/api/v1/network/proxy/discover", discover_body).await;
    assert_eq!(discovery.status, StatusCode::OK, "{}", discovery.text());
    assert_eq!(discovery.json()["plan"]["holder"], "web");
    assert_eq!(discovery.json()["plan"]["confirmation"], "recreate web");
    assert!(read_only_state(root));
    let refused=post(&mut admin,"/api/v1/network/proxy/move",json!({"target":{"kind":"container","id":id},"domains":["old.test"],"everything_else":true,"confirmation":"wrong"})).await;
    assert_eq!(refused.status, StatusCode::ACCEPTED);
    let tid = refused.json()["task"].as_str().unwrap().to_owned();
    let mut task_failed = false;
    for _ in 0..100 {
        let t = admin.get(&format!("/api/v1/tasks/{tid}")).await.json();
        if t["status"] == "failed" {
            assert!(t["error"].as_str().unwrap().contains("Type recreate web"));
            task_failed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(task_failed, "confirmation refusal task did not finish");
    assert!(read_only_state(root));
    let body = json!({"port":80,"target":{"kind":"container","id":id},"confirm":true});
    assert_eq!(
        post(&mut viewer, "/api/v1/network/proxy/takeover", body.clone())
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    let missing = post(
        &mut admin,
        "/api/v1/network/proxy/takeover",
        json!({"port":80,"target":{"kind":"container","id":id}}),
    )
    .await;
    assert_eq!(missing.status, StatusCode::CONFLICT);
    let mut bad = body.clone();
    bad["target"]["id"] = json!("b".repeat(64));
    assert_eq!(
        post(&mut admin, "/api/v1/network/proxy/takeover", bad)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    let owned = admin.get("/api/v1/network/proxy").await;
    assert_eq!(owned.status, StatusCode::OK);
    // A direct typed lookup exercises both IPv4 and IPv6 inode-to-PID mapping and Docker exact publication.
    let client = PrivClient::new(root.join("network-priv.sock"));
    let owners = client
        .call(
            "holders",
            None,
            opsbox_proto::PrivOp::PortHolders {
                ports: vec![80, 443],
            },
        )
        .await
        .unwrap();
    let opsbox_proto::PrivReply::PortHolders(owners) = owners else {
        panic!()
    };
    assert_eq!(owners.len(), 2);
    assert_eq!(owners[0].container.as_ref().unwrap().name, "web");
    assert_eq!(owners[1].port, 443);
    let taken = post(&mut admin, "/api/v1/network/proxy/takeover", body).await;
    assert_eq!(taken.status, StatusCode::OK, "{}", taken.text());
    let receipt = taken.json()["receipt"].clone();
    assert_eq!(receipt["previous_policy"], "always");
    let read_state = || {
        serde_json::from_slice::<Value>(&std::fs::read(root.join("state.json")).unwrap()).unwrap()
    };
    assert_eq!(read_state()["running"], false);
    assert_eq!(read_state()["policy"], "no");
    assert_eq!(
        admin.get("/api/v1/network/proxy").await.json()["undo"][0]["id"],
        receipt["id"]
    );
    let undone = post(
        &mut admin,
        "/api/v1/network/proxy/undo",
        json!({"id":receipt["id"],"confirm":true}),
    )
    .await;
    assert_eq!(undone.status, StatusCode::OK, "{}", undone.text());
    assert_eq!(read_state()["running"], true);
    assert_eq!(read_state()["policy"], "always");
    assert_eq!(
        post(
            &mut admin,
            "/api/v1/network/proxy/undo",
            json!({"id":receipt["id"],"confirm":true})
        )
        .await
        .status,
        StatusCode::BAD_REQUEST
    );
    // The same endpoint can only stop the actual cgroup service, never a caller-chosen unit.
    std::fs::write(root.join("proc/123/comm"), "nginx\n").unwrap();
    std::fs::write(
        root.join("proc/123/cgroup"),
        "0::/system.slice/nginx.service",
    )
    .unwrap();
    let taken = post(
        &mut admin,
        "/api/v1/network/proxy/takeover",
        json!({"port":80,"target":{"kind":"unit","id":"nginx.service"},"confirm":true}),
    )
    .await;
    assert_eq!(taken.status, StatusCode::OK, "{}", taken.text());
    assert_eq!(read_state()["enabled"], "disabled");
    assert_eq!(read_state()["running"], false);
    let undone = post(
        &mut admin,
        "/api/v1/network/proxy/undo",
        json!({"id":taken.json()["receipt"]["id"],"confirm":true}),
    )
    .await;
    assert_eq!(undone.status, StatusCode::OK);
    assert_eq!(read_state()["enabled"], "enabled");
    assert_eq!(read_state()["running"], true);
    assert_eq!(
        post(&mut admin, "/api/v1/network/proxy/keep", json!({}))
            .await
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        post(
            &mut admin,
            "/api/v1/network/proxy/keep",
            json!({"confirm":true})
        )
        .await
        .status,
        StatusCode::OK
    );
    assert!(!network.lock().await["port_conflict"].as_bool().unwrap());
    let saved = std::fs::read_to_string(config_path).unwrap();
    assert!(saved.contains("listen_http = \"0.0.0.0:8080\""));
    assert!(saved.contains("127.0.0.1:8443"));
    let audit = h.store.recent_audit(50).unwrap();
    assert!(
        audit
            .iter()
            .any(|a| a.action == "network.proxy_takeover" && a.detail.contains("always"))
    );
    assert!(audit.iter().any(|a| a.action == "network.proxy_undo"));
    assert!(h.store.verify_audit().is_ok());
    let commands = std::fs::read_to_string(root.join("commands")).unwrap();
    assert!(!commands.contains("remove") && !commands.contains(" rm"));
    stop.send(()).unwrap();
    task.await.unwrap();
    control_task.abort();
    h.shutdown().await;
}

fn read_only_state(root: &std::path::Path) -> bool {
    let s: Value =
        serde_json::from_slice(&std::fs::read(root.join("state.json")).unwrap()).unwrap();
    s["running"] == true && s["policy"] == "always" && !root.join("web-moves").exists()
}
