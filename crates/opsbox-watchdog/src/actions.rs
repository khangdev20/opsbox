//! System actions executed as root by `opsbox-watchdog`: systemd unit restarts,
//! journal tailing, emergency 1-hour SSH, and scheduled reboot.

use std::net::IpAddr;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use opsbox_proto::frame;
use opsbox_proto::{FirewallChange, PrivOp, PrivReply, PrivRequest, PrivResponse, PrivResult};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::exec::{find, run, tail};

const CTL_TIMEOUT: Duration = Duration::from_secs(20);
const EMERGENCY_SECS: i64 = 3600;
const EMERGENCY_NOTE: &str = "opsbox emergency ssh";

#[derive(Debug, Clone, PartialEq, Eq)]
enum EmergencyBackend {
    None,
    Ufw,
    Firewalld,
    Nft,
}

#[derive(Debug, Clone)]
struct EmergencyEntry {
    backend: EmergencyBackend,
    ip: String,
    port: u16,
    until: i64,
}

static FALLBACK_SSH: Mutex<Vec<EmergencyEntry>> = Mutex::new(Vec::new());

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

pub fn systemd_available() -> bool {
    Path::new("/run/systemd/system").is_dir()
        && find(&["/usr/bin/systemctl", "/bin/systemctl"]).is_some()
}

pub fn proxy_unit_installed() -> bool {
    Path::new("/etc/systemd/system/opsbox-proxy.service").exists()
}

pub fn unit_state(unit: &str) -> Option<String> {
    if !systemd_available() {
        return None;
    }
    let ctl = find(&["/usr/bin/systemctl", "/bin/systemctl"])?;
    run(&ctl, &["is-active", unit], Duration::from_secs(3))
        .map(|o| o.stdout.trim().to_owned())
        .filter(|s| !s.is_empty())
}

pub fn restart_unit(unit: &str) -> Result<(), String> {
    if !matches!(
        unit,
        "opsbox.service" | "opsbox-priv.service" | "opsbox-proxy.service"
    ) {
        return Err(format!("Refusing to restart unknown unit {unit}."));
    }
    let ctl = find(&["/usr/bin/systemctl", "/bin/systemctl"])
        .ok_or_else(|| "systemctl is not available on this machine.".to_owned())?;
    let out = run(&ctl, &["restart", unit], CTL_TIMEOUT)
        .ok_or_else(|| format!("systemctl restart {unit} timed out"))?;
    if out.ok() {
        Ok(())
    } else {
        Err(format!(
            "Failed to restart {unit}: {}",
            tail(&out.combined(), 2).join(" ")
        ))
    }
}

pub fn restart_all_opsbox() -> Result<(), String> {
    restart_unit("opsbox-priv.service")?;
    if proxy_unit_installed() {
        restart_unit("opsbox-proxy.service")?;
    }
    restart_unit("opsbox.service")?;
    Ok(())
}

