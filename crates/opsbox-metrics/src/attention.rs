//! The "needs attention" list (docs/BACKEND_SPEC.md 7.3): a fixed set of checks, each
//! with a sentence, and a link to where to fix it. Worded for people.

use opsbox_proto::SystemFacts;
use serde::Serialize;

use crate::engine::DiskView;

/// Disks fuller than this are listed.
pub const DISK_WARN_PCT: f64 = 75.0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Attention {
    /// `security`, `reboot`, `disk`, `units`, or `containers`.
    pub kind: &'static str,
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
            kind: "security",
            text: format!(
                "{} {verb} waiting{more}",
                plural(sec as usize, "security update", "security updates")
            ),
            link: link("Review and install →", "#/system"),
        });
    }
    if facts.reboot_required {
        out.push(Attention {
            kind: "reboot",
            text: "A reboot is needed to finish installing updates.".into(),
            link: link("Schedule reboot →", "#/system"),
        });
    }
    for d in disks.iter().filter(|d| d.pct() > DISK_WARN_PCT) {
        out.push(Attention {
            kind: "disk",
            text: format!("Disk {} is at {}%.", d.mount, d.pct().round()),
            link: link("Clean up →", "#/cleanup"),
        });
    }
    if !facts.failed_units.is_empty() {
        let names = facts
            .failed_units
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let rest = facts.failed_units.len().saturating_sub(3);
        let tail = if rest > 0 {
            format!(" and {rest} more")
        } else {
            String::new()
        };
        out.push(Attention {
            kind: "units",
            text: format!(
                "{} failed: {names}{tail}.",
                plural(facts.failed_units.len(), "service has", "services have")
            ),
            link: link("View services →", "#/system"),
        });
    }
    if !facts.restarting_containers.is_empty() {
        let names = facts
            .restarting_containers
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let rest = facts.restarting_containers.len().saturating_sub(3);
        let tail = if rest > 0 {
            format!(" and {rest} more")
        } else {
            String::new()
        };
        out.push(Attention {
            kind: "containers",
            text: format!(
                "{} restarting: {names}{tail}.",
                plural(
                    facts.restarting_containers.len(),
                    "container is",
                    "containers are"
                )
            ),
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
    fn each_condition_has_its_own_sentence_and_link() {
        let facts = SystemFacts {
            reboot_required: true,
            failed_units: vec!["snapd.service".into()],
            updates_total: Some(14),
            updates_security: Some(3),
            restarting_containers: vec!["worker-1".into()],
        };
        let items = compute(&facts, &[disk("/", 82), disk("/data", 10)]);
        let kinds: Vec<_> = items.iter().map(|a| a.kind).collect();
        assert_eq!(kinds, ["security", "reboot", "disk", "units", "containers"]);
        assert_eq!(
            items[0].text,
            "3 security updates are waiting, and 11 other updates"
        );
        assert_eq!(
            items[0].link,
            ("Review and install →".into(), "#/system".into())
        );
        assert_eq!(items[2].text, "Disk / is at 82%.");
        assert_eq!(items[2].link.1, "#/cleanup");
        assert_eq!(items[3].text, "1 service has failed: snapd.service.");
        assert_eq!(items[4].text, "1 container is restarting: worker-1.");
        assert_eq!(items[4].link.1, "#/containers");
    }

    #[test]
    fn wording_handles_singular_and_plural() {
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
        assert_eq!(
            compute(&many, &[])[0].text,
            "5 services have failed: u1.service, u2.service, u3.service and 2 more."
        );
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
