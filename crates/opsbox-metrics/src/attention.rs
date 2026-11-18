//! The "needs attention" list (docs/BACKEND_SPEC.md 7.3): a fixed set of checks, each
//! with a stable id, severity, timestamp, sentence, and a link to where to fix it. Worded for people.

use opsbox_proto::SystemFacts;
use serde::{Deserialize, Serialize};

use crate::engine::DiskView;

/// Disks fuller than this are listed.
pub const DISK_WARN_PCT: f64 = 75.0;
/// Disks at or above this percentage are critical.
pub const DISK_CRIT_PCT: f64 = 95.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

impl Severity {
    pub fn rank(self) -> i64 {
        match self {
            Self::Info => 0,
            Self::Warning => 1,
            Self::Critical => 2,
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "critical" => Self::Critical,
            "info" => Self::Info,
            _ => Self::Warning,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Attention {
    pub id: String,
    /// `security`, `reboot`, `disk`, `units`, `containers`, `update`, `deploy`, `tls`, `backup`, etc.
    pub kind: String,
    pub severity: Severity,
    pub since: i64,
    pub text: String,
    /// (label, route) in the UI's own terms.
    pub link: (String, String),
}

fn link(label: &str, route: &str) -> (String, String) {
    (label.to_owned(), route.to_owned())
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

pub fn compute(facts: &SystemFacts, disks: &[DiskView]) -> Vec<Attention> {
    compute_at(facts, disks, 0)
}

pub fn compute_at(facts: &SystemFacts, disks: &[DiskView], now: i64) -> Vec<Attention> {
    let mut out = Vec::new();

    if let Some(sec) = facts.updates_security.filter(|n| *n > 0) {
        let other = facts.updates_total.unwrap_or(sec).saturating_sub(sec);
        let more = if other > 0 {
            let noun = if other == 1 { "update" } else { "updates" };
            format!(", and {other} other {noun}")
        } else {
            String::new()
        };
        let verb = if sec == 1 { "is" } else { "are" };
        out.push(Attention {
            id: "security:host".into(),
            kind: "security".into(),
            severity: Severity::Warning,
            since: now,
            text: format!(
                "{} {verb} waiting{more}",
                plural(sec as usize, "security update", "security updates")
            ),
            link: link("Review and install →", "#/system"),
        });
    }
    if facts.reboot_required {
        out.push(Attention {
            id: "reboot:host".into(),
            kind: "reboot".into(),
            severity: Severity::Warning,
            since: now,
            text: "A reboot is needed to finish installing updates.".into(),
            link: link("Schedule reboot →", "#/system"),
        });
    }
    for d in disks.iter().filter(|d| d.pct() > DISK_WARN_PCT) {
        out.push(Attention {
            id: format!("disk:{}", d.mount),
            kind: "disk".into(),
            severity: if d.pct() >= DISK_CRIT_PCT {
                Severity::Critical
            } else {
                Severity::Warning
            },
            since: now,
            text: format!("Disk {} is at {}%.", d.mount, d.pct().round()),
            link: link("Clean up →", "#/cleanup"),
        });
    }
    for name in &facts.failed_units {
        out.push(Attention {
            id: format!("units:{name}"),
            kind: "units".into(),
            severity: Severity::Critical,
            since: now,
            text: format!("Service {name} has failed."),
            link: link("View services →", "#/system"),
        });
    }
    for name in &facts.restarting_containers {
        out.push(Attention {
            id: format!("containers:{name}"),
            kind: "containers".into(),
            severity: Severity::Critical,
            since: now,
            text: format!("Container {name} is restarting."),
            link: link("View containers →", "#/containers"),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(mount: &str, used: u64) -> DiskView {
        DiskView {
            mount: mount.into(),
            fstype: "ext4".into(),
            used,
            total: 100,
        }
    }

    #[test]
    fn a_healthy_machine_has_nothing_to_say() {
        assert!(compute(&SystemFacts::default(), &[disk("/", 40)]).is_empty());
        assert!(
            compute(&SystemFacts::default(), &[disk("/", 75)]).is_empty(),
            "exactly 75 % is not over the line"
        );
    }

    #[test]
    fn each_condition_has_its_own_id_severity_sentence_and_link() {
        let facts = SystemFacts {
            reboot_required: true,
            failed_units: vec!["snapd.service".into()],
            updates_total: Some(14),
            updates_security: Some(3),
            restarting_containers: vec!["worker-1".into()],
        };
        let items = compute_at(&facts, &[disk("/", 82), disk("/data", 96)], 1_700_000_000);
        let ids: Vec<_> = items.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "security:host",
                "reboot:host",
                "disk:/",
                "disk:/data",
                "units:snapd.service",
                "containers:worker-1"
            ]
        );
        assert_eq!(items[0].severity, Severity::Warning);
        assert_eq!(
            items[0].text,
            "3 security updates are waiting, and 11 other updates"
        );
        assert_eq!(
            items[0].link,
            ("Review and install →".into(), "#/system".into())
        );
        assert_eq!(items[2].severity, Severity::Warning);
        assert_eq!(items[2].text, "Disk / is at 82%.");
        assert_eq!(items[2].link.1, "#/cleanup");
        assert_eq!(items[3].severity, Severity::Critical);
        assert_eq!(items[3].text, "Disk /data is at 96%.");
        assert_eq!(items[4].severity, Severity::Critical);
        assert_eq!(items[4].text, "Service snapd.service has failed.");
        assert_eq!(items[5].severity, Severity::Critical);
        assert_eq!(items[5].text, "Container worker-1 is restarting.");
        assert_eq!(items[5].link.1, "#/containers");
        assert!(items.iter().all(|a| a.since == 1_700_000_000));
    }

    #[test]
    fn wording_handles_singular_and_plural_and_multiple_units_by_id() {
        let one = SystemFacts {
            updates_security: Some(1),
            updates_total: Some(1),
            ..SystemFacts::default()
        };
        assert_eq!(compute(&one, &[])[0].text, "1 security update is waiting");
        let many = SystemFacts {
            failed_units: (1..=5).map(|i| format!("u{i}.service")).collect(),
            ..SystemFacts::default()
        };
        let items = compute(&many, &[]);
        assert_eq!(items.len(), 5);
        assert_eq!(items[0].id, "units:u1.service");
        assert_eq!(items[4].id, "units:u5.service");
    }

    #[test]
    fn unknown_update_counts_are_not_reported_as_zero_or_as_a_problem() {
        let facts = SystemFacts {
            updates_security: None,
            updates_total: None,
            ..SystemFacts::default()
        };
        assert!(compute(&facts, &[]).is_empty());
    }
}
