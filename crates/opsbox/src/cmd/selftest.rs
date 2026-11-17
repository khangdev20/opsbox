//! `opsbox selftest`: a hermetic check that this binary works on this machine. It
//! needs no root, no network and no existing configuration, and cleans up after
//! itself. The updater runs it before switching to a new version.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use nix::unistd::getuid;
use opsbox_priv::{PrivClient, PrivConfig, PrivServer};
use opsbox_proto::frame;
use opsbox_proto::{PrivOp, PrivRequest};
use opsbox_store::{ActorKind, AuditResult, NewAudit, Store};

use crate::VERSION;

/// Returns `true` if every check passed.
pub fn run() -> bool {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            println!("FAIL  async runtime: {e}");
            return false;
        }
    };

    let mut all_ok = true;
    let mut report = |name: &str, outcome: Result<String>| match outcome {
        Ok(detail) => println!("ok    {name}: {detail}"),
        Err(e) => {
            all_ok = false;
            println!("FAIL  {name}: {e:#}");
        }
    };

    println!("opsbox {VERSION} selftest");
    report("embedded web UI", check_ui());
    report("database and audit chain", check_database());
    report("privileged protocol", check_protocol());
    report("reverse proxy checks", check_proxy());
    report(
        "privileged socket and peer check",
        runtime.block_on(check_priv_roundtrip()),
    );
    all_ok
}

fn check_ui() -> Result<String> {
    let files = opsbox_api::check_embedded_ui().map_err(anyhow::Error::msg)?;
    Ok(format!("{files} files"))
}

fn check_database() -> Result<String> {
    let store = Store::open_in_memory()?;
    for i in 0..3 {
        store.append_audit(NewAudit {
            actor: "selftest".into(),
            actor_kind: ActorKind::System,
            ip: None,
            action: format!("selftest.{i}"),
            target: None,
            result: AuditResult::Ok,
            detail: serde_json::json!({}),
        })?;
    }
    let report = store
        .verify_audit()
        .context("audit chain does not verify")?;
    Ok(format!(
        "schema v{}, {} audit entries chained",
        store.schema_version()?,
        report.entries
    ))
}

fn check_protocol() -> Result<String> {
    let request = PrivRequest {
        id: "selftest".into(),
        actor: None,
        op: PrivOp::HostInfo,
    };
    let bytes = frame::encode(&request)?;
    let header: [u8; 4] = bytes[..4].try_into().context("short frame")?;
    if frame::body_len(header)? != bytes.len() - 4 {
        bail!("frame length does not match its body");
    }
    if frame::decode::<PrivRequest>(&bytes[4..])? != request {
        bail!("request changed in transit");
    }
    Ok("frame round trip".into())
}

fn check_proxy() -> Result<String> {
    let normalized = opsbox_proxy::normalize_domain("https://Panel.Example.com/")
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if normalized != "panel.example.com" {
        bail!("unexpected domain normalization: {normalized}");
    }
    let req = opsbox_proxy::ProxyRequest {
        id: "selftest".into(),
        op: opsbox_proxy::ProxyOp::Ping,
    };
    let encoded = frame::encode(&req)?;
    let decoded: opsbox_proxy::ProxyRequest = frame::decode(&encoded[4..])?;
    if decoded != req {
        bail!("proxy request changed in transit");
    }
    Ok("domain & protocol ok".into())
}

/// Start a private helper on a temporary socket, call it as ourselves, and check
/// that it answers with real host facts.
async fn check_priv_roundtrip() -> Result<String> {
    let dir = tempfile::Builder::new()
        .prefix("opsbox-selftest-")
        .tempdir()
        .context("cannot create a temporary directory")?;
    let socket = dir.path().join("priv.sock");

    let server = PrivServer::bind(PrivConfig::new(&socket, vec![getuid().as_raw()]))
        .context("cannot start a helper on a temporary socket")?;
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(server.run(async {
        let _ = stopped.await;
    }));

    let client = PrivClient::new(&socket).with_timeout(Duration::from_secs(5));
    let outcome = async {
        let version = client.ping("selftest-ping").await?;
        let info = client.host_info("selftest-host", Some("selftest")).await?;
        Ok::<_, opsbox_priv::ClientError>((version, info))
    }
    .await;

    let _ = stop.send(());
    let _ = task.await;

    let (version, info) = outcome.context("helper call failed")?;
    if info.hostname.is_empty() || info.cpu_count == 0 || info.mem_total_bytes == 0 {
        bail!("host facts look empty: {info:?}");
    }
    Ok(format!(
        "helper v{version} answered for {}, {} CPUs",
        info.hostname, info.cpu_count
    ))
}
