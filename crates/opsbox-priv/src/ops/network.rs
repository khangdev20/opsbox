//! Port ownership and reversible takeover. Commands always use separate argv entries.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use super::{Env, exec};
use opsbox_proto::{
    ListenerAction, PortContainer, PortHolder, PortTakeover, PortTarget, PrivError, PrivErrorCode,
    PrivOp, PrivReply, ProxyNetwork, frame,
};
use serde_json::{Value, json};

static ACTION_LOCK: Mutex<()> = Mutex::new(());
pub(super) fn error(code: PrivErrorCode, message: &str) -> PrivError {
    PrivError {
        code,
        message: message.into(),
    }
}
pub(super) fn internal() -> PrivError {
    error(
        PrivErrorCode::Internal,
        "Cannot complete the port change. Review the privileged helper logs and use Undo if needed.",
    )
}
pub(super) fn invalid() -> PrivError {
    error(
        PrivErrorCode::Invalid,
        "The port holder changed or cannot be managed safely. Refresh Network settings.",
    )
}
pub(super) fn confirmed(confirm: bool) -> Result<(), PrivError> {
    if confirm {
        Ok(())
    } else {
        Err(error(
            PrivErrorCode::ConfirmRequired,
            "Confirm this change before continuing.",
        ))
    }
}

/// Both Linux TCP tables have the same local address, state and inode columns.
pub fn listening_inodes(raw: &str, ports: &[u16]) -> BTreeMap<String, u16> {
    raw.lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 10 || fields[3] != "0A" {
                return None;
            }
            let port = u16::from_str_radix(fields[1].rsplit_once(':')?.1, 16).ok()?;
            (ports.contains(&port) && fields[9] != "0").then(|| (fields[9].to_owned(), port))
        })
        .collect()
}
fn service_unit(raw: &str) -> Option<String> {
    raw.lines()
        .filter_map(|line| line.splitn(3, ':').nth(2))
        .find_map(|path| {
            let parts: Vec<_> = path.split('/').filter(|p| !p.is_empty()).collect();
            if parts
                .iter()
                .any(|p| *p == "user.slice" || p.ends_with(".scope"))
            {
                return None;
            }
            parts
                .last()
                .filter(|p| valid_unit(p))
                .map(|p| (*p).to_owned())
        })
}
fn valid_unit(unit: &str) -> bool {
    unit.ends_with(".service")
        && unit.len() <= 128
        && !unit.starts_with('-')
        && !unit.starts_with("opsbox")
        && unit != "docker.service"
        && unit != "containerd.service"
        && unit
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.@:-".contains(&c))
}
pub(super) fn docker_bin(env: &Env) -> String {
    env.docker_bin
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| exec::find(&["/usr/bin/docker", "/usr/local/bin/docker"]))
        .unwrap_or_else(|| "/usr/bin/docker".into())
}
pub(super) fn systemctl_bin(env: &Env) -> String {
    env.network_systemctl
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/usr/bin/systemctl".into())
}
pub(super) fn command(program: &str, args: &[&str]) -> Result<String, PrivError> {
    let out = exec::run(program, args, Duration::from_secs(20)).ok_or_else(internal)?;
    if !out.ok() {
        tracing::warn!(program, args = ?args, "port-conflict command failed");
        return Err(internal());
    }
    Ok(out.stdout.trim().into())
}
/// Filter results are inspected again: only a running container with the exact host TCP port qualifies.
fn containers(env: &Env, port: u16) -> Vec<PortContainer> {
    let binary = docker_bin(env);
    let Ok(ids) = command(
        &binary,
        &[
            "ps",
            "--no-trunc",
            "--filter",
            &format!("publish={port}"),
            "--format",
            "{{.ID}}",
        ],
    ) else {
        return vec![];
    };
    ids.lines()
        .filter(|id| valid_container(id))
        .filter_map(|id| {
            let value: Value =
                serde_json::from_str(&command(&binary, &["inspect", id]).ok()?).ok()?;
            let c = value.as_array()?.first()?;
            if c["State"]["Running"].as_bool() != Some(true) {
                return None;
            }
            let matches =
                c["NetworkSettings"]["Ports"]
                    .as_object()?
                    .iter()
                    .any(|(key, bindings)| {
                        key.ends_with("/tcp")
                            && bindings.as_array().is_some_and(|bs| {
                                bs.iter()
                                    .any(|b| b["HostPort"].as_str() == Some(&port.to_string()))
                            })
                    });
            matches.then(|| PortContainer {
                id: c["Id"].as_str().unwrap_or(id).into(),
                name: c["Name"]
                    .as_str()
                    .unwrap_or(id)
                    .trim_start_matches('/')
                    .into(),
                image: c["Config"]["Image"].as_str().unwrap_or("unknown").into(),
            })
        })
        .collect()
}
pub(super) fn valid_container(id: &str) -> bool {
    (12..=64).contains(&id.len()) && id.bytes().all(|c| c.is_ascii_hexdigit())
}
pub(crate) fn holders(env: &Env, ports: &[u16]) -> Result<Vec<PortHolder>, PrivError> {
    if ports.len() > 4 || ports.contains(&0) {
        return Err(invalid());
    }
    let mut result = proc_holders(&env.network_proc_root, ports)?;
    for holder in &mut result {
        let candidates = if holder.process == "docker-proxy" {
            containers(env, holder.port)
        } else {
            vec![]
        };
        // Ambiguous publication is informational only; never stop an arbitrary match.
        holder.container = (candidates.len() == 1).then(|| candidates[0].clone());
        if holder.container.is_some() {
            holder.unit = None;
        }
    }
    Ok(result)
}

