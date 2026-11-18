//! Pure admission policy. Host IO belongs exclusively to opsbox-priv.
pub mod requirements;
use requirements::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    Install,
    Docker,
    #[default]
    App,
    Build,
    Database,
    Template,
    Backup,
    ImagePull,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disk {
    pub path: String,
    pub total_mb: u64,
    pub free_mb: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Profile {
    pub memory_mb: u64,
    pub available_mb: u64,
    pub swap_mb: u64,
    pub swap_free_mb: u64,
    pub zram: bool,
    pub online_cpus: u64,
    pub cpu_millis: u64,
    pub cgroup_v2: bool,
    pub docker: bool,
    pub docker_data_root: Option<String>,
    pub disks: Vec<Disk>,
    /// Hundredths of a percent, avoiding float ambiguity on the wire.
    pub memory_psi_avg60: Option<u64>,
    pub cpu_psi_avg60: Option<u64>,
    pub pressure_seconds: u64,
    pub below_minimum: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reservation {
    pub name: String,
    pub memory_mb: u64,
    pub cpu_millis: u64,
    pub database: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub profile: Profile,
    pub reservations: Vec<Reservation>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Request {
    pub feature: Feature,
    pub database: bool,
    pub memory_mb: u64,
    pub recommended_memory_mb: u64,
    pub min_memory_mb: u64,
    pub cpu_millis: u64,
    pub disk_mb: u64,
    /// A replaced reservation is removed exactly once. Deploys keep old+new until healthy.
    pub replacing: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Fit {
    Ok,
    Tight {
        reasons: Vec<String>,
    },
    Blocked {
        reasons: Vec<String>,
        suggestions: Vec<String>,
    },
}
impl Fit {
    pub fn blocked(&self) -> bool {
        matches!(self, Self::Blocked { .. })
    }
    pub fn reasons(&self) -> &[String] {
        match self {
            Self::Ok => &[],
            Self::Tight { reasons } | Self::Blocked { reasons, .. } => reasons,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    pub os_mb: u64,
    pub opsbox_mb: u64,
    pub docker_mb: u64,
    pub reserved_mb: u64,
    pub swap_credit_mb: u64,
    pub allocatable_mb: u64,
    pub database_allocatable_mb: u64,
    pub allocatable_cpu_millis: u64,
    pub pressure: bool,
}
impl Snapshot {
    pub fn budget(&self, replacing: Option<&str>) -> Budget {
        let p = &self.profile;
        let mut replaced = false;
        let used: Vec<_> = self
            .reservations
            .iter()
            .filter(|r| {
                if !replaced && Some(r.name.as_str()) == replacing {
                    replaced = true;
                    false
                } else {
                    true
                }
            })
            .collect();
        let reserved_mb = used.iter().fold(0u64, |n, r| n.saturating_add(r.memory_mb));
        let cpu = used
            .iter()
            .fold(0u64, |n, r| n.saturating_add(r.cpu_millis));
        let os = os_mb(p.memory_mb);
        let docker = if p.docker { DOCKER_MB } else { 0 };
        let physical = p.memory_mb.saturating_sub(
            os.saturating_add(OPSBOX_MB)
                .saturating_add(docker)
                .saturating_add(reserved_mb),
        );
        let swap = p.swap_mb.min(p.swap_free_mb) / 2;
        Budget {
            os_mb: os,
            opsbox_mb: OPSBOX_MB,
            docker_mb: docker,
            reserved_mb,
            swap_credit_mb: swap,
            allocatable_mb: p.memory_mb.saturating_add(swap).saturating_sub(
                os.saturating_add(OPSBOX_MB)
                    .saturating_add(docker)
                    .saturating_add(reserved_mb),
            ),
            database_allocatable_mb: physical,
            allocatable_cpu_millis: p
                .cpu_millis
                .saturating_sub(RESERVED_CPU_MILLIS)
                .saturating_mul(2)
                .saturating_sub(cpu),
            pressure: p.available_mb < os
                || (p.memory_psi_avg60.unwrap_or(0) > 2000
                    && p.pressure_seconds >= PRESSURE_SECONDS),
        }
    }
    pub fn admit(&self, request: &Request) -> Fit {
        admit(self, request)
    }
    pub fn over_capacity(&self) -> bool {
        let b = self.budget(None);
        let cpu = self
            .reservations
            .iter()
            .fold(0u64, |n, r| n.saturating_add(r.cpu_millis));
        b.pressure
            || b.os_mb
                .saturating_add(b.opsbox_mb)
                .saturating_add(b.docker_mb)
                .saturating_add(b.reserved_mb)
                > self.profile.memory_mb.saturating_add(b.swap_credit_mb)
            || cpu
                > self
                    .profile
                    .cpu_millis
                    .saturating_sub(RESERVED_CPU_MILLIS)
                    .saturating_mul(2)
            || self
                .profile
                .disks
                .iter()
                .any(|d| d.free_mb < disk_mb(d.total_mb))
    }
    pub fn swap_offer_mb(&self) -> Option<u64> {
        (self.profile.memory_mb <= 2048 && self.profile.swap_mb == 0).then(|| {
            if self
                .profile
                .disks
                .first()
                .is_some_and(|d| d.free_mb > 20 * 1024)
            {
                2048
            } else {
                1024
            }
        })
    }
}
/// The single policy used by every entry point, including dry runs.
pub fn admit(snapshot: &Snapshot, request: &Request) -> Fit {
    let p = &snapshot.profile;
    let b = snapshot.budget(request.replacing.as_deref());
    let (_, min, recommended, disk) = FEATURES
        .iter()
        .find(|r| r.0 == request.feature)
        .expect("all features have requirements");
    let minimum = (*min).max(request.min_memory_mb);
    let recommended = (*recommended)
        .max(request.recommended_memory_mb)
        .max(minimum);
    let total = matches!(request.feature, Feature::Install | Feature::Docker);
    let needed = request.memory_mb.max(minimum);
    let free = if total {
        p.memory_mb
    } else if request.feature == Feature::Database || request.database {
        b.database_allocatable_mb
            .min(p.available_mb.saturating_sub(b.os_mb))
    } else {
        b.allocatable_mb.min(
            p.available_mb
                .saturating_sub(b.os_mb)
                .saturating_add(b.swap_credit_mb),
        )
    };
    let mut reasons = Vec::new();
    let mut warnings = Vec::new();
    if b.pressure && !total {
        reasons.push(
            "The server is under memory pressure right now. Wait for memory use to fall.".into(),
        );
    }
    if !total && !matches!(request.feature, Feature::Backup | Feature::ImagePull) && !p.cgroup_v2 {
        reasons.push("Cgroup v2 is required to enforce workload memory limits.".into());
    }
    if free < needed || (request.memory_mb > 0 && request.memory_mb < minimum) {
        reasons.push(format!("Needs {needed} MB, {free} MB allocatable ({} MB RAM, {} MB OS, {} MB opsbox, {} MB Docker, {} MB workloads, {} MB swap credit); minimum limit {minimum} MB.", p.memory_mb,b.os_mb,b.opsbox_mb,b.docker_mb,b.reserved_mb,if request.feature == Feature::Database || request.database {0} else {b.swap_credit_mb}));
    } else if free < recommended || (request.memory_mb > 0 && request.memory_mb < recommended) {
        warnings.push(format!(
            "Fits, but tight: {free} MB available; {recommended} MB recommended."
        ));
    }
    if !total && request.cpu_millis > b.allocatable_cpu_millis {
        reasons.push(format!(
            "Needs {}m CPU, {}m available with 2x overcommit.",
            request.cpu_millis, b.allocatable_cpu_millis
        ));
    }
    for d in &p.disks {
        let reserve = disk_mb(d.total_mb);
        let required = if request.feature == Feature::Install {
            *disk
        } else {
            reserve.saturating_add((*disk).max(request.disk_mb))
        };
        if d.free_mb < required {
            reasons.push(format!(
                "{} needs {required} MB free including {reserve} MB disk reserve; {} MB free.",
                d.path, d.free_mb
            ));
        }
    }
    if !reasons.is_empty() {
        let mut suggestions = vec!["Lower the memory limit, or upgrade the instance.".into()];
        if let Some(r) = snapshot.reservations.iter().max_by_key(|r| r.memory_mb) {
            suggestions.push(format!(
                "Stop or remove {} to free {} MB.",
                r.name, r.memory_mb
            ));
        }
        if snapshot.swap_offer_mb().is_some() {
            suggestions.push(
                "Add a swap file for apps and builds; swap never counts for databases.".into(),
            );
        }
        if request.feature == Feature::Build {
            suggestions.push("Build in GitHub Actions and pull the image instead.".into());
        }
        Fit::Blocked {
            reasons,
            suggestions,
        }
    } else if warnings.is_empty() {
        Fit::Ok
    } else {
        Fit::Tight { reasons: warnings }
    }
}
/// PSI `some avg60` in hundredths of a percent. Missing/invalid data is unknown.
pub fn parse_psi(text: &str) -> Option<u64> {
    text.lines()
        .find(|l| l.starts_with("some "))?
        .split_whitespace()
        .find_map(|f| {
            let v = f.strip_prefix("avg60=")?.parse::<f64>().ok()?;
            (v.is_finite() && (0.0..=100.0).contains(&v)).then_some((v * 100.0).round() as u64)
        })
}
pub fn meminfo_mb(text: &str, field: &str) -> u64 {
    text.lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k == field)
                .then(|| v.split_whitespace().next()?.parse::<u64>().ok())
                .flatten()
        })
        .unwrap_or(0)
        / 1024
}
