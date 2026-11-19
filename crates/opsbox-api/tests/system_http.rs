//! The System endpoints over HTTP.
//!
//! These tests never change the machine they run on (CI or a developer's laptop, maybe as
//! root): they cover who may call what, input validation, the refusal list and how errors
//! reach the page, all of which is decided before any tool is started.

mod common;

use axum::http::{Method, StatusCode};
use common::{Harness, Opts};
use opsbox_proto::Role;
use serde_json::{Value, json};

fn code(r: &common::Reply) -> String {
    r.error_code()
}

#[tokio::test]
async fn reading_needs_a_session_and_every_change_needs_an_admin() {
    let h = Harness::new(true);
    let mut anon = h.browser();
    for path in [
        "/api/v1/system/updates",
        "/api/v1/system/units",
        "/api/v1/system/journal",
        "/api/v1/jobs",
        "/api/v1/firewall",
        "/api/v1/system/disk",
        "/api/v1/tasks/x",
    ] {
        assert_eq!(
            anon.get(path).await.status,
            StatusCode::UNAUTHORIZED,
            "{path}"
        );
    }
    // (method, path, body) of every mutating endpoint.
    let changes: Vec<(Method, &str, Option<Value>)> = vec![
        (
            Method::POST,
            "/api/v1/system/updates/install",
            Some(json!({"which": "security"})),
        ),
        (
            Method::PUT,
            "/api/v1/system/updates/auto",
            Some(json!({"on": true})),
        ),
        (Method::POST, "/api/v1/system/reboot", None),
        (Method::POST, "/api/v1/system/reboot/cancel", None),
        (
            Method::POST,
            "/api/v1/system/units/nginx.service/restart",
            None,
        ),
        (
            Method::POST,
            "/api/v1/jobs",
            Some(json!({"name": "x", "cron": "0 2 * * *", "command": "true"})),
        ),
        (Method::POST, "/api/v1/jobs/0a1b2c3d/run", None),
        (Method::DELETE, "/api/v1/jobs/0a1b2c3d", None),
        (
            Method::POST,
            "/api/v1/firewall/rules",
            Some(json!({"port": 80})),
        ),
        (
            Method::DELETE,
            "/api/v1/firewall/rules/tcp:80:anywhere",
            None,
        ),
        (Method::POST, "/api/v1/firewall/confirm", None),
        (Method::POST, "/api/v1/ssh/emergency", None),
        (
            Method::PUT,
            "/api/v1/system/swap",
            Some(json!({"sizeMb": 1024})),
        ),
        (
            Method::POST,
            "/api/v1/system/disk/grow",
            Some(json!({"mount": "/", "confirm": "grow", "password": "x"})),
        ),
        (
            Method::PUT,
            "/api/v1/system/hostname",
            Some(json!({"name": "box"})),
        ),
        (
            Method::PUT,
            "/api/v1/system/timezone",
            Some(json!({"zone": "UTC"})),
        ),
    ];
    for role in [Role::Viewer, Role::Deployer] {
        let mut b = h.signed_in(&format!("{}user", role.as_str()), role);
        for (m, path, body) in &changes {
            let r = b.send(m.clone(), path, body.clone(), Opts::default()).await;
            assert_eq!(
                (r.status, code(&r)),
                (StatusCode::FORBIDDEN, "forbidden".to_owned()),
                "{role:?} {m} {path}"
            );
        }
    }
    // Without the CSRF header even an admin is refused.
    let mut admin = h.signed_in("theadmin", Role::Admin);
    for (m, path, body) in &changes {
        let r = admin
            .send(m.clone(), path, body.clone(), Opts::bare())
            .await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "no CSRF: {m} {path}");
    }
    h.shutdown().await;
}