fn proc_holders(root: &Path, ports: &[u16]) -> Result<Vec<PortHolder>, PrivError> {
    let mut readable = false;
    let mut inodes = BTreeMap::new();
    for table in ["net/tcp", "net/tcp6"] {
        if let Ok(raw) = fs::read_to_string(root.join(table)) {
            readable = true;
            inodes.extend(listening_inodes(&raw, ports));
        }
    }
    if !readable {
        return Err(internal());
    }
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    let entries = fs::read_dir(root).map_err(|_| internal())?;
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let base = entry.path();
        let Ok(fds) = fs::read_dir(base.join("fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            let Ok(link) = fs::read_link(fd.path()) else {
                continue;
            };
            let link = link.to_string_lossy();
            let Some(inode) = link
                .strip_prefix("socket:[")
                .and_then(|s| s.strip_suffix(']'))
            else {
                continue;
            };
            let Some(&port) = inodes.get(inode) else {
                continue;
            };
            if !seen.insert((pid, port)) {
                continue;
            }
            let process = fs::read_to_string(base.join("comm"))
                .unwrap_or_else(|_| "unknown".into())
                .trim()
                .to_owned();
            let unit = fs::read_to_string(base.join("cgroup"))
                .ok()
                .and_then(|s| service_unit(&s));
            result.push(PortHolder {
                port,
                pid,
                process,
                unit,
                container: None,
            });
        }
    }
    result.sort_by_key(|h| (h.port, h.pid));
    Ok(result)
}

