//! End-to-end tests of the privileged socket: a real server on a real Unix socket.

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use opsbox_priv::{ClientError, PrivClient, PrivConfig, PrivServer, ServerError};
use opsbox_proto::frame;
use opsbox_proto::{PrivErrorCode, PrivReply, PrivResponse, PrivResult};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::oneshot;

/// The user id this test process runs as, found without unsafe code: whoever owns
/// a file we just created.
fn own_uid(dir: &Path) -> u32 {
    let probe = dir.join("uid-probe");
    std::fs::write(&probe, b"").unwrap();
    std::fs::metadata(&probe).unwrap().uid()
}

struct Running {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    stop: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl Running {
    fn client(&self) -> PrivClient {
        PrivClient::new(&self.socket).with_timeout(Duration::from_secs(5))
    }

    async fn stop(mut self) {
        self.stop.take().unwrap().send(()).unwrap();
        self.task.await.unwrap();
    }
}

/// Start a server that allows `allowed(own_uid)`.
fn start(allowed: impl FnOnce(u32) -> Vec<u32>) -> Running {
    start_with_key(allowed, None)
}

fn start_with_key(allowed: impl FnOnce(u32) -> Vec<u32>, key: Option<PathBuf>) -> Running {
    let dir = tempfile::tempdir().unwrap();
    let uid = own_uid(dir.path());
    let socket = dir.path().join("run").join("priv.sock");
    let mut cfg = PrivConfig::new(&socket, allowed(uid));
    cfg.master_key_path = key.unwrap_or_else(|| dir.path().join("master.key"));
    let server = PrivServer::bind(cfg).unwrap();
    let (stop, rx) = oneshot::channel();
    let task = tokio::spawn(server.run(async {
        let _ = rx.await;
    }));
    Running {
        _dir: dir,
        socket,
        stop: Some(stop),
        task,
    }
}

async fn raw_exchange(socket: &Path, payload: &[u8]) -> Option<PrivResponse> {
    let mut s = UnixStream::connect(socket).await.unwrap();
    s.write_all(&(payload.len() as u32).to_be_bytes())
        .await
        .unwrap();
    s.write_all(payload).await.unwrap();
    read_response(&mut s).await
}

async fn read_response(s: &mut UnixStream) -> Option<PrivResponse> {
    let mut header = [0u8; 4];
    s.read_exact(&mut header).await.ok()?;
    let mut body = vec![0u8; frame::body_len(header).ok()?];
    s.read_exact(&mut body).await.ok()?;
    Some(frame::decode(&body).unwrap())
}

fn failure_code(r: &PrivResponse) -> PrivErrorCode {
    match &r.result {
        PrivResult::Failure { error } => error.code,
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[tokio::test]
async fn ping_and_host_info_round_trip() {
    let srv = start(|uid| vec![uid]);
    let client = srv.client();

    let version = client.ping("req-ping").await.unwrap();
    assert_eq!(version, opsbox_priv::VERSION);

    let info = client.host_info("req-host", Some("tester")).await.unwrap();
    assert!(!info.hostname.is_empty());
    assert!(info.cpu_count >= 1);
    assert!(info.mem_total_bytes > 0);

    srv.stop().await;
}

#[tokio::test]
async fn several_requests_share_one_connection() {
    let srv = start(|uid| vec![uid]);
    let mut s = UnixStream::connect(&srv.socket).await.unwrap();
    for i in 0..3 {
        let req = serde_json::json!({"id": format!("r{i}"), "op": {"type": "ping"}});
        let bytes = serde_json::to_vec(&req).unwrap();
        s.write_all(&(bytes.len() as u32).to_be_bytes())
            .await
            .unwrap();
        s.write_all(&bytes).await.unwrap();
        let resp = read_response(&mut s).await.unwrap();
        assert_eq!(resp.id, format!("r{i}"));
        assert!(matches!(
            resp.result,
            PrivResult::Success {
                reply: PrivReply::Pong { .. }
            }
        ));
    }
    srv.stop().await;
}

#[tokio::test]
async fn peer_with_a_user_id_outside_the_allow_list_is_rejected() {
    // Allow some other user id; this process is not it.
    let srv = start(|uid| vec![uid.wrapping_add(1)]);

    // Through the client: the answer is a Forbidden error, never a reply.
    let err = srv.client().ping("req-1").await.unwrap_err();
    match err {
        ClientError::Remote(e) => assert_eq!(e.code, PrivErrorCode::Forbidden),
        other => panic!("expected Forbidden, got {other:?}"),
    }
    let err = srv.client().host_info("req-2", None).await.unwrap_err();
    assert!(matches!(err, ClientError::Remote(e) if e.code == PrivErrorCode::Forbidden));

    // Even a perfectly valid request written by hand is not executed.
    let valid =
        serde_json::to_vec(&serde_json::json!({"id": "x", "op": {"type": "ping"}})).unwrap();
    let resp = raw_exchange(&srv.socket, &valid).await.unwrap();
    assert_eq!(failure_code(&resp), PrivErrorCode::Forbidden);

    srv.stop().await;
}

#[tokio::test]
async fn an_empty_allow_list_admits_nobody() {
    let srv = start(|_| vec![]);
    let err = srv.client().ping("r").await.unwrap_err();
    assert!(matches!(err, ClientError::Remote(e) if e.code == PrivErrorCode::Forbidden));
    srv.stop().await;
}

#[tokio::test]
async fn anything_outside_privop_is_rejected_with_the_request_id_echoed() {
    let srv = start(|uid| vec![uid]);
    for payload in [
        r#"{"id":"abc","op":{"type":"run_shell","args":{"cmd":"id"}}}"#,
        r#"{"id":"abc","op":{"type":"Ping"}}"#,
        r#"{"id":"abc","op":"ping","extra":1}"#,
        r#"{"id":"abc"}"#,
    ] {
        let resp = raw_exchange(&srv.socket, payload.as_bytes()).await.unwrap();
        assert_eq!(failure_code(&resp), PrivErrorCode::BadRequest, "{payload}");
        assert_eq!(resp.id, "abc", "{payload}");
    }
    srv.stop().await;
}

#[tokio::test]
async fn garbage_gets_a_bad_request_not_a_crash() {
    let srv = start(|uid| vec![uid]);
    let resp = raw_exchange(&srv.socket, b"\xff\xfe not json")
        .await
        .unwrap();
    assert_eq!(failure_code(&resp), PrivErrorCode::BadRequest);
    assert_eq!(resp.id, "");
    // The server is still fine afterwards.
    srv.client().ping("after").await.unwrap();
    srv.stop().await;
}

#[tokio::test]
async fn oversized_frame_is_refused_and_the_connection_closed() {
    let srv = start(|uid| vec![uid]);
    let mut s = UnixStream::connect(&srv.socket).await.unwrap();
    s.write_all(&u32::MAX.to_be_bytes()).await.unwrap();
    let resp = read_response(&mut s).await.unwrap();
    assert_eq!(failure_code(&resp), PrivErrorCode::BadRequest);
    let mut rest = Vec::new();
    s.read_to_end(&mut rest).await.unwrap();
    assert!(rest.is_empty(), "connection should be closed");
    srv.client().ping("after").await.unwrap();
    srv.stop().await;
}

#[tokio::test]
async fn socket_gets_the_configured_mode_and_is_removed_on_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("s.sock");
    let uid = own_uid(dir.path());
    let mut cfg = PrivConfig::new(&socket, vec![uid]);
    cfg.socket_mode = 0o600;
    let server = PrivServer::bind(cfg).unwrap();
    assert_eq!(
        std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let (tx, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(server.run(async {
        let _ = rx.await;
    }));
    tx.send(()).unwrap();
    task.await.unwrap();
    assert!(!socket.exists());
}

#[tokio::test]
async fn bind_replaces_a_stale_socket_but_never_a_live_one_or_a_regular_file() {
    let dir = tempfile::tempdir().unwrap();
    let uid = own_uid(dir.path());
    let socket = dir.path().join("s.sock");

    // A dead socket file, as left behind by a crash: bind a listener, forget the file.
    drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
    assert!(socket.exists());
    let live =
        PrivServer::bind(PrivConfig::new(&socket, vec![uid])).expect("stale socket is replaced");

    // While that one is listening, a second bind must refuse.
    let (tx, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(live.run(async {
        let _ = rx.await;
    }));
    let second = PrivServer::bind(PrivConfig::new(&socket, vec![uid]));
    assert!(matches!(second, Err(ServerError::AlreadyRunning(_))));
    tx.send(()).unwrap();
    task.await.unwrap();

    // A regular file at the path is never deleted.
    let file = dir.path().join("precious");
    std::fs::write(&file, b"data").unwrap();
    let third = PrivServer::bind(PrivConfig::new(&file, vec![uid]));
    assert!(matches!(third, Err(ServerError::NotASocket(_))));
    assert_eq!(std::fs::read(&file).unwrap(), b"data");
}

#[tokio::test]
async fn client_reports_a_missing_helper_clearly() {
    let dir = tempfile::tempdir().unwrap();
    let client = PrivClient::new(dir.path().join("nope.sock"));
    let err = client.ping("r").await.unwrap_err();
    assert_eq!(err.connect_kind(), Some(std::io::ErrorKind::NotFound));
}

#[tokio::test]
async fn totp_enrolment_and_login_through_the_socket() {
    let srv = start(|uid| vec![uid]);
    let client = srv.client();

    let (sealed, secret) = client.totp_new_secret("r1", "totp:u1").await.unwrap();
    assert!(sealed.starts_with("enc:v1:"));
    assert_eq!(secret.expose().len(), 32);

    // Compute the current code the way an authenticator app would.
    let raw = decode_base32(secret.expose());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let code = opsbox_priv::code_at(&raw, now / 30);

    let step = client
        .totp_verify("r2", &sealed, "totp:u1", &code, 0)
        .await
        .unwrap();
    assert_eq!(step, Some(now / 30));
    // Replay with the recorded step is refused; a wrong code never matches.
    assert_eq!(
        client
            .totp_verify("r3", &sealed, "totp:u1", &code, now / 30)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        client
            .totp_verify("r4", &sealed, "totp:u1", "000000", 0)
            .await
            .unwrap(),
        None
    );
    // A sealed secret is useless under another user's context.
    assert!(
        client
            .totp_verify("r5", &sealed, "totp:u2", &code, 0)
            .await
            .is_err()
    );
    srv.stop().await;
}

fn decode_base32(s: &str) -> Vec<u8> {
    let (mut buf, mut bits, mut out) = (0u32, 0u32, Vec::new());
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'2'..=b'7' => c - b'2' + 26,
            _ => panic!("not base32"),
        };
        buf = (buf << 5) | u32::from(v);
        bits += 5;
        if bits >= 8 {
            out.push((buf >> (bits - 8)) as u8);
            bits -= 8;
            buf &= (1 << bits) - 1;
        }
    }
    out
}

// ---- terminal -------------------------------------------------------------------------

mod terminal {
    use super::*;
    use opsbox_priv::PtySession;
    use opsbox_proto::PtyFrame;

    fn current_user() -> String {
        nix::unistd::User::from_uid(nix::unistd::geteuid())
            .unwrap()
            .expect("the test user is in the passwd database")
            .name
    }

    /// Read output until `needle` appears; returns everything read.
    async fn read_until(s: &mut PtySession, needle: &str) -> String {
        read_while(s, |seen| seen.contains(needle)).await
    }

    /// Read output until `done` is satisfied by everything seen so far.
    async fn read_while(s: &mut PtySession, done: impl Fn(&str) -> bool) -> String {
        let mut seen = String::new();
        let step = async {
            loop {
                match s.reader.recv().await.unwrap() {
                    Some(PtyFrame::Data(d)) => {
                        seen.push_str(&String::from_utf8_lossy(&d));
                        if done(&seen) {
                            return;
                        }
                    }
                    Some(PtyFrame::Exit { .. }) | None => return,
                    Some(other) => panic!("unexpected {other:?}"),
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(20), step)
            .await
            .unwrap_or_else(|_| panic!("timed out; saw {seen:?}"));
        assert!(done(&seen), "output ended too early: {seen:?}");
        seen
    }

    async fn type_line(s: &mut PtySession, line: &str) {
        s.writer
            .send(&PtyFrame::Data(format!("{line}\r").into_bytes()))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_shell_runs_commands_resizes_and_reports_its_exit_code() {
        let srv = start(|uid| vec![uid]);
        let mut s = srv
            .client()
            .pty_open("t1", Some("admin"), &current_user(), 80, 24)
            .await
            .unwrap();
        assert_eq!(s.user, current_user());

        // Keystrokes reach the shell and output comes back. The sum is computed by the
        // shell, so the echoed command line cannot satisfy the check by itself.
        type_line(&mut s, "echo sum=$((40+2))").await;
        read_until(&mut s, "sum=42").await;

        // The terminal size is real: a resize is visible to programs.
        s.writer
            .send(&PtyFrame::Resize {
                cols: 101,
                rows: 33,
            })
            .await
            .unwrap();
        type_line(&mut s, "stty size").await;
        read_until(&mut s, "33 101").await;

        type_line(&mut s, "exit 7").await;
        let mut exit = None;
        let wait = async {
            while let Some(f) = s.reader.recv().await.unwrap() {
                if let PtyFrame::Exit { code } = f {
                    exit = Some(code);
                    break;
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(20), wait)
            .await
            .expect("the shell ends");
        assert_eq!(exit, Some(Some(7)));
        srv.stop().await;
    }

    #[tokio::test]
    async fn closing_the_stream_ends_the_shell() {
        let srv = start(|uid| vec![uid]);
        let mut s = srv
            .client()
            .pty_open("t2", None, &current_user(), 80, 24)
            .await
            .unwrap();
        type_line(&mut s, "echo shellpid=$$.").await;
        // The echoed command line has `$$` where the answer has digits.
        let find = |seen: &str| -> Option<i32> {
            seen.split("shellpid=")
                .filter_map(|p| p.split_once('.'))
                .find_map(|(n, _)| n.parse().ok())
        };
        let seen = read_while(&mut s, |seen| find(seen).is_some()).await;
        let pid = find(&seen).expect("the shell reports its pid");
        let pid = nix::unistd::Pid::from_raw(pid);
        assert!(nix::sys::signal::kill(pid, None).is_ok(), "alive before");
        drop(s);
        // priv notices, hangs the shell up and reaps it; the server keeps serving.
        let gone = async {
            while nix::sys::signal::kill(pid, None).is_ok() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), gone)
            .await
            .expect("the shell is gone after the stream closes");
        assert!(srv.client().ping("p").await.is_ok());
        srv.stop().await;
    }

    #[tokio::test]
    async fn unknown_and_unusable_users_are_refused() {
        let srv = start(|uid| vec![uid]);
        for (name, code) in [
            ("no-such-user-xyz", PrivErrorCode::NotFound),
            ("-r", PrivErrorCode::Invalid),
            ("a b;c", PrivErrorCode::Invalid),
        ] {
            let err = srv
                .client()
                .pty_open("t3", None, name, 80, 24)
                .await
                .unwrap_err();
            match err {
                ClientError::Remote(e) => assert_eq!(e.code, code, "{name}"),
                other => panic!("{name}: {other:?}"),
            }
        }
        srv.stop().await;
    }

    #[tokio::test]
    async fn the_user_list_is_available() {
        let srv = start(|uid| vec![uid]);
        let users = srv.client().users("u1").await.unwrap();
        assert!(!users.is_empty());
        assert!(users.iter().all(|u| !u.name.is_empty()));
        srv.stop().await;
    }

    #[tokio::test]
    async fn a_user_who_is_not_allowed_cannot_open_a_terminal() {
        let srv = start(|uid| vec![uid.wrapping_add(1)]);
        let err = srv
            .client()
            .pty_open("t4", None, &current_user(), 80, 24)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, ClientError::Remote(e) if e.code == PrivErrorCode::Forbidden),
            "{err:?}"
        );
        srv.stop().await;
    }
}