#[tokio::test]
async fn bad_input_is_a_validation_error_in_words_before_anything_runs() {
    let h = Harness::new(true);
    let mut a = h.signed_in("theadmin", Role::Admin);

    let cases: Vec<(Method, &str, Option<Value>, &str)> = vec![
        (
            Method::POST,
            "/api/v1/system/updates/install",
            Some(json!({"which": "everything"})),
            "Choose security or all",
        ),
        (
            Method::POST,
            "/api/v1/system/reboot",
            Some(json!({"delayMin": 0})),
            "between 1 and 60",
        ),
        (
            Method::POST,
            "/api/v1/system/reboot",
            Some(json!({"delayMin": 61})),
            "between 1 and 60",
        ),
        (
            Method::POST,
            "/api/v1/system/units/nginx.service/explode",
            None,
            "must be start, stop",
        ),
        (
            Method::POST,
            "/api/v1/system/units/..%2F..%2Fetc%2Fpasswd/start",
            None,
            "not a service name",
        ),
        (
            Method::POST,
            "/api/v1/system/units/ssh/start",
            None,
            "not a service name",
        ),
        (
            Method::POST,
            "/api/v1/jobs",
            Some(json!({"name": "x", "cron": "0 2 * *", "command": "true"})),
            "5 fields",
        ),
        (
            Method::POST,
            "/api/v1/jobs",
            Some(json!({"name": "x", "cron": "0 2 1 * 1", "command": "true"})),
            "either the day of the month",
        ),
        (
            Method::POST,
            "/api/v1/jobs",
            Some(json!({"name": "", "cron": "0 2 * * *", "command": "true"})),
            "Describe what the job does",
        ),
        (
            Method::POST,
            "/api/v1/jobs",
            Some(json!({"name": "x", "cron": "0 2 * * *", "command": "a\nb"})),
            "single line",
        ),
        (
            Method::POST,
            "/api/v1/jobs",
            Some(json!({"name": "x", "cron": "0 2 * * *", "command": ""})),
            "Enter the command",
        ),
        (Method::POST, "/api/v1/jobs/NOPE/run", None, "not a job id"),
        (Method::DELETE, "/api/v1/jobs/..", None, ""),
        (
            Method::POST,
            "/api/v1/firewall/rules",
            Some(json!({"port": 0})),
            "between 1 and 65535",
        ),
        (
            Method::POST,
            "/api/v1/firewall/rules",
            Some(json!({"port": "http"})),
            "between 1 and 65535",
        ),
        (
            Method::POST,
            "/api/v1/firewall/rules",
            Some(json!({"port": 80, "proto": "icmp"})),
            "tcp or udp",
        ),
        (
            Method::POST,
            "/api/v1/firewall/rules",
            Some(json!({"port": 80, "src": "everyone"})),
            "anywhere",
        ),
        (
            Method::POST,
            "/api/v1/firewall/rules",
            Some(json!({"port": 80, "src": "10.0.0.0/33"})),
            "anywhere",
        ),
        (
            Method::POST,
            "/api/v1/firewall/rules",
            Some(json!({"port": 80, "note": "x".repeat(101)})),
            "one line",
        ),
        (
            Method::DELETE,
            "/api/v1/firewall/rules/garbage",
            None,
            "not a rule id",
        ),
        (
            Method::POST,
            "/api/v1/ssh/emergency",
            Some(json!({"ip": "nope"})),
            "not an IP address",
        ),
        (
            Method::PUT,
            "/api/v1/system/hostname",
            Some(json!({"name": "Bad Name"})),
            "lower-case letters",
        ),
        (
            Method::PUT,
            "/api/v1/system/timezone",
            Some(json!({"zone": "../../etc/passwd"})),
            "not a time zone",
        ),
    ];
    for (m, path, body, expect) in cases {
        let r = a.send(m.clone(), path, body, Opts::default()).await;
        if expect.is_empty() {
            assert!(r.status.is_client_error(), "{m} {path} -> {}", r.status);
            continue;
        }
        assert_eq!(
            (r.status, code(&r)),
            (StatusCode::BAD_REQUEST, "validation".to_owned()),
            "{m} {path}: {}",
            r.text()
        );
        let msg = r.json()["error"]["message"].as_str().unwrap().to_owned();
        assert!(msg.contains(expect), "{m} {path}: {msg}");
    }
    h.shutdown().await;
}