fn config(env: &Env) -> Result<(String, toml::Value), PrivError> {
    let raw = match fs::read_to_string(&env.config_path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return Err(internal()),
    };
    let parsed = toml::from_str(&raw).map_err(|_| internal())?;
    Ok((raw, parsed))
}
pub(super) fn control(env: &Env, op: Value) -> Result<Value, PrivError> {
    let (_, parsed) = config(env)?;
    let path = parsed
        .get("proxy")
        .and_then(|v| v.get("socket"))
        .and_then(|v| v.as_str())
        .unwrap_or("/run/opsbox-proxy/proxy.sock");
    let mut stream = UnixStream::connect(path).map_err(|_| internal())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(if op["type"] == "issue_cert" {
            180
        } else {
            10
        })))
        .map_err(|_| internal())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| internal())?;
    let bytes = frame::encode(&json!({"id":"priv-network","op":op})).map_err(|_| internal())?;
    stream.write_all(&bytes).map_err(|_| internal())?;
    let mut header = [0; 4];
    stream.read_exact(&mut header).map_err(|_| internal())?;
    let mut body = vec![0; frame::body_len(header).map_err(|_| internal())?];
    stream.read_exact(&mut body).map_err(|_| internal())?;
    let value: Value = frame::decode(&body).map_err(|_| internal())?;
    if value["result"]["status"] != "ok" {
        return Err(internal());
    }
    Ok(value["result"]["reply"]["data"].clone())
}
pub(super) fn network(env: &Env) -> Result<ProxyNetwork, PrivError> {
    serde_json::from_value(control(env, json!({"type":"status"}))?["network"].clone())
        .map_err(|_| internal())
}
pub(super) fn listeners(env: &Env, action: ListenerAction) -> Result<(), PrivError> {
    control(env, json!({"type":"listeners","args":{"action":action}})).map(|_| ())
}
fn receipts_path(env: &Env) -> PathBuf {
    env.config_path.with_file_name("proxy-takeovers.json")
}
fn receipts(env: &Env) -> Result<Vec<PortTakeover>, PrivError> {
    match fs::read(receipts_path(env)) {
        Ok(raw) => serde_json::from_slice(&raw).map_err(|_| internal()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(_) => Err(internal()),
    }
}
pub(super) fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> Result<(), PrivError> {
    let parent = path.parent().ok_or_else(internal)?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|_| internal())?;
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(mode))
        .map_err(|_| internal())?;
    file.write_all(bytes)
        .and_then(|()| file.as_file().sync_all())
        .map_err(|_| internal())?;
    file.persist(path).map_err(|_| internal())?;
    fs::File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|_| internal())
}
fn save_receipts(env: &Env, rows: &[PortTakeover]) -> Result<(), PrivError> {
    atomic_write(
        &receipts_path(env),
        &serde_json::to_vec(rows).map_err(|_| internal())?,
        0o600,
    )
}
fn policy(env: &Env, target: &PortTarget) -> Result<(String, bool), PrivError> {
    match target {
        PortTarget::Container(id) if valid_container(id) => {
            let raw = command(&docker_bin(env), &["inspect", id])?;
            let value: Value = serde_json::from_str(&raw).map_err(|_| internal())?;
            let c = &value[0];
            if c["Id"].as_str() != Some(id) {
                return Err(invalid());
            }
            let policy = c["HostConfig"]["RestartPolicy"]["Name"]
                .as_str()
                .ok_or_else(invalid)?;
            let max = c["HostConfig"]["RestartPolicy"]["MaximumRetryCount"]
                .as_u64()
                .unwrap_or(0);
            let policy = if policy == "on-failure" && max > 0 {
                format!("on-failure:{max}")
            } else {
                policy.into()
            };
            Ok((policy, c["State"]["Running"].as_bool().unwrap_or(false)))
        }
        PortTarget::Unit(unit) if valid_unit(unit) => {
            let enabled = command(
                &systemctl_bin(env),
                &["show", "--property=UnitFileState", "--value", unit],
            )?;
            if !["enabled", "enabled-runtime", "disabled", "static"].contains(&enabled.as_str()) {
                return Err(invalid());
            }
            Ok((enabled, true))
        }
        _ => Err(invalid()),
    }
}
fn restore(env: &Env, receipt: &PortTakeover) -> Result<(), PrivError> {
    match &receipt.target {
        PortTarget::Container(id) => {
            command(
                &docker_bin(env),
                &[
                    "update",
                    &format!("--restart={}", receipt.previous_policy),
                    id,
                ],
            )?;
            if receipt.was_running {
                command(&docker_bin(env), &["start", id])?;
            }
        }
        PortTarget::Unit(unit) => {
            if receipt.previous_policy == "enabled" {
                command(&systemctl_bin(env), &["enable", unit])?;
            }
            if receipt.previous_policy == "enabled-runtime" {
                command(&systemctl_bin(env), &["enable", "--runtime", unit])?;
            }
            if receipt.was_running {
                command(&systemctl_bin(env), &["start", unit])?;
            }
        }
    }
    Ok(())
}
/// Preserve comments and all unrelated keys; reject unsupported TOML layouts safely.
fn replace_ports(
    raw: &str,
    http: std::net::SocketAddr,
    https: std::net::SocketAddr,
) -> Result<String, PrivError> {
    let mut out = Vec::new();
    let mut inside = false;
    let mut found = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            inside = trimmed == "[proxy]";
            if inside {
                found = true;
                out.push(line.to_owned());
                out.push(format!("listen_http = \"{http}\""));
                out.push(format!("listen_https = \"{https}\""));
                continue;
            }
        }
        if inside
            && trimmed
                .split_once('=')
                .is_some_and(|(key, _)| ["listen_http", "listen_https"].contains(&key.trim()))
        {
            continue;
        }
        out.push(line.to_owned());
    }
    if !found {
        out.extend([
            "[proxy]".into(),
            format!("listen_http = \"{http}\""),
            format!("listen_https = \"{https}\""),
        ]);
    }
    let result = out.join("\n") + "\n";
    let _: toml::Value = toml::from_str(&result).map_err(|_| invalid())?;
    Ok(result)
}
fn mode(env: &Env) -> Result<opsbox_proto::ProxyMode, PrivError> {
    let (_, parsed) = config(env)?;
    parsed
        .get("proxy")
        .and_then(|p| p.get("mode"))
        .map(|v| v.clone().try_into().map_err(|_| invalid()))
        .unwrap_or(Ok(opsbox_proto::ProxyMode::Builtin))
}

