//! Persistent alert state machine and TLS-only outbound channels (spec 7.13).
pub mod channels;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const REPEAT_SECS: i64 = 4 * 3600;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub cond: String,
    pub level: String,
    pub on: bool,
    pub duration: i64,
}

pub fn default_rules() -> Vec<Rule> {
    [
        (
            "disk",
            "Disk space",
            "over 75% (critical at 95%) for 1 minute",
            "warning",
            60,
        ),
        (
            "cpu",
            "CPU",
            "over 90% (critical at 99%) for 5 minutes",
            "warning",
            300,
        ),
        (
            "memory",
            "Memory",
            "over 90% (critical at 95%) for 5 minutes",
            "warning",
            300,
        ),
        (
            "units",
            "Service down",
            "system service failed for 30 seconds",
            "critical",
            30,
        ),
        (
            "containers",
            "Container down",
            "stopped or restarting for 30 seconds",
            "critical",
            30,
        ),
        (
            "deploy",
            "Deploy failed",
            "latest deploy failed",
            "critical",
            0,
        ),
        (
            "backup",
            "Backup failed",
            "latest backup or restore test failed",
            "critical",
            0,
        ),
        (
            "missed",
            "Backup missed",
            "scheduled run overdue by 15 minutes",
            "warning",
            0,
        ),
        (
            "tls",
            "Certificate expiring",
            "expires within 14 days or renewal failed",
            "warning",
            0,
        ),
        (
            "security",
            "Security updates waiting",
            "updates waiting for 2 days",
            "warning",
            172800,
        ),
        (
            "signins",
            "Repeated failed sign-ins",
            "5 failures in 15 minutes",
            "critical",
            0,
        ),
        (
            "ssh",
            "Emergency SSH open",
            "emergency access is open",
            "critical",
            0,
        ),
        (
            "health",
            "opsbox unhealthy",
            "metrics unavailable for 1 minute",
            "critical",
            60,
        ),
    ]
    .into_iter()
    .map(|(id, name, cond, level, duration)| Rule {
        id: id.into(),
        name: name.into(),
        cond: cond.into(),
        level: level.into(),
        on: true,
        duration,
    })
    .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quiet {
    pub from: String,
    pub to: String,
}
impl Default for Quiet {
    fn default() -> Self {
        Self {
            from: "23:00".into(),
            to: "07:00".into(),
        }
    }
}
impl Quiet {
    pub fn minute(s: &str) -> Option<i64> {
        let (h, m) = s.split_once(':')?;
        if h.len() != 2 || m.len() != 2 {
            return None;
        }
        let h: i64 = h.parse().ok()?;
        let m: i64 = m.parse().ok()?;
        (h < 24 && m < 60 && h >= 0 && m >= 0).then_some(h * 60 + m)
    }
    pub fn contains(&self, now: i64) -> bool {
        let (Some(from), Some(to)) = (Self::minute(&self.from), Self::minute(&self.to)) else {
            return false;
        };
        let minute = now.rem_euclid(86400) / 60;
        if from < to {
            minute >= from && minute < to
        } else if from > to {
            minute >= from || minute < to
        } else {
            false
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Channel {
    pub id: String,
    /// All configuration, including secret webhook URLs, is sealed by priv.
    pub sealed: String,
    pub verified: bool,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Ok,
    Pending,
    Firing,
    Resolved,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Delivery {
    pub revision: u64,
    pub at: i64,
    pub attempt: Option<i64>,
    pub firing_sent: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Incident {
    pub id: String,
    pub rule: String,
    pub target: String,
    pub name: String,
    pub msg: String,
    pub link: String,
    pub level: String,
    pub state: State,
    pub since: i64,
    pub updated: i64,
    pub revision: u64,
    pub silence_until: i64,
    pub deliveries: BTreeMap<String, Delivery>,
}

#[derive(Clone, Debug)]
pub struct Observation {
    pub rule: String,
    pub target: String,
    pub msg: String,
    pub link: String,
    pub critical: bool,
}
impl Observation {
    pub fn new(rule: &str, target: &str, msg: String, link: &str, critical: bool) -> Self {
        Self {
            rule: rule.into(),
            target: target.into(),
            msg,
            link: link.into(),
            critical,
        }
    }
    pub fn key(&self) -> String {
        format!("{}:{}", self.rule, self.target)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct History {
    pub id: String,
    pub name: String,
    pub msg: String,
    pub state: State,
    pub ts: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Engine {
    pub rules: Vec<Rule>,
    pub quiet: Quiet,
    pub channels: BTreeMap<String, Channel>,
    pub incidents: BTreeMap<String, Incident>,
    pub history: Vec<History>,
}
impl Default for Engine {
    fn default() -> Self {
        Self {
            rules: default_rules(),
            quiet: Quiet::default(),
            channels: BTreeMap::new(),
            incidents: BTreeMap::new(),
            history: Vec::new(),
        }
    }
}
impl Engine {
    /// Only complete, successful source groups may resolve missing targets. Unknown
    /// sources preserve state, rather than announcing recovery during an outage.
    pub fn evaluate(&mut self, observations: Vec<Observation>, known: &BTreeSet<String>, now: i64) {
        let observed: BTreeMap<String, Observation> =
            observations.into_iter().map(|o| (o.key(), o)).collect();
        for rule in &self.rules {
            if !known.contains(&rule.id) {
                for i in self
                    .incidents
                    .values_mut()
                    .filter(|i| i.rule == rule.id && i.state == State::Pending)
                {
                    i.since = now;
                }
                continue;
            }
            if rule.on {
                for (key, o) in observed.iter().filter(|(_, o)| o.rule == rule.id) {
                    let i = self
                        .incidents
                        .entry(key.clone())
                        .or_insert_with(|| Incident {
                            id: key.clone(),
                            rule: rule.id.clone(),
                            target: o.target.clone(),
                            name: rule.name.clone(),
                            msg: o.msg.clone(),
                            link: o.link.clone(),
                            level: rule.level.clone(),
                            state: State::Ok,
                            since: now,
                            updated: now,
                            revision: 0,
                            silence_until: 0,
                            deliveries: BTreeMap::new(),
                        });
                    let old = i.state;
                    let old_level = i.level.clone();
                    if matches!(old, State::Ok | State::Resolved) {
                        i.since = now;
                        i.state = State::Pending;
                    }
                    i.msg = o.msg.clone();
                    i.link = o.link.clone();
                    i.level = if o.critical {
                        "critical".into()
                    } else {
                        rule.level.clone()
                    };
                    if i.state == State::Pending && now.saturating_sub(i.since) >= rule.duration {
                        i.state = State::Firing;
                    }
                    if i.state != old
                        || (i.state == State::Firing
                            && old_level != "critical"
                            && i.level == "critical")
                    {
                        i.updated = now;
                        i.revision += 1;
                        self.history.push(History {
                            id: i.id.clone(),
                            name: i.name.clone(),
                            msg: i.msg.clone(),
                            state: i.state,
                            ts: now,
                        });
                    }
                }
            }
            for i in self.incidents.values_mut().filter(|i| i.rule == rule.id) {
                if rule.on && observed.contains_key(&i.id) {
                    continue;
                }
                let next = match i.state {
                    State::Firing => State::Resolved,
                    State::Pending => State::Ok,
                    _ => continue,
                };
                i.state = next;
                i.updated = now;
                i.revision += 1;
                self.history.push(History {
                    id: i.id.clone(),
                    name: i.name.clone(),
                    msg: i.msg.clone(),
                    state: next,
                    ts: now,
                });
            }
        }
        if self.history.len() > 200 {
            self.history.drain(..self.history.len() - 200);
        }
        // Preserve unresolved deliveries for seven days; bound historical target churn.
        self.incidents.retain(|_, i| {
            matches!(i.state, State::Pending | State::Firing)
                || now.saturating_sub(i.updated) < 7 * 86400
        });
    }

    pub fn due(&self, id: &str, channel: &str, now: i64) -> bool {
        let Some(i) = self.incidents.get(id) else {
            return false;
        };
        if !matches!(i.state, State::Firing | State::Resolved)
            || now < i.silence_until
            || (i.level != "critical" && self.quiet.contains(now))
        {
            return false;
        }
        let d = i.deliveries.get(channel).cloned().unwrap_or_default();
        if d.attempt.is_some_and(|at| now.saturating_sub(at) < 60) {
            return false;
        }
        if i.state == State::Resolved && !d.firing_sent {
            return false;
        }
        i.revision != d.revision
            || (i.state == State::Firing && now.saturating_sub(d.at) >= REPEAT_SECS)
    }
    pub fn attempted(&mut self, id: &str, channel: &str, now: i64) {
        if let Some(i) = self.incidents.get_mut(id) {
            i.deliveries.entry(channel.into()).or_default().attempt = Some(now);
        }
    }
    pub fn delivered(&mut self, id: &str, channel: &str, now: i64) {
        if let Some(i) = self.incidents.get_mut(id) {
            i.deliveries.insert(
                channel.into(),
                Delivery {
                    revision: i.revision,
                    at: now,
                    attempt: None,
                    firing_sent: i.state == State::Firing,
                },
            );
        }
    }
}