pub fn tail_opsbox_logs(limit: usize) -> Vec<String> {
    let Some(bin) = find(&["/usr/bin/journalctl", "/bin/journalctl"]) else {
        return Vec::new();
    };
    let n = limit.clamp(10, 200).to_string();
    let args = [
        "--no-pager",
        "-o",
        "short-iso",
        "-n",
        &n,
        "-u",
        "opsbox",
        "-u",
        "opsbox-priv",
        "-u",
        "opsbox-proxy",
        "-u",
        "opsbox-watchdog",
    ];
    match run(&bin, &args, Duration::from_secs(10)) {
        Some(o) if o.ok() => o
            .stdout
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(ToOwned::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

fn ssh_port() -> u16 {
    let mut texts = vec![std::fs::read_to_string("/etc/ssh/sshd_config").unwrap_or_default()];
    if let Ok(dir) = std::fs::read_dir("/etc/ssh/sshd_config.d") {
        for entry in dir.flatten() {
            if entry.path().extension().is_some_and(|e| e == "conf")
                && let Ok(text) = std::fs::read_to_string(entry.path())
            {
                texts.push(text);
            }
        }
    }
    texts
        .iter()
        .flat_map(|t| t.lines())
        .filter_map(|line| {
            let trimmed = line.trim();
            let rest = trimmed
                .strip_prefix("Port ")
                .or_else(|| trimmed.strip_prefix("Port\t"))?;
            rest.trim().parse::<u16>().ok()
        })
        .next()
        .unwrap_or(22)
}

async fn try_priv_ssh_emergency(
    priv_socket: &Path,
    caller_ip: &str,
) -> Option<Result<i64, String>> {
    let res = tokio::time::timeout(Duration::from_secs(10), async {
        let mut stream = UnixStream::connect(priv_socket).await.ok()?;
        let req = PrivRequest {
            id: "watchdog-ssh".into(),
            actor: Some("safemode".into()),
            op: PrivOp::SshEmergency {
                caller_ip: caller_ip.to_owned(),
            },
        };
        let bytes = frame::encode(&req).ok()?;
        stream.write_all(&bytes).await.ok()?;
        stream.flush().await.ok()?;
        let mut hdr = [0u8; 4];
        stream.read_exact(&mut hdr).await.ok()?;
        let len = frame::body_len(hdr).ok()?;
        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).await.ok()?;
        let resp: PrivResponse = frame::decode(&buf).ok()?;
        match resp.result {
            PrivResult::Success {
                reply: PrivReply::FirewallChange(FirewallChange { revert_at: Some(t) }),
            } => Some(Ok(t)),
            PrivResult::Success { .. } => Some(Err("Unexpected reply from helper.".to_owned())),
            PrivResult::Failure { error } => Some(Err(error.message)),
        }
    })
    .await;
    res.ok().flatten()
}

pub async fn open_ssh_1h(priv_socket: &Path, caller_ip: &str) -> Result<i64, String> {
    let ip: IpAddr = caller_ip
        .trim()
        .parse()
        .map_err(|_| "Enter a valid IPv4 or IPv6 address.".to_owned())?;
    if ip.is_loopback() || ip.is_unspecified() {
        return Err("Enter your public IP address so SSH is not opened to loopback.".to_owned());
    }
    let ip_str = ip.to_string();

    if let Some(priv_res) = try_priv_ssh_emergency(priv_socket, &ip_str).await {
        return priv_res;
    }

    // Fallback when `opsbox-priv` is down: nft owns its timeout; other backends expire here.
    let ctl = find(&["/usr/bin/systemctl", "/bin/systemctl"])
        .ok_or_else(|| "systemctl is not available on this machine.".to_owned())?;
    let started = ["ssh.service", "sshd.service"]
        .iter()
        .any(|u| run(&ctl, &["start", u], CTL_TIMEOUT).is_some_and(|o| o.ok()));
    if !started {
        return Err(
            "SSH is not installed on this machine, so there is nothing to open.".to_owned(),
        );
    }

    let port = ssh_port();
    let until = now_secs() + EMERGENCY_SECS;
    let backend = open_firewall_ssh(&ip, &ip_str, port)?;

    let mut guard = FALLBACK_SSH
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.retain(|e| e.ip != ip_str);
    guard.push(EmergencyEntry {
        backend,
        ip: ip_str,
        port,
        until,
    });
    Ok(until)
}

fn open_firewall_ssh(ip: &IpAddr, ip_str: &str, port: u16) -> Result<EmergencyBackend, String> {
    if let Some(ufw) = find(&["/usr/sbin/ufw", "/sbin/ufw"])
        && run(&ufw, &["status"], CTL_TIMEOUT)
            .is_some_and(|o| o.stdout.trim_start().starts_with("Status: active"))
    {
        let p = port.to_string();
        let out = run(
            &ufw,
            &[
                "--force",
                "allow",
                "proto",
                "tcp",
                "from",
                ip_str,
                "to",
                "any",
                "port",
                &p,
                "comment",
                EMERGENCY_NOTE,
            ],
            CTL_TIMEOUT,
        )
        .ok_or_else(|| "ufw timed out".to_owned())?;
        if !out.ok() {
            return Err(format!(
                "ufw refused the change: {}",
                tail(&out.combined(), 2).join(" ")
            ));
        }
        return Ok(EmergencyBackend::Ufw);
    }

    if let Some(fc) = find(&["/usr/bin/firewall-cmd", "/bin/firewall-cmd"])
        && run(&fc, &["--state"], CTL_TIMEOUT).is_some_and(|o| o.stdout.trim() == "running")
    {
        let family = if ip.is_ipv4() { "ipv4" } else { "ipv6" };
        let rich = format!(
            "rule family=\"{family}\" source address=\"{ip_str}\" port port=\"{port}\" protocol=\"tcp\" accept"
        );
        let out = run(&fc, &["--add-rich-rule", &rich], CTL_TIMEOUT)
            .ok_or_else(|| "firewalld timed out".to_owned())?;
        if !out.ok() {
            return Err(format!(
                "firewalld refused the change: {}",
                tail(&out.combined(), 2).join(" ")
            ));
        }
        return Ok(EmergencyBackend::Firewalld);
    }

    if let Some(nft) = find(&["/usr/sbin/nft", "/sbin/nft", "/usr/bin/nft"])
        && run(&nft, &["list", "table", "inet", "opsbox"], CTL_TIMEOUT).is_some_and(|o| o.ok())
    {
        open_nft_ssh(ip, ip_str, port, |args, input| {
            crate::exec::run_with_stdin(&nft, args, CTL_TIMEOUT, input)
        })?;
        return Ok(EmergencyBackend::Nft);
    }

    Ok(EmergencyBackend::None)
}

fn open_nft_ssh(
    ip: &IpAddr,
    ip_str: &str,
    port: u16,
    mut nft: impl FnMut(&[&str], Option<&str>) -> Option<crate::exec::CmdOutput>,
) -> Result<(), String> {
    let (set, address_type, field) = if ip.is_ipv4() {
        ("emergency_ssh4", "ipv4_addr", "ip saddr")
    } else {
        ("emergency_ssh6", "ipv6_addr", "ip6 saddr")
    };
    let element = format!("{{ {ip_str} . {port} }}");
    let mut script = String::new();
    if nft(&["list", "set", "inet", "opsbox", set], None).is_some_and(|o| o.ok()) {
        // Delete/re-add atomically to renew an existing grant's kernel deadline.
        if nft(&["get", "element", "inet", "opsbox", set, &element], None).is_some_and(|o| o.ok()) {
            script.push_str(&format!("delete element inet opsbox {set} {element}\n"));
        }
    } else {
        script.push_str(&format!(
            "add set inet opsbox {set} {{ type {address_type} . inet_service; flags timeout; timeout 1h; }}\n\
             insert rule inet opsbox input {field} . tcp dport @{set} accept comment \"{EMERGENCY_NOTE}\"\n"
        ));
    }
    // Empty sets/rules may remain, but only kernel-timed elements grant access.
    // Their lifetime is independent of housekeeping and watchdog process restarts.
    script.push_str(&format!(
        "add element inet opsbox {set} {{ {ip_str} . {port} timeout 1h }}\n"
    ));
    let out = nft(&["-f", "-"], Some(&script)).ok_or_else(|| "nft timed out".to_owned())?;
    if !out.ok() {
        return Err(format!(
            "nft refused the change: {}",
            tail(&out.combined(), 2).join(" ")
        ));
    }
    Ok(())
}

pub fn expire_fallback_ssh() -> Option<i64> {
    expire_fallback_ssh_at(&FALLBACK_SSH, now_secs())
}

fn expire_fallback_ssh_at(entries: &Mutex<Vec<EmergencyEntry>>, now: i64) -> Option<i64> {
    let mut guard = entries
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut expired = Vec::new();
    guard.retain(|e| {
        if e.until <= now {
            expired.push(e.clone());
            false
        } else {
            true
        }
    });
    let max_until = guard.iter().map(|e| e.until).max();
    drop(guard);

    for e in expired {
        match e.backend {
            EmergencyBackend::Ufw => {
                if let Some(ufw) = find(&["/usr/sbin/ufw", "/sbin/ufw"]) {
                    let p = e.port.to_string();
                    let _ = run(
                        &ufw,
                        &[
                            "--force", "delete", "allow", "proto", "tcp", "from", &e.ip, "to",
                            "any", "port", &p,
                        ],
                        CTL_TIMEOUT,
                    );
                }
            }
            EmergencyBackend::Firewalld => {
                if let (Some(fc), Ok(ip)) = (
                    find(&["/usr/bin/firewall-cmd", "/bin/firewall-cmd"]),
                    e.ip.parse::<IpAddr>(),
                ) {
                    let family = if ip.is_ipv4() { "ipv4" } else { "ipv6" };
                    let rich = format!(
                        "rule family=\"{family}\" source address=\"{}\" port port=\"{}\" protocol=\"tcp\" accept",
                        e.ip, e.port
                    );
                    let _ = run(&fc, &["--remove-rich-rule", &rich], CTL_TIMEOUT);
                }
            }
            // nft elements expire in the kernel, including while the watchdog is stopped.
            EmergencyBackend::Nft | EmergencyBackend::None => {}
        }
    }
    max_until
}

pub fn schedule_reboot() -> Result<String, String> {
    let shutdown = find(&["/usr/sbin/shutdown", "/sbin/shutdown", "/usr/bin/shutdown"])
        .ok_or_else(|| "shutdown command is not available on this machine.".to_owned())?;
    let out = run(
        &shutdown,
        &["-r", "+1", "opsbox safe mode: reboot requested"],
        CTL_TIMEOUT,
    )
    .ok_or_else(|| "Scheduling reboot timed out.".to_owned())?;
    if out.ok() {
        Ok("Reboot scheduled in 1 minute. Apps will return shortly.".to_owned())
    } else {
        Err(format!(
            "Failed to schedule reboot: {}",
            tail(&out.combined(), 2).join(" ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::CmdOutput;

    // A kernel-owned nft set and clock survive replacement of the watchdog's state.
    #[derive(Default)]
    struct NftKernel {
        now: i64,
        set: bool,
        timed: bool,
        allowance: Option<(String, u16, Option<i64>)>,
        rules: usize,
    }

    impl NftKernel {
        fn command(&mut self, args: &[&str], input: Option<&str>) -> Option<CmdOutput> {
            let ok = match args {
                ["list", "set", "inet", "opsbox", _] => self.set,
                ["get", "element", "inet", "opsbox", _, _] => self.allowed(),
                ["-f", "-"] => {
                    for line in input.unwrap().lines() {
                        let words: Vec<_> = line.split_whitespace().collect();
                        match words.as_slice() {
                            ["add", "set", ..] => {
                                self.set = true;
                                self.timed = line.contains("flags timeout;");
                                assert!(line.contains("_addr . inet_service;"));
                            }
                            ["delete", "element", ..] => self.allowance = None,
                            [
                                "add",
                                "element",
                                _,
                                _,
                                _,
                                "{",
                                ip,
                                ".",
                                port,
                                "timeout",
                                ttl,
                                "}",
                            ] => {
                                assert!(self.set && self.timed);
                                assert_eq!(*ttl, "1h");
                                self.allowance = Some((
                                    (*ip).into(),
                                    port.parse().unwrap(),
                                    Some(self.now + 3600),
                                ));
                            }
                            ["insert" | "add", "rule", ..] => {
                                self.rules += 1;
                                if !line.contains('@') {
                                    // The original literal accept rule has no kernel expiry.
                                    let at = words.iter().position(|w| *w == "saddr").unwrap();
                                    let port = words.iter().position(|w| *w == "dport").unwrap();
                                    self.allowance = Some((
                                        words[at + 1].into(),
                                        words[port + 1].parse().unwrap(),
                                        None,
                                    ));
                                }
                            }
                            _ => panic!("unexpected nft command: {line}"),
                        }
                    }
                    true
                }
                _ => panic!("unexpected nft args: {args:?}"),
            };
            Some(CmdOutput {
                code: Some(if ok { 0 } else { 1 }),
                stdout: String::new(),
                stderr: String::new(),
            })
        }

        fn allowed(&self) -> bool {
            self.allowance
                .as_ref()
                .is_some_and(|(_, _, until)| until.is_none_or(|until| self.now < until))
        }
    }

    #[test]
    fn nft_emergency_ssh_expires_after_one_hour_even_after_watchdog_restart() {
        for source in ["198.51.100.23", "2001:db8::23"] {
            for restart in [false, true] {
                let ip: IpAddr = source.parse().unwrap();
                let mut kernel = NftKernel::default();
                open_nft_ssh(&ip, source, 2222, |args, input| kernel.command(args, input)).unwrap();
                assert!(kernel.allowed());
                assert_eq!(
                    kernel
                        .allowance
                        .as_ref()
                        .map(|(ip, port, _)| (ip.as_str(), *port)),
                    Some((source, 2222))
                );
                let mut watchdog = Mutex::new(vec![EmergencyEntry {
                    backend: EmergencyBackend::Nft,
                    ip: source.into(),
                    port: 2222,
                    until: 3600,
                }]);
                kernel.now = 59 * 60;
                assert!(kernel.allowed());
                assert_eq!(expire_fallback_ssh_at(&watchdog, kernel.now), Some(3600));
                if restart {
                    // A new watchdog process loses its in-memory tracking, not kernel state.
                    watchdog = Mutex::new(Vec::new());
                }
                kernel.now = 61 * 60;
                assert_eq!(expire_fallback_ssh_at(&watchdog, kernel.now), None);
                assert!(watchdog.lock().unwrap().is_empty());
                assert!(
                    !kernel.allowed(),
                    "{source}: allowance survived 61m (restart={restart})"
                );
            }
        }
    }

    #[test]
    fn nft_emergency_ssh_renewal_reuses_rule_and_refreshes_timeout() {
        let source = "198.51.100.23";
        let ip = source.parse().unwrap();
        let mut kernel = NftKernel::default();
        for now in [0, 1800] {
            kernel.now = now;
            open_nft_ssh(&ip, source, 2222, |args, input| kernel.command(args, input)).unwrap();
            assert_eq!(kernel.rules, 1);
        }
        for now in [3660, 5399] {
            kernel.now = now;
            assert!(kernel.allowed());
        }
        kernel.now = 5400;
        assert!(!kernel.allowed());
        kernel.now = 6000;
        open_nft_ssh(&ip, source, 2222, |args, input| kernel.command(args, input)).unwrap();
        assert!(kernel.allowed());
        assert_eq!(kernel.rules, 1);
        kernel.now = 9600;
        assert!(!kernel.allowed());
    }

    #[test]
    fn nft_emergency_ssh_reports_timeout_and_rejection() {
        let source = "198.51.100.23";
        let ip = source.parse().unwrap();
        for timed_out in [false, true] {
            let result = open_nft_ssh(&ip, source, 22, |args, _| {
                if args == ["-f", "-"] && timed_out {
                    None
                } else {
                    Some(CmdOutput {
                        code: Some(1),
                        stdout: String::new(),
                        stderr: "denied".into(),
                    })
                }
            });
            assert!(
                result
                    .unwrap_err()
                    .contains(if timed_out { "timed out" } else { "denied" })
            );
        }
    }
}