/// Preserve source text and verify that the requested value is the only semantic change.
fn replace_mode(raw: &str, mode: opsbox_proto::ProxyMode) -> Result<String, PrivError> {
    let mut expected: toml::Value = toml::from_str(raw).map_err(|_| invalid())?;
    let table = expected.as_table_mut().ok_or_else(invalid)?;
    let proxy = table
        .entry("proxy")
        .or_insert_with(|| toml::Value::Table(Default::default()));
    proxy
        .as_table_mut()
        .ok_or_else(invalid)?
        .insert("mode".into(), toml::Value::String(mode.to_string()));
    let mut out = Vec::new();
    let mut inside = false;
    let mut found = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            inside = trimmed.split('#').next().unwrap_or("").trim() == "[proxy]";
            if inside {
                found = true;
                out.push(line.to_owned());
                out.push(format!("mode = \"{mode}\""));
                continue;
            }
        }
        if inside
            && trimmed
                .split_once('=')
                .is_some_and(|(key, _)| key.trim() == "mode")
        {
            continue;
        }
        out.push(line.to_owned());
    }
    if !found {
        out.extend(["[proxy]".into(), format!("mode = \"{mode}\"")]);
    }
    let updated = out.join("\n") + "\n";
    let parsed: toml::Value = toml::from_str(&updated).map_err(|_| invalid())?;
    if parsed != expected {
        return Err(invalid());
    }
    Ok(updated)
}

fn set_mode(
    env: &Env,
    desired: opsbox_proto::ProxyMode,
    confirm: bool,
) -> Result<PrivReply, PrivError> {
    confirmed(confirm)?;
    // An active adoption/takeover must first be undone; disabling its proxy would strand sites.
    if desired != opsbox_proto::ProxyMode::Builtin
        && (!receipts(env)?.is_empty()
            || matches!(super::adoption::dispatch(&PrivOp::WebMoves, env)?, PrivReply::WebMoves(rows) if rows.iter().any(|r| r.active)))
    {
        return Err(error(
            PrivErrorCode::Invalid,
            "Undo active web moves and port takeovers in Network settings before disabling the proxy.",
        ));
    }
    let (raw, _) = config(env)?;
    let existed = env.config_path.exists();
    let updated = replace_mode(&raw, desired)?;
    let permissions =
        fs::metadata(&env.config_path).map_or(0o644, |m| m.permissions().mode() & 0o777);
    atomic_write(&env.config_path, updated.as_bytes(), permissions)?;
    if command(&systemctl_bin(env), &["restart", "opsbox-proxy.service"]).is_err() {
        if existed {
            atomic_write(&env.config_path, raw.as_bytes(), permissions)?;
        } else {
            fs::remove_file(&env.config_path).map_err(|_| internal())?;
        }
        let _ = command(&systemctl_bin(env), &["restart", "opsbox-proxy.service"]);
        return Err(error(
            PrivErrorCode::Internal,
            "The proxy could not restart. The previous configuration was restored; check the proxy service logs.",
        ));
    }
    Ok(PrivReply::Done)
}