#[tokio::test]
async fn the_refusal_list_keeps_opsbox_ssh_and_the_network_up() {
    let h = Harness::new(true);
    let mut a = h.signed_in("theadmin", Role::Admin);
    for (unit, action) in [
        ("opsbox.service", "stop"),
        ("opsbox-priv.service", "restart"),
        ("opsbox.service", "disable"),
        ("ssh.service", "stop"),
        ("sshd.service", "disable"),
        ("systemd-networkd.service", "stop"),
        ("NetworkManager.service", "restart"),
        ("dbus.service", "stop"),
    ] {
        let r = a
            .send(
                Method::POST,
                &format!("/api/v1/system/units/{unit}/{action}"),
                None,
                Opts::default(),
            )
            .await;
        assert_eq!(
            (r.status, code(&r)),
            (StatusCode::FORBIDDEN, "forbidden".to_owned()),
            "{unit} {action}: {}",
            r.text()
        );
        let msg = r.json()["error"]["message"].as_str().unwrap().to_owned();
        assert!(
            msg.ends_with('.') && msg.len() > 30,
            "a sentence for people: {msg}"
        );
    }
    h.shutdown().await;
}

#[tokio::test]
async fn swap_runs_as_a_task_and_its_validation_arrives_as_a_failed_task() {
    let h = Harness::new(true);
    let mut a = h.signed_in("theadmin", Role::Admin);
    let r = a
        .send(
            Method::PUT,
            "/api/v1/system/swap",
            Some(json!({"sizeMb": 64})),
            Opts::default(),
        )
        .await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text());
    let id = r.json()["task"].as_str().unwrap().to_owned();
    let mut task = Value::Null;
    for _ in 0..100 {
        task = a.get(&format!("/api/v1/tasks/{id}")).await.json();
        if task["status"] != "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(task["status"], "failed", "{task}");
    assert!(
        task["error"].as_str().unwrap().contains("not useful"),
        "{task}"
    );
    assert!(task["finished"].is_number() && task["kind"] == "system.swap");
    assert_eq!(
        a.get("/api/v1/tasks/does-not-exist").await.status,
        StatusCode::NOT_FOUND
    );
    h.shutdown().await;
}

#[tokio::test]
async fn machines_without_the_tools_say_so_instead_of_failing_mysteriously() {
    // Whatever this machine has, an answer is either data or a clear "unsupported" in the
    // spec's error format, never a bare 500.
    let h = Harness::new(true);
    let mut v = h.signed_in("viewer1", Role::Viewer);
    for path in [
        "/api/v1/system/units",
        "/api/v1/firewall",
        "/api/v1/system/updates",
    ] {
        let r = v.get(path).await;
        match r.status.as_u16() {
            200 => {}
            501 => {
                assert_eq!(code(&r), "unsupported", "{path}");
                assert!(r.json()["error"]["message"].as_str().unwrap().len() > 20);
            }
            other => assert!(
                r.json()["error"]["message"].is_string(),
                "{path}: {other} {}",
                r.text()
            ),
        }
    }
    h.shutdown().await;
}

#[tokio::test]
async fn journal_arguments_are_checked() {
    let h = Harness::new(true);
    let mut v = h.signed_in("viewer1", Role::Viewer);
    for q in ["prio=9", "unit=..%2Fx", "unit=a%20b"] {
        let r = v.get(&format!("/api/v1/system/journal?{q}")).await;
        assert_eq!(
            (r.status, code(&r)),
            (StatusCode::BAD_REQUEST, "validation".to_owned()),
            "{q}: {}",
            r.text()
        );
    }
    // The live tail needs a session and a matching Origin, like the metrics stream.
    let mut anon = h.browser();
    assert_eq!(
        anon.get("/api/v1/stream/journal").await.status,
        StatusCode::UNAUTHORIZED
    );
    h.shutdown().await;
}

