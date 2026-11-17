//! systemd services (docs/BACKEND_SPEC.md 7.8): list, start, stop, restart, enable,
//! disable, with a refusal list for units opsbox must not take down.

use std::collections::HashMap;
use std::time::Duration;

use opsbox_proto::{PrivError, PrivErrorCode, UnitActionKind, UnitInfo};

use super::exec::{find, run, tail};

fn err(code: PrivErrorCode, msg: impl Into<String>) -> PrivError {
    PrivError {
        code,
        message: msg.into(),
    }
}

fn systemctl() -> Result<String, PrivError> {
    find(&["/usr/bin/systemctl", "/bin/systemctl"]).ok_or_else(|| {
        err(
            PrivErrorCode::Unsupported,
            "systemd is not available on this machine.",
        )
    })
}

/// `ssh.service`, `docker.service`, `getty@tty1.service`, ...
pub fn valid_service_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".service") else {
        return false;
    };
    !stem.is_empty()
        && name.len() <= 256
        && !stem.starts_with(['-', '.'])
        && stem.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':' | b'@' | b'\\')
        })
}

/// Units that are never stopped, disabled or restarted from the panel.
/// Returns the reason, in words for a person.
pub fn refusal(name: &str, action: UnitActionKind) -> Option<String> {
    use UnitActionKind::{Disable, Enable, Restart, Start, Stop};
    if matches!(action, Start | Enable) {
        return None;
    }
    let base = name.trim_end_matches(".service");
    if base.starts_with("opsbox") {
        return Some(format!(
            "{name} is opsbox itself. Stopping or restarting it from the panel would cut you off; update or restart it from a shell or through an update."
        ));
    }
    if matches!(base, "ssh" | "sshd") && matches!(action, Stop | Disable) {
        return Some("SSH may be your only way back in if the panel stops working. It can be restarted but not stopped or disabled here.".into());
    }
    const NETWORK: [&str; 7] = [
        "systemd-networkd",
        "NetworkManager",
        "networking",
        "network",
        "systemd-resolved",
        "dbus",
        "systemd-logind",
    ];
    if NETWORK.contains(&base) && matches!(action, Stop | Disable | Restart) {
        return Some(format!(
            "{name} keeps this machine reachable or running. It cannot be stopped, disabled or restarted from the panel."
        ));
    }
    None
}

/// `systemctl list-units --type=service --all --output=json`
pub fn parse_units_json(text: &str) -> Vec<(String, String, String)> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(text)
    else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|u| {
            Some((
                u["unit"].as_str()?.to_owned(),
                u["active"].as_str()?.to_owned(),
                u["description"].as_str().unwrap_or("").to_owned(),
            ))
        })
        .collect()
}

/// Text form (older systemd): `unit load active sub description...`, with an optional
/// leading marker for failed units.
pub fn parse_units_text(text: &str) -> Vec<(String, String, String)> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim().trim_start_matches(['●', '*', '○']).trim_start();
            let mut f = l.split_whitespace();
            let (unit, _load, active, _sub) = (f.next()?, f.next()?, f.next()?, f.next()?);
            let desc = f.collect::<Vec<_>>().join(" ");
            unit.ends_with(".service")
                .then(|| (unit.to_owned(), active.to_owned(), desc))
        })
        .collect()
}

/// `systemctl list-unit-files --type=service --no-legend`: `ssh.service enabled enabled`.
pub fn parse_unit_files(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let (name, state) = (f.next()?, f.next()?);
            name.ends_with(".service")
                .then(|| (name.to_owned(), state.to_owned()))
        })
        .collect()
}

/// `systemctl show --type=service -p Id,MemoryCurrent`: blocks of `Key=Value`.
pub fn parse_memory(text: &str) -> HashMap<String, u64> {
    let mut out = HashMap::new();
    for block in text.split("\n\n") {
        let (mut id, mut mem) = (None, None);
        for line in block.lines() {
            match line.split_once('=') {
                Some(("Id", v)) => id = Some(v.to_owned()),
                Some(("MemoryCurrent", v)) => {
                    mem = v.parse::<u64>().ok().filter(|m| *m != u64::MAX)
                }
                _ => {}
            }
        }
        if let (Some(id), Some(m)) = (id, mem) {
            out.insert(id, m);
        }
    }
    out
}