pub(crate) fn dispatch(op: &PrivOp, env: &Env) -> Result<PrivReply, PrivError> {
    if matches!(op, PrivOp::ProxyModeGet) {
        return mode(env).map(PrivReply::ProxyMode);
    }
    if let PrivOp::PortHolders { ports } = op {
        return holders(env, ports).map(PrivReply::PortHolders);
    }
    if matches!(op, PrivOp::WebDiscover { .. } | PrivOp::WebMoves) {
        return super::adoption::dispatch(op, env);
    }
    let _guard = ACTION_LOCK.lock().map_err(|_| internal())?;
    match op {
        PrivOp::WebMove { .. } | PrivOp::WebUndo { .. } | PrivOp::WebTls { .. } => {
            super::adoption::dispatch(op, env)
        }
        PrivOp::ProxyModeSet { mode, confirm } => set_mode(env, *mode, *confirm),
        PrivOp::ProxyNetwork => Ok(PrivReply::ProxyNetwork {
            network: Box::new(if mode(env)? == opsbox_proto::ProxyMode::Builtin {
                network(env)?
            } else {
                ProxyNetwork::default()
            }),
            undo: receipts(env)?,
        }),
        PrivOp::ProxyTakeover {
            port,
            target,
            confirm,
        } => {
            confirmed(*confirm)?;
            let net = network(env)?;
            if !net.port_conflict || !net.conflict_ports.contains(port) {
                return Err(invalid());
            }
            let owners = holders(env, &[*port])?;
            if owners.is_empty()
                || !owners.iter().all(|h| match target {
                    PortTarget::Container(id) => h.container.as_ref().is_some_and(|c| &c.id == id),
                    PortTarget::Unit(unit) => {
                        h.container.is_none() && h.unit.as_ref() == Some(unit)
                    }
                })
            {
                return Err(invalid());
            }
            let (previous_policy, was_running) = policy(env, target)?;
            let mut rows = receipts(env)?;
            if rows.iter().any(|r| &r.target == target) || rows.len() >= 32 {
                return Err(invalid());
            }
            let mut random = [0u8; 16];
            getrandom::fill(&mut random).map_err(|_| internal())?;
            let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
            let receipt = PortTakeover {
                id,
                target: target.clone(),
                previous_policy,
                was_running,
            };
            rows.push(receipt.clone());
            save_receipts(env, &rows)?;
            let result = match target {
                PortTarget::Container(id) => {
                    command(&docker_bin(env), &["update", "--restart=no", id])
                        .and_then(|_| command(&docker_bin(env), &["stop", id]))
                }
                PortTarget::Unit(unit) => command(&systemctl_bin(env), &["disable", unit])
                    .and_then(|_| command(&systemctl_bin(env), &["stop", unit])),
            };
            if result.is_err() {
                tracing::warn!(receipt = ?receipt, "takeover incomplete; persistent Undo retained");
            }
            result?;
            Ok(PrivReply::PortTakeover(receipt))
        }
        PrivOp::ProxyUndo { id, confirm } => {
            confirmed(*confirm)?;
            let mut rows = receipts(env)?;
            let index = rows.iter().position(|r| &r.id == id).ok_or_else(invalid)?;
            listeners(env, ListenerAction::YieldPrimary)?;
            restore(env, &rows[index])?;
            let receipt = rows.remove(index);
            save_receipts(env, &rows)?;
            Ok(PrivReply::PortTakeover(receipt))
        }
        PrivOp::ProxyKeepPorts { confirm } => {
            confirmed(*confirm)?;
            let net = network(env)?;
            if !net.port_conflict {
                return Err(invalid());
            }
            let (raw, _) = config(env)?;
            let updated = replace_ports(
                &raw,
                net.listen_http.ok_or_else(invalid)?,
                net.listen_https.ok_or_else(invalid)?,
            )?;
            let mode =
                fs::metadata(&env.config_path).map_or(0o644, |m| m.permissions().mode() & 0o777);
            atomic_write(&env.config_path, updated.as_bytes(), mode)?;
            listeners(env, ListenerAction::KeepFallback)?;
            Ok(PrivReply::Done)
        }
        _ => Err(invalid()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proxy_mode_edit_preserves_settings_and_rejects_unsupported_layouts() {
        let raw = "[log]\nmode = 'keep'\n[proxy] # public proxy\nmode = 'builtin'\nlisten_http = '0.0.0.0:80'\n";
        let edited = replace_mode(raw, opsbox_proto::ProxyMode::External).unwrap();
        let parsed: toml::Value = toml::from_str(&edited).unwrap();
        assert_eq!(parsed["proxy"]["mode"].as_str(), Some("external"));
        assert_eq!(parsed["log"]["mode"].as_str(), Some("keep"));
        assert!(edited.contains("# public proxy"));
        assert_eq!(
            replace_mode(&edited, opsbox_proto::ProxyMode::External).unwrap(),
            edited
        );
        assert!(
            replace_mode("proxy = { mode = 'builtin' }", opsbox_proto::ProxyMode::Off).is_err()
        );
        let empty = replace_mode("", opsbox_proto::ProxyMode::Off).unwrap();
        assert!(empty.contains("mode = \"off\""));
    }
    #[test]
    fn tcp_v4_v6_listeners_only_and_units() {
        for local in ["00000000:0050", "00000000000000000000000000000000:0050"] {
            let raw = format!(
                "0: {local} 00000000:0000 0A 0:0 0:0 0 0 0 12345 1\n1: {local} 0:0 01 0:0 0:0 0 0 0 999 1"
            );
            assert_eq!(
                listening_inodes(&raw, &[80]),
                BTreeMap::from([("12345".into(), 80)])
            );
            assert!(listening_inodes(&raw, &[443]).is_empty());
        }
        assert_eq!(
            service_unit("0::/system.slice/nginx.service"),
            Some("nginx.service".into())
        );
        assert_eq!(
            service_unit("0::/system.slice/docker.service/docker-abc.scope"),
            None
        );
        assert_eq!(
            service_unit(
                "0::/user.slice/user-1000.slice/user@1000.service/app.slice/nginx.service"
            ),
            None
        );
        assert_eq!(
            service_unit("0::/system.slice/run-test.scope/nginx.service"),
            None
        );
        assert!(!valid_unit("opsbox-priv.service"));
        assert!(!valid_unit("--bad.service"));
        assert!(!valid_container(";stop something"));
        assert_eq!(
            confirmed(false).unwrap_err().code,
            PrivErrorCode::ConfirmRequired
        );
    }
    #[test]
    fn fake_port_table_holders_and_unavailable_proc() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("net")).unwrap();
        fs::create_dir_all(root.join("321/fd")).unwrap();
        fs::write(
            root.join("net/tcp"),
            "0: 00000000:0050 0:0 0A 0:0 0:0 0 0 0 1234 1",
        )
        .unwrap();
        fs::write(
            root.join("net/tcp6"),
            "0: 00000000000000000000000000000000:01BB 0:0 0A 0:0 0:0 0 0 0 5678 1",
        )
        .unwrap();
        fs::write(root.join("321/comm"), "caddy\n").unwrap();
        for (fd, inode) in [(3, 1234), (4, 1234), (5, 5678)] {
            std::os::unix::fs::symlink(
                format!("socket:[{inode}]"),
                root.join(format!("321/fd/{fd}")),
            )
            .unwrap();
        }
        let holders = proc_holders(root, &[80, 443]).unwrap();
        assert_eq!(
            holders
                .iter()
                .map(|h| (h.port, h.pid, h.process.as_str()))
                .collect::<Vec<_>>(),
            vec![(80, 321, "caddy"), (443, 321, "caddy")]
        );
        fs::remove_dir_all(root.join("net")).unwrap();
        assert!(
            proc_holders(root, &[80, 443]).is_err(),
            "unavailable ownership data must not look like an empty table"
        );
    }
    #[test]
    fn keep_ports_preserves_other_configuration() {
        let updated = replace_ports(
            "# hi\n[proxy]\nlisten_http = '0.0.0.0:80'\n# note\n[api]\nlisten = '127.0.0.1:8443'\n",
            "0.0.0.0:8081".parse().unwrap(),
            "0.0.0.0:8446".parse().unwrap(),
        )
        .unwrap();
        let v: toml::Value = toml::from_str(&updated).unwrap();
        assert_eq!(v["proxy"]["listen_http"].as_str(), Some("0.0.0.0:8081"));
        assert_eq!(v["api"]["listen"].as_str(), Some("127.0.0.1:8443"));
        assert!(updated.contains("# note"));
    }
}
