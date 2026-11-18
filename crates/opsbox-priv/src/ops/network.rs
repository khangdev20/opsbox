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
fn error(code: PrivErrorCode, message: &str) -> PrivError {
    PrivError {
        code,
        message: message.into(),
    }
}
fn internal() -> PrivError {
    error(
        PrivErrorCode::Internal,
        "Cannot complete the port change. Review the privileged helper logs and use Undo if needed.",
    )
}
fn invalid() -> PrivError {
    error(
        PrivErrorCode::Invalid,
        "The port holder changed or cannot be managed safely. Refresh Network settings.",
    )
}
fn confirmed(confirm: bool) -> Result<(), PrivError> {
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
fn docker_bin(env: &Env) -> String {
    env.docker_bin
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| exec::find(&["/usr/bin/docker", "/usr/local/bin/docker"]))
        .unwrap_or_else(|| "/usr/bin/docker".into())
}
fn systemctl_bin(env: &Env) -> String {
    env.network_systemctl
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/usr/bin/systemctl".into())
}
fn command(program: &str, args: &[&str]) -> Result<String, PrivError> {
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
fn valid_container(id: &str) -> bool {
    (12..=64).contains(&id.len()) && id.bytes().all(|c| c.is_ascii_hexdigit())
}
pub(crate) fn holders(env: &Env, ports: &[u16]) -> Result<Vec<PortHolder>, PrivError> {
    if ports.len() > 4 || ports.contains(&0) {
        return Err(invalid());
    }
    let root = &env.network_proc_root;
    let mut inodes = BTreeMap::new();
    for table in ["net/tcp", "net/tcp6"] {
        if let Ok(raw) = fs::read_to_string(root.join(table)) {
            inodes.extend(listening_inodes(&raw, ports));
        }
    }
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    let Ok(entries) = fs::read_dir(root) else {
        return Ok(result);
    };
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
            let candidates = if process == "docker-proxy" {
                containers(env, port)
            } else {
                vec![]
            };
            // Ambiguous Docker publication is informational only; never stop an arbitrary match.
            let container = (candidates.len() == 1).then(|| candidates[0].clone());
            result.push(PortHolder {
                port,
                pid,
                process,
                unit: if container.is_some() { None } else { unit },
                container,
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
fn control(env: &Env, op: Value) -> Result<Value, PrivError> {
    let (_, parsed) = config(env)?;
    let path = parsed
        .get("proxy")
        .and_then(|v| v.get("socket"))
        .and_then(|v| v.as_str())
        .unwrap_or("/run/opsbox-proxy/proxy.sock");
    let mut stream = UnixStream::connect(path).map_err(|_| internal())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
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
fn network(env: &Env) -> Result<ProxyNetwork, PrivError> {
    serde_json::from_value(control(env, json!({"type":"status"}))?["network"].clone())
        .map_err(|_| internal())
}
fn listeners(env: &Env, action: ListenerAction) -> Result<(), PrivError> {
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
fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> Result<(), PrivError> {
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
pub(crate) fn dispatch(op: &PrivOp, env: &Env) -> Result<PrivReply, PrivError> {
    if let PrivOp::PortHolders { ports } = op {
        return holders(env, ports).map(PrivReply::PortHolders);
    }
    let _guard = ACTION_LOCK.lock().map_err(|_| internal())?;
    match op {
        PrivOp::ProxyNetwork => Ok(PrivReply::ProxyNetwork {
            network: Box::new(network(env)?),
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