#[tokio::test]
async fn growing_the_disk_asks_for_the_password_before_looking_at_anything() {
    let h = Harness::new(true);
    let mut a = h.signed_in("theadmin", Role::Admin);
    let r = a
        .post(
            "/api/v1/system/disk/grow",
            json!({"mount": "/", "confirm": "grow", "password": "not the password"}),
        )
        .await;
    assert_eq!(
        (r.status, code(&r)),
        (StatusCode::FORBIDDEN, "forbidden".to_owned())
    );
    assert!(
        r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("password")
    );
    // The refusal never started a task.
    assert!(r.json().get("task").is_none());
    // A missing field is a validation error, not a crash.
    let r = a
        .post("/api/v1/system/disk/grow", json!({"mount": "/"}))
        .await;
    assert!(r.status.is_client_error(), "{}", r.text());
    h.shutdown().await;
}

#[tokio::test]
async fn the_disk_view_is_data_or_a_clear_unsupported() {
    let h = Harness::new(true);
    let mut v = h.signed_in("viewer1", Role::Viewer);
    let r = v.get("/api/v1/system/disk").await;
    match r.status.as_u16() {
        200 => {
            let j = r.json();
            assert!(
                j["volumes"].is_array() && j["growpart_installed"].is_boolean(),
                "{j}"
            );
            for vol in j["volumes"].as_array().unwrap() {
                assert!(
                    vol["mount"].is_string() && vol["growable_bytes"].is_number(),
                    "{vol}"
                );
            }
        }
        501 => assert_eq!(code(&r), "unsupported"),
        _ => assert!(r.json()["error"]["message"].is_string(), "{}", r.text()),
    }
    h.shutdown().await;
}

#[tokio::test]
async fn panel_firewall_removal_forwards_actual_listener_confirmation_and_revert() {
    use opsbox_proto::{
        FirewallChange, PrivErrorCode, PrivOp, PrivReply, PrivRequest, PrivResponse, frame,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("firewall.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let h = Harness::new_with(false, |mut state| {
        state.priv_client = opsbox_priv::PrivClient::new(&socket);
        state.with_listen("127.0.0.1:10443".parse().unwrap())
    });
    // The real helper's guard/undo is tested in opsbox-priv; this fixture checks the
    // HTTP boundary forwards the running listener and preserves both responses.
    let helper = tokio::spawn(async move {
        for confirmed in [false, true] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = [0; 4];
            stream.read_exact(&mut header).await.unwrap();
            let mut body = vec![0; frame::body_len(header).unwrap()];
            stream.read_exact(&mut body).await.unwrap();
            let request: PrivRequest = frame::decode(&body).unwrap();
            match request.op {
                PrivOp::FirewallRemove {
                    panel_port,
                    id,
                    confirm,
                    ..
                } => {
                    assert_eq!(panel_port, Some(10443));
                    assert_eq!(id, "tcp:10443:anywhere");
                    assert_eq!(confirm, confirmed);
                }
                other => panic!("unexpected operation: {other:?}"),
            }
            let response = if confirmed {
                PrivResponse::success(
                    request.id,
                    PrivReply::FirewallChange(FirewallChange {
                        revert_at: Some(123456),
                    }),
                )
            } else {
                PrivResponse::failure(
                    request.id,
                    PrivErrorCode::ConfirmRequired,
                    "Panel access would be lost",
                )
            };
            stream
                .write_all(&frame::encode(&response).unwrap())
                .await
                .unwrap();
        }
    });
    let mut admin = h.signed_in("firewall-admin", Role::Admin);
    let denied = admin
        .send(
            Method::DELETE,
            "/api/v1/firewall/rules/tcp:10443:anywhere",
            None,
            Opts::default(),
        )
        .await;
    assert_eq!(denied.status, StatusCode::CONFLICT);
    assert_eq!(denied.error_code(), "confirmation_required");
    let allowed = admin
        .send(
            Method::DELETE,
            "/api/v1/firewall/rules/tcp:10443:anywhere?confirm=true",
            None,
            Opts::default(),
        )
        .await;
    assert_eq!(allowed.status, StatusCode::OK);
    assert_eq!(allowed.json()["revertAt"], 123456);
    helper.await.unwrap();
    h.shutdown().await;
}