pub(super) fn list() -> Result<Vec<UnitInfo>, PrivError> {
    let ctl = systemctl()?;
    let t = Duration::from_secs(20);
    let base = [
        "list-units",
        "--type=service",
        "--all",
        "--no-pager",
        "--plain",
    ];
    let mut args = base.to_vec();
    args.push("--output=json");
    let loaded = run(&ctl, &args, t)
        .filter(|o| o.ok())
        .map(|o| parse_units_json(&o.stdout))
        .filter(|v| !v.is_empty())
        .or_else(|| {
            let mut a = base.to_vec();
            a.push("--no-legend");
            run(&ctl, &a, t)
                .filter(|o| o.ok())
                .map(|o| parse_units_text(&o.stdout))
        })
        .ok_or_else(|| {
            err(
                PrivErrorCode::Internal,
                "Could not read the list of services from systemd.",
            )
        })?;
    let files = run(
        &ctl,
        &[
            "list-unit-files",
            "--type=service",
            "--no-legend",
            "--no-pager",
        ],
        t,
    )
    .map(|o| parse_unit_files(&o.stdout))
    .unwrap_or_default();
    let mem = run(
        &ctl,
        &[
            "show",
            "--type=service",
            "-p",
            "Id",
            "-p",
            "MemoryCurrent",
            "--no-pager",
        ],
        t,
    )
    .map(|o| parse_memory(&o.stdout))
    .unwrap_or_default();

    let mut out: Vec<UnitInfo> = loaded
        .into_iter()
        .filter(|(name, _, _)| !name.starts_with("opsbox-job-") && valid_service_name(name))
        .map(|(name, state, description)| {
            let boot = files.get(&name).cloned().unwrap_or_else(|| "static".into());
            UnitInfo {
                mem_bytes: mem.get(&name).copied(),
                name,
                state,
                boot,
                description,
            }
        })
        // Quiet, uninteresting ones (static helpers that are not running) stay out of the list.
        .filter(|u| u.state != "inactive" || u.boot == "enabled" || u.boot == "enabled-runtime")
        .collect();
    out.sort_by(|a, b| {
        let rank = |u: &UnitInfo| match u.state.as_str() {
            "failed" => 0,
            "active" | "activating" | "reloading" => 1,
            _ => 2,
        };
        rank(a).cmp(&rank(b)).then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

pub(super) fn action(name: &str, action: UnitActionKind) -> Result<(), PrivError> {
    if !valid_service_name(name) {
        return Err(err(
            PrivErrorCode::Invalid,
            "That is not a service name (expected something like nginx.service).",
        ));
    }
    if let Some(why) = refusal(name, action) {
        return Err(err(PrivErrorCode::Refused, why));
    }
    let ctl = systemctl()?;
    let o = run(
        &ctl,
        &[action.as_str(), "--no-pager", name],
        Duration::from_secs(90),
    )
    .ok_or_else(|| {
        err(
            PrivErrorCode::Internal,
            format!(
                "systemd did not finish the {} of {name} in time.",
                action.as_str()
            ),
        )
    })?;
    if o.ok() {
        Ok(())
    } else {
        let why = tail(&o.stderr, 2).join(" ");
        Err(err(
            PrivErrorCode::Internal,
            format!(
                "Could not {} {name}: {}",
                action.as_str(),
                if why.is_empty() {
                    "systemd refused"
                } else {
                    &why
                }
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use UnitActionKind::*;

    #[test]
    fn service_names_are_validated() {
        for ok in [
            "ssh.service",
            "docker.service",
            "getty@tty1.service",
            "systemd-networkd.service",
            "user@1000.service",
        ] {
            assert!(valid_service_name(ok), "{ok}");
        }
        for bad in [
            "",
            ".service",
            "ssh",
            "ssh.timer",
            "../x.service",
            "a b.service",
            "a;b.service",
            "-x.service",
            "a/b.service",
            &format!("{}.service", "a".repeat(260)),
            "x.service\n",
        ] {
            assert!(!valid_service_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_refusal_list() {
        for a in [Stop, Restart, Disable] {
            assert!(refusal("opsbox.service", a).is_some(), "{a:?}");
            assert!(refusal("opsbox-priv.service", a).is_some());
        }
        assert!(
            refusal("ssh.service", Stop)
                .unwrap()
                .contains("only way back")
        );
        assert!(refusal("sshd.service", Disable).is_some());
        assert!(
            refusal("ssh.service", Restart).is_none(),
            "restarting sshd is allowed"
        );
        for n in [
            "systemd-networkd.service",
            "NetworkManager.service",
            "networking.service",
            "dbus.service",
        ] {
            assert!(
                refusal(n, Stop).is_some() && refusal(n, Restart).is_some(),
                "{n}"
            );
        }
        assert!(refusal("nginx.service", Stop).is_none());
        assert!(
            refusal("opsbox.service", Start).is_none() && refusal("ssh.service", Enable).is_none(),
            "starting is always fine"
        );
        assert!(
            refusal("opsboxfoo.service", Stop).is_some(),
            "anything starting with opsbox"
        );
    }

    #[test]
    fn json_and_text_listings() {
        let json = r#"[{"unit":"ssh.service","load":"loaded","active":"active","sub":"running","description":"OpenBSD Secure Shell server"},{"unit":"snapd.service","load":"loaded","active":"failed","sub":"failed","description":"Snap Daemon"}]"#;
        assert_eq!(
            parse_units_json(json),
            [
                (
                    "ssh.service".into(),
                    "active".into(),
                    "OpenBSD Secure Shell server".into()
                ),
                (
                    "snapd.service".into(),
                    "failed".into(),
                    "Snap Daemon".into()
                )
            ]
        );
        assert!(parse_units_json("not json").is_empty());
        let text = "  ssh.service      loaded active running OpenBSD Secure Shell server\n● snapd.service    loaded failed failed  Snap Daemon\n  foo.timer loaded active waiting x\n";
        let t = parse_units_text(text);
        assert_eq!(t.len(), 2);
        assert_eq!(
            t[1],
            (
                "snapd.service".into(),
                "failed".into(),
                "Snap Daemon".into()
            )
        );
    }

    #[test]
    fn unit_files_and_memory() {
        let f = parse_unit_files(
            "ssh.service enabled enabled\ncron.service enabled enabled\nsnapd.service disabled enabled\napt-daily.service static -\nfoo.timer enabled enabled\n",
        );
        assert_eq!(f["ssh.service"], "enabled");
        assert_eq!(f["apt-daily.service"], "static");
        assert!(!f.contains_key("foo.timer"));
        let m = parse_memory(
            "Id=ssh.service\nMemoryCurrent=3145728\n\nId=idle.service\nMemoryCurrent=[not set]\n\nId=big.service\nMemoryCurrent=18446744073709551615\n\nId=cron.service\nMemoryCurrent=1048576\n",
        );
        assert_eq!(m.get("ssh.service"), Some(&3_145_728));
        assert_eq!(m.get("cron.service"), Some(&1_048_576));
        assert!(
            !m.contains_key("idle.service") && !m.contains_key("big.service"),
            "unset is not zero"
        );
    }

    #[test]
    fn actions_check_the_name_before_anything_else() {
        assert_eq!(
            action("../../etc/passwd", Start).unwrap_err().code,
            PrivErrorCode::Invalid
        );
        assert_eq!(
            action("ssh.service; reboot", Start).unwrap_err().code,
            PrivErrorCode::Invalid
        );
        assert_eq!(
            action("opsbox.service", Stop).unwrap_err().code,
            PrivErrorCode::Refused
        );
    }
}
