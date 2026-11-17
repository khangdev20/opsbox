//! The firewall (docs/BACKEND_SPEC.md 7.8): manage `ufw` or `firewalld` if one is active,
//! otherwise own an nftables table `inet opsbox`.
//!
//! The decisions that protect the administrator (does this change cut the caller off? what
//! does the ruleset look like?) are pure functions with tests. A change that could lock
//! the caller out needs `confirm`, is applied, and is undone after 60 seconds unless
//! `FirewallConfirm` arrives; a janitor thread performs the undo and closes emergency
//! SSH entries when their hour is up.

use std::net::IpAddr;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use opsbox_proto::{
    FirewallChange, FirewallRule, FirewallState, NewRule, PrivError, PrivErrorCode,
};
use serde::{Deserialize, Serialize};

use super::exec::{find, run, run_with, tail};

const STATE_FILE: &str = "/etc/opsbox/firewall.json";
const REVERT_SECS: i64 = 60;
const EMERGENCY_SECS: i64 = 3600;
/// Opened for the desktop app's private network (docs/BACKEND_SPEC.md 7.17); kept always.
const WIREGUARD_PORT: u16 = 51820;
const EMERGENCY_NOTE: &str = "opsbox emergency ssh";

fn err(code: PrivErrorCode, msg: impl Into<String>) -> PrivError {
    PrivError {
        code,
        message: msg.into(),
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

// ------------------------------------------------------------------ validation

pub fn valid_proto(p: &str) -> bool {
    matches!(p, "tcp" | "udp")
}

/// `anywhere`, an address, or an address with a prefix length.
pub fn parse_src(src: &str) -> Result<Option<(IpAddr, u8)>, String> {
    let bad = || {
        "The source must be \"anywhere\" or an IP address / CIDR such as 203.0.113.0/24.".to_owned()
    };
    if src == "anywhere" {
        return Ok(None);
    }
    let (addr, prefix) = match src.split_once('/') {
        Some((a, p)) => (a, Some(p.parse::<u8>().map_err(|_| bad())?)),
        None => (src, None),
    };
    let ip: IpAddr = addr.parse().map_err(|_| bad())?;
    let max = if ip.is_ipv4() { 32 } else { 128 };
    let prefix = prefix.unwrap_or(max);
    if prefix > max {
        return Err(bad());
    }
    Ok(Some((ip, prefix)))
}

/// Does `src` (a rule's source) include `ip`?
pub fn covers(src: &str, ip: IpAddr) -> bool {
    match parse_src(src) {
        Ok(None) => true,
        Ok(Some((net, prefix))) => match (net, ip) {
            (IpAddr::V4(n), IpAddr::V4(i)) => mask_eq(
                u128::from(u32::from(n)),
                u128::from(u32::from(i)),
                prefix,
                32,
            ),
            (IpAddr::V6(n), IpAddr::V6(i)) => mask_eq(u128::from(n), u128::from(i), prefix, 128),
            _ => false,
        },
        Err(_) => false,
    }
}

fn mask_eq(a: u128, b: u128, prefix: u8, bits: u8) -> bool {
    if prefix == 0 {
        return true;
    }
    let shift = u32::from(bits - prefix);
    (a >> shift) == (b >> shift)
}

pub fn check_rule(r: &NewRule) -> Result<(), String> {
    if r.port == 0 {
        return Err("Port must be between 1 and 65535.".into());
    }
    if !valid_proto(&r.proto) {
        return Err("The protocol must be tcp or udp.".into());
    }
    parse_src(&r.src)?;
    if r.note.chars().count() > 100 || r.note.chars().any(char::is_control) {
        return Err("The note must be one line of at most 100 characters.".into());
    }
    Ok(())
}

/// Stable id for a rule, usable in a URL path: `tcp:5432:203.0.113.0_24`.
pub fn rule_id(proto: &str, port: u16, src: &str) -> String {
    format!("{proto}:{port}:{}", src.replace('/', "_"))
}

pub fn parse_rule_id(id: &str) -> Option<(String, u16, String)> {
    let mut it = id.splitn(3, ':');
    let (proto, port, src) = (
        it.next()?,
        it.next()?.parse::<u16>().ok()?,
        it.next()?.replace('_', "/"),
    );
    ((valid_proto(proto) || proto == "any") && !src.is_empty())
        .then(|| (proto.to_owned(), port, src))
}

// ------------------------------------------------------------ the lockout guard

/// Would removing `rule` leave `caller` unable to reach a protected port?
///
/// Only rules on a protected port matter, and only if no *other* rule on that port still
/// covers the caller. With an unknown caller address any such removal counts as risky.
pub fn removal_is_risky(
    rule: &FirewallRule,
    all: &[FirewallRule],
    caller: Option<IpAddr>,
    protected: &[u16],
) -> bool {
    if !protected.contains(&rule.port) || !matches!(rule.proto.as_str(), "tcp" | "any") {
        return false;
    }
    let Some(ip) = caller else { return true };
    if !covers(&rule.src, ip) {
        return false; // it never let the caller in
    }
    !all.iter().any(|o| {
        o.id != rule.id
            && o.port == rule.port
            && matches!(o.proto.as_str(), "tcp" | "any")
            && covers(&o.src, ip)
    })
}

// ------------------------------------------------------------------ nftables

/// The full `inet opsbox` ruleset for `rules`, to be loaded atomically.
pub fn render_nft(rules: &[FirewallRule], ssh_ports: &[u16], emergency: &[Emergency]) -> String {
    let mut s = String::from(
        "add table inet opsbox\ndelete table inet opsbox\ntable inet opsbox {\n  chain input {\n    type filter hook input priority 0; policy drop;\n    ct state established,related accept\n    ct state invalid drop\n    iifname \"lo\" accept\n    meta l4proto { icmp, ipv6-icmp } accept\n",
    );
    let ports: Vec<String> = ssh_ports.iter().map(u16::to_string).collect();
    if !ports.is_empty() {
        s.push_str(&format!(
            "    tcp dport {{ {} }} accept comment \"ssh\"\n",
            ports.join(", ")
        ));
    }
    s.push_str(&format!(
        "    udp dport {WIREGUARD_PORT} accept comment \"opsbox private network\"\n"
    ));
    let saddr = |src: &str| match parse_src(src) {
        Ok(Some((IpAddr::V4(_), _))) => format!("ip saddr {src} "),
        Ok(Some((IpAddr::V6(_), _))) => format!("ip6 saddr {src} "),
        _ => String::new(),
    };
    for r in rules.iter().filter(|r| valid_proto(&r.proto)) {
        let note: String = r
            .note
            .chars()
            .filter(|c| !c.is_control() && *c != '"' && *c != '\\')
            .collect();
        let comment = if note.is_empty() {
            String::new()
        } else {
            format!(" comment \"{note}\"")
        };
        s.push_str(&format!(
            "    {}{} dport {} accept{comment}\n",
            saddr(&r.src),
            r.proto,
            r.port
        ));
    }
    for e in emergency {
        s.push_str(&format!(
            "    {}tcp dport {} accept comment \"{EMERGENCY_NOTE}\"\n",
            saddr(&e.src),
            e.port
        ));
    }
    s.push_str("  }\n}\n");
    s
}

// --------------------------------------------------------------------- ufw

/// `ufw status`: `5432/tcp   ALLOW   203.0.113.0/24   # note`. IPv6 duplicates, ranges and
/// non-allow rules are skipped.
pub fn parse_ufw_status(text: &str) -> Vec<FirewallRule> {
    let mut out = Vec::new();
    for line in text.lines() {
        let (body, note) = match line.split_once('#') {
            Some((b, n)) => (b, n.trim().to_owned()),
            None => (line, String::new()),
        };
        if body.contains("(v6)") {
            continue;
        }
        let f: Vec<&str> = body.split_whitespace().collect();
        if f.len() < 3 || f[1] != "ALLOW" || (f.len() > 2 && f[2] == "IN" && f.len() < 4) {
            continue;
        }
        // "ALLOW IN" appears in `numbered`/verbose output; plain status has "ALLOW".
        let from_idx = if f.get(2) == Some(&"IN") { 3 } else { 2 };
        let src_raw = f[from_idx..].join(" ");
        let Some((port_s, proto)) = f[0]
            .split_once('/')
            .map(|(p, pr)| (p, pr.to_owned()))
            .or_else(|| Some((f[0], "any".to_owned())))
        else {
            continue;
        };
        let Ok(port) = port_s.parse::<u16>() else {
            continue;
        };
        let src = if src_raw == "Anywhere" {
            "anywhere".to_owned()
        } else {
            src_raw
        };
        if parse_src(&src).is_err() {
            continue;
        }
        let id = rule_id(&proto, port, &src);
        if !out.iter().any(|r: &FirewallRule| r.id == id) {
            out.push(FirewallRule {
                id,
                port,
                proto,
                src,
                note,
            });
        }
    }
    out
}

// --------------------------------------------------------------- persistent state

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Emergency {
    pub src: String,
    pub port: u16,
    pub until: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Pending {
    revert_at: i64,
    /// What to do to undo the change.
    undo: Undo,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
enum Undo {
    #[default]
    Nothing,
    /// Put this rule back.
    Add(NewRule),
    /// Take this rule away again.
    Remove(String),
    /// nftables: switch the whole firewall back to these rules (empty and `enabled: false` = off).
    NftRules {
        enabled: bool,
        rules: Vec<FirewallRule>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Saved {
    /// nftables backend only.
    nft_enabled: bool,
    nft_rules: Vec<FirewallRule>,
    emergency: Vec<Emergency>,
    pending: Option<Pending>,
}

static LOCK: Mutex<()> = Mutex::new(());

fn load() -> Saved {
    std::fs::read_to_string(STATE_FILE)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(s: &Saved) -> Result<(), PrivError> {
    super::pkg::write_atomic(
        STATE_FILE,
        &serde_json::to_string_pretty(s).unwrap_or_default(),
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(STATE_FILE, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

// ------------------------------------------------------------------ backends

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Backend {
    Ufw,
    Firewalld,
    Nft,
}

impl Backend {
    fn name(self) -> &'static str {
        match self {
            Self::Ufw => "ufw",
            Self::Firewalld => "firewalld",
            Self::Nft => "nftables",
        }
    }
}

const T: Duration = Duration::from_secs(30);

fn detect() -> Result<Backend, PrivError> {
    if let Some(ufw) = find(&["/usr/sbin/ufw", "/sbin/ufw"])
        && run(&ufw, &["status"], T)
            .is_some_and(|o| o.stdout.trim_start().starts_with("Status: active"))
    {
        return Ok(Backend::Ufw);
    }
    if let Some(fc) = find(&["/usr/bin/firewall-cmd", "/bin/firewall-cmd"])
        && run(&fc, &["--state"], T).is_some_and(|o| o.stdout.trim() == "running")
    {
        return Ok(Backend::Firewalld);
    }
    if find(&["/usr/sbin/nft", "/sbin/nft", "/usr/bin/nft"]).is_some() {
        return Ok(Backend::Nft);
    }
    Err(err(
        PrivErrorCode::Unsupported,
        "No firewall tool (ufw, firewalld or nft) was found on this machine.",
    ))
}

fn tool(candidates: &[&str]) -> Result<String, PrivError> {
    find(candidates).ok_or_else(|| {
        err(
            PrivErrorCode::Unsupported,
            "A required firewall tool is missing.",
        )
    })
}

/// SSH ports from sshd_config and its drop-ins; 22 if none are set.
pub fn parse_ssh_ports(configs: &[String]) -> Vec<u16> {
    let mut ports: Vec<u16> = configs
        .iter()
        .flat_map(|t| t.lines())
        .filter_map(|l| {
            let l = l.trim();
            let rest = l
                .strip_prefix("Port ")
                .or_else(|| l.strip_prefix("Port\t"))?;
            rest.trim().parse::<u16>().ok()
        })
        .collect();
    if ports.is_empty() {
        ports.push(22);
    }
    ports.sort_unstable();
    ports.dedup();
    ports
}

fn ssh_ports() -> Vec<u16> {
    let mut texts = vec![std::fs::read_to_string("/etc/ssh/sshd_config").unwrap_or_default()];
    if let Ok(dir) = std::fs::read_dir("/etc/ssh/sshd_config.d") {
        for e in dir.flatten() {
            if e.path().extension().is_some_and(|x| x == "conf")
                && let Ok(t) = std::fs::read_to_string(e.path())
            {
                texts.push(t);
            }
        }
    }
    parse_ssh_ports(&texts)
}

fn list_rules(b: Backend, saved: &Saved) -> Result<Vec<FirewallRule>, PrivError> {
    match b {
        Backend::Ufw => {
            let o = run(&tool(&["/usr/sbin/ufw", "/sbin/ufw"])?, &["status"], T)
                .ok_or_else(|| err(PrivErrorCode::Internal, "ufw did not answer"))?;
            Ok(parse_ufw_status(&o.stdout)
                .into_iter()
                .filter(|r| r.note != EMERGENCY_NOTE)
                .collect())
        }
        Backend::Firewalld => {
            let o = run(
                &tool(&["/usr/bin/firewall-cmd", "/bin/firewall-cmd"])?,
                &["--list-ports"],
                T,
            )
            .ok_or_else(|| err(PrivErrorCode::Internal, "firewalld did not answer"))?;
            let rich = run(
                &tool(&["/usr/bin/firewall-cmd", "/bin/firewall-cmd"])?,
                &["--list-rich-rules"],
                T,
            )
            .ok_or_else(|| err(PrivErrorCode::Internal, "firewalld did not answer"))?;
            Ok(o.stdout
                .split_whitespace()
                .filter_map(|t| {
                    let (p, proto) = t.split_once('/')?;
                    let port = p.parse::<u16>().ok()?;
                    valid_proto(proto).then(|| FirewallRule {
                        id: rule_id(proto, port, "anywhere"),
                        port,
                        proto: proto.to_owned(),
                        src: "anywhere".into(),
                        note: String::new(),
                    })
                })
                .chain(rich.stdout.lines().filter_map(parse_rich_rule))
                .collect())
        }
        Backend::Nft => Ok(if saved.nft_enabled {
            saved.nft_rules.clone()
        } else {
            Vec::new()
        }),
    }
}

fn apply_nft(saved: &Saved) -> Result<(), PrivError> {
    let nft = tool(&["/usr/sbin/nft", "/sbin/nft", "/usr/bin/nft"])?;
    if !saved.nft_enabled {
        // Switching off: remove our table (add first so deleting a missing table is not an error).
        run_with(
            &nft,
            &["-f", "-"],
            T,
            Some("add table inet opsbox\ndelete table inet opsbox\n"),
            &[],
        );
        return Ok(());
    }
    let script = render_nft(&saved.nft_rules, &ssh_ports(), &saved.emergency);
    // Check the whole ruleset first, so a mistake can never half-apply.
    let check = run_with(&nft, &["-c", "-f", "-"], T, Some(&script), &[])
        .ok_or_else(|| err(PrivErrorCode::Internal, "nft did not answer"))?;
    if !check.ok() {
        return Err(err(
            PrivErrorCode::Internal,
            format!(
                "The firewall rules were not accepted: {}",
                tail(&check.stderr, 2).join(" ")
            ),
        ));
    }
    let o = run_with(&nft, &["-f", "-"], T, Some(&script), &[])
        .ok_or_else(|| err(PrivErrorCode::Internal, "nft did not answer"))?;
    if o.ok() {
        Ok(())
    } else {
        Err(err(
            PrivErrorCode::Internal,
            format!(
                "The firewall rules could not be applied: {}",
                tail(&o.stderr, 2).join(" ")
            ),
        ))
    }
}

/// Each rich rule is one argv element; no shell sees the source string.
fn firewalld_change(port: u16, proto: &str, src: &str, add: bool) -> Result<String, PrivError> {
    check_rule(&NewRule {
        port,
        proto: proto.into(),
        src: src.into(),
        note: String::new(),
    })
    .map_err(|m| err(PrivErrorCode::Invalid, m))?;
    let verb = if add { "add" } else { "remove" };
    if src == "anywhere" {
        return Ok(format!("--{verb}-port={port}/{proto}"));
    }
    let family = if src.contains(':') { "ipv6" } else { "ipv4" };
    Ok(format!(
        "--{verb}-rich-rule=rule family=\"{family}\" source address=\"{src}\" port port=\"{port}\" protocol=\"{proto}\" accept"
    ))
}

fn parse_rich_rule(line: &str) -> Option<FirewallRule> {
    fn field<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
        line.split_once(prefix)?.1.split('"').next()
    }
    if !line.starts_with("rule family=") || !line.ends_with(" accept") {
        return None;
    }
    let src = field(line, "source address=\"")?;
    let port = field(line, "port port=\"")?.parse().ok()?;
    let proto = field(line, "protocol=\"")?;
    firewalld_change(port, proto, src, true).ok()?;
    Some(FirewallRule {
        id: rule_id(proto, port, src),
        port,
        proto: proto.into(),
        src: src.into(),
        note: String::new(),
    })
}

fn backend_add(b: Backend, r: &NewRule, saved: &mut Saved) -> Result<(), PrivError> {
    match b {
        Backend::Ufw => {
            let ufw = tool(&["/usr/sbin/ufw", "/sbin/ufw"])?;
            let src = if r.src == "anywhere" {
                "any"
            } else {
                r.src.as_str()
            };
            let port = r.port.to_string();
            let mut args = vec![
                "--force",
                "allow",
                "proto",
                r.proto.as_str(),
                "from",
                src,
                "to",
                "any",
                "port",
                port.as_str(),
            ];
            if !r.note.is_empty() {
                args.extend(["comment", r.note.as_str()]);
            }
            check_ok(run(&ufw, &args, T), "ufw")
        }
        Backend::Firewalld => {
            let fc = tool(&["/usr/bin/firewall-cmd", "/bin/firewall-cmd"])?;
            let spec = firewalld_change(r.port, &r.proto, &r.src, true)?;
            check_ok(run(&fc, &["--permanent", &spec], T), "firewalld")?;
            check_ok(run(&fc, &["--reload"], T), "firewalld")
        }
        Backend::Nft => {
            let id = rule_id(&r.proto, r.port, &r.src);
            saved.nft_rules.retain(|x| x.id != id);
            saved.nft_rules.push(FirewallRule {
                id,
                port: r.port,
                proto: r.proto.clone(),
                src: r.src.clone(),
                note: r.note.clone(),
            });
            saved.nft_enabled = true;
            apply_nft(saved)
        }
    }
}

fn backend_remove(b: Backend, rule: &FirewallRule, saved: &mut Saved) -> Result<(), PrivError> {
    match b {
        Backend::Ufw => {
            let ufw = tool(&["/usr/sbin/ufw", "/sbin/ufw"])?;
            let src = if rule.src == "anywhere" {
                "any"
            } else {
                rule.src.as_str()
            };
            let port = rule.port.to_string();
            let mut args = vec!["--force", "delete", "allow"];
            if rule.proto != "any" {
                args.extend(["proto", rule.proto.as_str()]);
            }
            args.extend(["from", src, "to", "any", "port", port.as_str()]);
            check_ok(run(&ufw, &args, T), "ufw")
        }
        Backend::Firewalld => {
            let fc = tool(&["/usr/bin/firewall-cmd", "/bin/firewall-cmd"])?;
            let spec = firewalld_change(rule.port, &rule.proto, &rule.src, false)?;
            check_ok(run(&fc, &["--permanent", &spec], T), "firewalld")?;
            check_ok(run(&fc, &["--reload"], T), "firewalld")
        }
        Backend::Nft => {
            saved.nft_rules.retain(|x| x.id != rule.id);
            apply_nft(saved)
        }
    }
}

fn check_ok(o: Option<super::exec::Output>, what: &str) -> Result<(), PrivError> {
    match o {
        Some(o) if o.ok() => Ok(()),
        Some(o) => Err(err(
            PrivErrorCode::Internal,
            format!(
                "{what} refused the change: {}",
                tail(&o.combined(), 2).join(" ")
            ),
        )),
        None => Err(err(
            PrivErrorCode::Internal,
            format!("{what} did not answer in time"),
        )),
    }
}

// ------------------------------------------------------------------ operations

pub(super) fn list() -> Result<FirewallState, PrivError> {
    let _g = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let b = detect()?;
    let saved = load();
    Ok(FirewallState {
        backend: b.name().into(),
        rules: list_rules(b, &saved)?,
        ssh_open_until: saved
            .emergency
            .iter()
            .map(|e| e.until)
            .filter(|u| *u > now())
            .max(),
        revert_at: saved.pending.as_ref().map(|p| p.revert_at),
        protected_ports: ssh_ports(),
    })
}

fn caller(ip: Option<&str>) -> Result<Option<IpAddr>, PrivError> {
    ip.map(|s| {
        s.parse::<IpAddr>()
            .map_err(|_| err(PrivErrorCode::Invalid, "The caller's address is not valid."))
    })
    .transpose()
}

pub(super) fn add(
    rule: &NewRule,
    caller_ip: Option<&str>,
    confirm: bool,
) -> Result<FirewallChange, PrivError> {
    check_rule(rule).map_err(|m| err(PrivErrorCode::Invalid, m))?;
    caller(caller_ip)?; // validated even though adding is rarely risky
    let _g = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let b = detect()?;
    let mut saved = load();
    let before = (saved.nft_enabled, saved.nft_rules.clone());
    // Turning the nftables firewall on for the first time blocks everything not allowed.
    let enabling = b == Backend::Nft && !saved.nft_enabled;
    if enabling && !confirm {
        return Err(err(
            PrivErrorCode::ConfirmRequired,
            "This turns the firewall on: everything that is not allowed is blocked. SSH and opsbox's private network stay open. If something goes wrong it switches itself off after 60 seconds unless you confirm. Continue?",
        ));
    }
    backend_add(b, rule, &mut saved)?;
    let revert_at = enabling.then(|| now() + REVERT_SECS);
    if let Some(at) = revert_at {
        saved.pending = Some(Pending {
            revert_at: at,
            undo: Undo::NftRules {
                enabled: before.0,
                rules: before.1,
            },
        });
    }
    // nftables rules live in our own state file; for ufw and firewalld only a pending
    // undo needs remembering.
    if b == Backend::Nft || revert_at.is_some() {
        save(&saved)?;
    }
    Ok(FirewallChange { revert_at })
}

pub(super) fn remove(
    id: &str,
    caller_ip: Option<&str>,
    confirm: bool,
) -> Result<FirewallChange, PrivError> {
    let ip = caller(caller_ip)?;
    let (_proto, _port, _src) =
        parse_rule_id(id).ok_or_else(|| err(PrivErrorCode::Invalid, "That is not a rule id."))?;
    let _g = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let b = detect()?;
    let mut saved = load();
    let rules = list_rules(b, &saved)?;
    let rule = rules
        .iter()
        .find(|r| r.id == id)
        .cloned()
        .ok_or_else(|| err(PrivErrorCode::NotFound, "There is no such rule."))?;
    let protected = ssh_ports();
    let risky = removal_is_risky(&rule, &rules, ip, &protected);
    if risky && !confirm {
        return Err(err(
            PrivErrorCode::ConfirmRequired,
            format!(
                "This rule is what lets you reach port {} (SSH). Removing it could lock you out of this machine. If you go ahead, it is put back after 60 seconds unless you confirm that you can still connect.",
                rule.port
            ),
        ));
    }
    let before = (saved.nft_enabled, saved.nft_rules.clone());
    backend_remove(b, &rule, &mut saved)?;
    let revert_at = risky.then(|| now() + REVERT_SECS);
    if let Some(at) = revert_at {
        let undo = if b == Backend::Nft {
            Undo::NftRules {
                enabled: before.0,
                rules: before.1,
            }
        } else {
            Undo::Add(NewRule {
                port: rule.port,
                proto: if rule.proto == "any" {
                    "tcp".into()
                } else {
                    rule.proto.clone()
                },
                src: rule.src.clone(),
                note: rule.note.clone(),
            })
        };
        saved.pending = Some(Pending {
            revert_at: at,
            undo,
        });
    }
    if b == Backend::Nft || revert_at.is_some() {
        save(&saved)?;
    }
    Ok(FirewallChange { revert_at })
}

pub(super) fn confirm() -> Result<(), PrivError> {
    let _g = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut saved = load();
    if saved.pending.take().is_none() {
        return Err(err(
            PrivErrorCode::NotFound,
            "There is no firewall change waiting for confirmation (it may already have been undone).",
        ));
    }
    save(&saved)
}

pub(super) fn ssh_emergency(caller_ip: &str) -> Result<i64, PrivError> {
    let ip: IpAddr = caller_ip.parse().map_err(|_| {
        err(
            PrivErrorCode::Invalid,
            "Could not tell which address you are connecting from, so SSH was not opened.",
        )
    })?;
    let _g = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // sshd must be running to be reachable.
    if let Some(ctl) = find(&["/usr/bin/systemctl", "/bin/systemctl"]) {
        let started = ["ssh.service", "sshd.service"]
            .iter()
            .any(|u| run(&ctl, &["start", u], T).is_some_and(|o| o.ok()));
        if !started {
            return Err(err(
                PrivErrorCode::Internal,
                "SSH is not installed on this machine, so there is nothing to open.",
            ));
        }
    }
    let port = ssh_ports()[0];
    let until = now() + EMERGENCY_SECS;
    let b = detect()?;
    let mut saved = load();
    saved.emergency.retain(|e| e.src != caller_ip);
    saved.emergency.push(Emergency {
        src: caller_ip.to_owned(),
        port,
        until,
    });
    match b {
        Backend::Nft => {
            if saved.nft_enabled {
                apply_nft(&saved)?;
            } // no firewall: SSH is already reachable
        }
        Backend::Ufw => {
            let ufw = tool(&["/usr/sbin/ufw", "/sbin/ufw"])?;
            let p = port.to_string();
            check_ok(
                run(
                    &ufw,
                    &[
                        "--force",
                        "allow",
                        "proto",
                        "tcp",
                        "from",
                        caller_ip,
                        "to",
                        "any",
                        "port",
                        &p,
                        "comment",
                        EMERGENCY_NOTE,
                    ],
                    T,
                ),
                "ufw",
            )?;
        }
        Backend::Firewalld => {
            let fc = tool(&["/usr/bin/firewall-cmd", "/bin/firewall-cmd"])?;
            let rich = format!(
                "rule family=\"{}\" source address=\"{ip}\" port port=\"{port}\" protocol=\"tcp\" accept",
                if ip.is_ipv4() { "ipv4" } else { "ipv6" }
            );
            check_ok(run(&fc, &["--add-rich-rule", &rich], T), "firewalld")?;
        }
    }
    save(&saved)?;
    Ok(until)
}

/// Called every few seconds: undo an unconfirmed change, close expired emergency entries.
pub fn janitor_tick() {
    if !Path::new(STATE_FILE).exists() {
        return;
    }
    let _g = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut saved = load();
    let mut changed = false;
    let t = now();
    if let Some(p) = saved.pending.clone()
        && p.revert_at <= t
    {
        tracing::warn!("firewall change was not confirmed in time: undoing it");
        if let Ok(b) = detect() {
            let result = match p.undo {
                Undo::Add(r) => backend_add(b, &r, &mut saved),
                Undo::Remove(id) => parse_rule_id(&id).map_or(Ok(()), |(proto, port, src)| {
                    backend_remove(
                        b,
                        &FirewallRule {
                            id,
                            port,
                            proto,
                            src,
                            note: String::new(),
                        },
                        &mut saved,
                    )
                }),
                Undo::NftRules { enabled, rules } => {
                    saved.nft_enabled = enabled;
                    saved.nft_rules = rules;
                    apply_nft(&saved)
                }
                Undo::Nothing => Ok(()),
            };
            if let Err(e) = result {
                tracing::error!(error = %e.message, "could not undo the firewall change");
            }
        }
        saved.pending = None;
        changed = true;
    }
    let expired: Vec<Emergency> = saved
        .emergency
        .iter()
        .filter(|e| e.until <= t)
        .cloned()
        .collect();
    if !expired.is_empty() {
        saved.emergency.retain(|e| e.until > t);
        if let Ok(b) = detect() {
            for e in &expired {
                match b {
                    Backend::Nft => {
                        if saved.nft_enabled {
                            let _ = apply_nft(&saved);
                        }
                    }
                    Backend::Ufw => {
                        if let Some(ufw) = find(&["/usr/sbin/ufw", "/sbin/ufw"]) {
                            let p = e.port.to_string();
                            run(
                                &ufw,
                                &[
                                    "--force", "delete", "allow", "proto", "tcp", "from", &e.src,
                                    "to", "any", "port", &p,
                                ],
                                T,
                            );
                        }
                    }
                    Backend::Firewalld => {
                        if let (Some(fc), Ok(ip)) = (
                            find(&["/usr/bin/firewall-cmd", "/bin/firewall-cmd"]),
                            e.src.parse::<IpAddr>(),
                        ) {
                            let rich = format!(
                                "rule family=\"{}\" source address=\"{ip}\" port port=\"{}\" protocol=\"tcp\" accept",
                                if ip.is_ipv4() { "ipv4" } else { "ipv6" },
                                e.port
                            );
                            run(&fc, &["--remove-rich-rule", &rich], T);
                        }
                    }
                }
            }
        }
        changed = true;
    }
    if changed {
        let _ = save(&saved);
    }
}

/// At start-up: put our nftables ruleset back (it does not survive a reboot by itself).
pub fn restore_on_start() {
    let saved = load();
    if saved.nft_enabled && matches!(detect(), Ok(Backend::Nft)) {
        let _ = apply_nft(&saved);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn restricted_firewalld_commands_round_trip() {
        for src in ["192.0.2.10", "192.0.2.0/24", "2001:db8::/64"] {
            let add = super::firewalld_change(8080, "tcp", src, true).unwrap();
            let remove = super::firewalld_change(8080, "tcp", src, false).unwrap();
            assert_eq!(add.replacen("--add-", "--remove-", 1), remove);
            let rule =
                super::parse_rich_rule(add.strip_prefix("--add-rich-rule=").unwrap()).unwrap();
            assert_eq!(rule.src, src);
            assert_eq!(rule.port, 8080);
        }
        assert!(super::firewalld_change(8080, "tcp", "x\" accept", true).is_err());
        assert_eq!(
            super::firewalld_change(22, "tcp", "anywhere", false).unwrap(),
            "--remove-port=22/tcp"
        );
    }

    use super::*;

    fn rule(proto: &str, port: u16, src: &str) -> FirewallRule {
        FirewallRule {
            id: rule_id(proto, port, src),
            port,
            proto: proto.into(),
            src: src.into(),
            note: String::new(),
        }
    }
    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn rules_are_validated() {
        let ok = NewRule {
            port: 5432,
            proto: "tcp".into(),
            src: "203.0.113.0/24".into(),
            note: "db".into(),
        };
        assert!(check_rule(&ok).is_ok());
        for bad in [
            NewRule {
                port: 0,
                ..ok.clone()
            },
            NewRule {
                proto: "icmp".into(),
                ..ok.clone()
            },
            NewRule {
                proto: "tcp; rm".into(),
                ..ok.clone()
            },
            NewRule {
                src: "10.0.0.0/33".into(),
                ..ok.clone()
            },
            NewRule {
                src: "not an ip".into(),
                ..ok.clone()
            },
            NewRule {
                src: "1.2.3.4; drop".into(),
                ..ok.clone()
            },
            NewRule {
                note: "a\nb".into(),
                ..ok.clone()
            },
            NewRule {
                note: "x".repeat(101),
                ..ok.clone()
            },
        ] {
            assert!(check_rule(&bad).is_err(), "{bad:?}");
        }
        assert!(
            check_rule(&NewRule {
                src: "anywhere".into(),
                ..ok.clone()
            })
            .is_ok()
        );
        assert!(
            check_rule(&NewRule {
                src: "2001:db8::/32".into(),
                ..ok
            })
            .is_ok()
        );
    }

    #[test]
    fn source_matching() {
        assert!(covers("anywhere", ip("1.2.3.4")));
        assert!(covers("203.0.113.0/24", ip("203.0.113.77")));
        assert!(!covers("203.0.113.0/24", ip("203.0.114.1")));
        assert!(
            covers("203.0.113.5", ip("203.0.113.5")) && !covers("203.0.113.5", ip("203.0.113.6"))
        );
        assert!(covers("0.0.0.0/0", ip("8.8.8.8")));
        assert!(
            covers("2001:db8::/32", ip("2001:db8:1::9"))
                && !covers("2001:db9::/32", ip("2001:db8::1"))
        );
        assert!(
            !covers("203.0.113.0/24", ip("2001:db8::1")),
            "families never match"
        );
        assert!(!covers("garbage", ip("1.2.3.4")));
    }

    #[test]
    fn ids_round_trip() {
        let id = rule_id("tcp", 5432, "203.0.113.0/24");
        assert_eq!(id, "tcp:5432:203.0.113.0_24");
        assert_eq!(
            parse_rule_id(&id),
            Some(("tcp".into(), 5432, "203.0.113.0/24".into()))
        );
        assert_eq!(
            parse_rule_id("any:80:anywhere"),
            Some(("any".into(), 80, "anywhere".into()))
        );
        assert_eq!(
            parse_rule_id("tcp:5432:2001:db8::1"),
            Some(("tcp".into(), 5432, "2001:db8::1".into())),
            "IPv6 colons survive"
        );
        for bad in ["", "tcp", "tcp:x:y", "evil:1:2", "tcp:70000:x", "tcp:1:"] {
            assert_eq!(parse_rule_id(bad), None, "{bad}");
        }
    }

    // ---- the lockout guard: the part that must never be wrong

    #[test]
    fn removing_the_only_rule_that_lets_the_caller_reach_ssh_is_risky() {
        let all = vec![rule("tcp", 22, "anywhere"), rule("tcp", 5432, "anywhere")];
        let caller = Some(ip("203.0.113.5"));
        assert!(removal_is_risky(&all[0], &all, caller, &[22]));
        assert!(
            !removal_is_risky(&all[1], &all, caller, &[22]),
            "a database port is not protected"
        );
    }

    #[test]
    fn it_is_safe_when_another_rule_still_covers_the_caller() {
        let all = vec![
            rule("tcp", 22, "anywhere"),
            rule("tcp", 22, "203.0.113.0/24"),
        ];
        let me = Some(ip("203.0.113.5"));
        assert!(
            !removal_is_risky(&all[0], &all, me, &[22]),
            "the /24 still lets me in"
        );
        assert!(
            !removal_is_risky(&all[1], &all, me, &[22]),
            "anywhere still lets me in"
        );
    }

    #[test]
    fn a_rule_that_never_covered_the_caller_is_safe_to_remove() {
        let all = vec![
            rule("tcp", 22, "198.51.100.0/24"),
            rule("tcp", 22, "203.0.113.5"),
        ];
        assert!(!removal_is_risky(
            &all[0],
            &all,
            Some(ip("203.0.113.5")),
            &[22]
        ));
        assert!(
            removal_is_risky(&all[1], &all, Some(ip("203.0.113.5")), &[22]),
            "the one that let me in"
        );
    }

    #[test]
    fn an_unknown_caller_counts_as_risky_on_protected_ports_only() {
        let all = vec![
            rule("tcp", 22, "203.0.113.5"),
            rule("tcp", 8080, "anywhere"),
        ];
        assert!(removal_is_risky(&all[0], &all, None, &[22]));
        assert!(!removal_is_risky(&all[1], &all, None, &[22]));
    }

    #[test]
    fn udp_rules_and_other_ports_do_not_matter_and_any_proto_counts_as_tcp() {
        assert!(!removal_is_risky(
            &rule("udp", 22, "anywhere"),
            &[rule("udp", 22, "anywhere")],
            Some(ip("1.1.1.1")),
            &[22]
        ));
        assert!(removal_is_risky(
            &rule("any", 22, "anywhere"),
            &[rule("any", 22, "anywhere")],
            Some(ip("1.1.1.1")),
            &[22]
        ));
        assert!(removal_is_risky(
            &rule("tcp", 2222, "anywhere"),
            &[rule("tcp", 2222, "anywhere")],
            Some(ip("1.1.1.1")),
            &[22, 2222]
        ));
        // IPv6 caller, IPv4-only rules: the rule never let them in.
        assert!(!removal_is_risky(
            &rule("tcp", 22, "203.0.113.5"),
            &[rule("tcp", 22, "203.0.113.5")],
            Some(ip("2001:db8::1")),
            &[22]
        ));
    }

    // ---- ruleset rendering

    #[test]
    fn the_nftables_ruleset_blocks_by_default_and_keeps_ssh_and_wireguard() {
        let mut r = rule("tcp", 5432, "203.0.113.0/24");
        r.note = "db \"quoted\"".into();
        let s = render_nft(
            &[
                r,
                rule("udp", 53, "anywhere"),
                rule("tcp", 443, "2001:db8::/32"),
            ],
            &[22],
            &[Emergency {
                src: "198.51.100.9".into(),
                port: 22,
                until: 1,
            }],
        );
        assert!(
            s.starts_with("add table inet opsbox\ndelete table inet opsbox\ntable inet opsbox {")
        );
        assert!(
            s.contains("policy drop;")
                && s.contains("ct state established,related accept")
                && s.contains("iifname \"lo\" accept")
        );
        assert!(s.contains("tcp dport { 22 } accept comment \"ssh\""), "{s}");
        assert!(s.contains("udp dport 51820 accept"));
        assert!(
            s.contains("ip saddr 203.0.113.0/24 tcp dport 5432 accept comment \"db quoted\""),
            "quotes are dropped from comments:\n{s}"
        );
        assert!(s.contains("    udp dport 53 accept\n"));
        assert!(s.contains("ip6 saddr 2001:db8::/32 tcp dport 443 accept"));
        assert!(s.contains(
            "ip saddr 198.51.100.9 tcp dport 22 accept comment \"opsbox emergency ssh\""
        ));
        assert_eq!(
            s.matches('{').count(),
            s.matches('}').count(),
            "balanced braces"
        );
    }

    #[test]
    fn rules_with_a_bad_protocol_never_reach_the_ruleset() {
        let mut bad = rule("tcp", 80, "anywhere");
        bad.proto = "tcp; flush ruleset".into();
        assert!(!render_nft(&[bad], &[22], &[]).contains("flush"));
    }

    // ---- ufw

    #[test]
    fn ufw_status_is_parsed() {
        let text = "Status: active\n\nTo                         Action      From\n--                         ------      ----\n22/tcp                     ALLOW       Anywhere\n5432/tcp                   ALLOW       203.0.113.0/24\n80                         ALLOW       Anywhere                   # web\n22/tcp (v6)                ALLOW       Anywhere (v6)\n6000:6007/tcp              ALLOW       Anywhere\n23/tcp                     DENY        1.2.3.4\n";
        let r = parse_ufw_status(text);
        assert_eq!(
            r.iter()
                .map(|x| (x.proto.as_str(), x.port, x.src.as_str(), x.note.as_str()))
                .collect::<Vec<_>>(),
            [
                ("tcp", 22, "anywhere", ""),
                ("tcp", 5432, "203.0.113.0/24", ""),
                ("any", 80, "anywhere", "web")
            ]
        );
        assert_eq!(r[0].id, "tcp:22:anywhere");
    }

    #[test]
    fn ssh_ports_come_from_the_config_with_a_default() {
        assert_eq!(parse_ssh_ports(&[]), [22]);
        assert_eq!(
            parse_ssh_ports(&["# Port 2200\nPermitRootLogin no\n".into()]),
            [22]
        );
        assert_eq!(
            parse_ssh_ports(&["Port 2222\n".into(), "  Port 22\nPort 2222\n".into()]),
            [22, 2222]
        );
        assert_eq!(parse_ssh_ports(&["Port notanumber\n".into()]), [22]);
    }

    #[test]
    fn invalid_arguments_are_refused_before_any_command_runs() {
        let bad = NewRule {
            port: 0,
            proto: "tcp".into(),
            src: "anywhere".into(),
            note: String::new(),
        };
        assert_eq!(
            add(&bad, None, false).unwrap_err().code,
            PrivErrorCode::Invalid
        );
        let ok = NewRule {
            port: 80,
            proto: "tcp".into(),
            src: "anywhere".into(),
            note: String::new(),
        };
        assert_eq!(
            add(&ok, Some("not-an-ip"), false).unwrap_err().code,
            PrivErrorCode::Invalid
        );
        assert_eq!(
            remove("garbage", None, false).unwrap_err().code,
            PrivErrorCode::Invalid
        );
        assert_eq!(
            ssh_emergency("not-an-ip").unwrap_err().code,
            PrivErrorCode::Invalid
        );
    }
}
