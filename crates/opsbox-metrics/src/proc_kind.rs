//! Classify top processes from `/proc/<pid>/cgroup` (cgroup v2).
//!
//! Reads happen only for the top-N list after sorting, and results are cached by
//! PID until that process leaves the list (exited or fell out of the top set).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Where a process is running from the host's point of view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProcKind {
    Container,
    System,
    User,
    #[default]
    Unknown,
}

/// Docker container linked to a process via its cgroup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProcContainer {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
}

/// One entry from the Docker list used to resolve container ids to names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerIndexEntry {
    pub id: String,
    pub full_id: String,
    pub name: String,
    pub service: Option<String>,
}

/// Result of parsing a cgroup file (before Docker name lookup).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Classified {
    Container { id: String },
    System { unit: String },
    User,
    Unknown,
}

/// Cached resolution for a PID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedKind {
    pub kind: ProcKind,
    pub container: Option<ProcContainer>,
    pub unit: Option<String>,
}

/// PID → kind, kept until the process exits (or drops out of the top list).
#[derive(Debug, Default)]
pub struct ProcKindCache {
    by_pid: HashMap<u32, ResolvedKind>,
    /// Override for tests; production uses `/proc/<pid>/cgroup`.
    proc_root: Option<PathBuf>,
}

impl ProcKindCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read cgroup files under `root/<pid>/cgroup` instead of `/proc`.
    pub fn with_proc_root(root: impl Into<PathBuf>) -> Self {
        Self {
            by_pid: HashMap::new(),
            proc_root: Some(root.into()),
        }
    }

    /// Drop cache entries for PIDs that are no longer in the live top list.
    pub fn retain(&mut self, live: &[u32]) {
        self.by_pid.retain(|pid, _| live.contains(pid));
    }

    /// Resolve `pid`, reading cgroup only on a cache miss.
    pub fn resolve(&mut self, pid: u32, docker: &[DockerIndexEntry]) -> ResolvedKind {
        if let Some(cached) = self.by_pid.get(&pid) {
            // Refresh container name/service from the latest Docker list without
            // re-reading cgroup (the id is stable for the life of the PID).
            if cached.kind == ProcKind::Container
                && let Some(id) = cached.container.as_ref().map(|c| c.id.clone())
            {
                let refreshed = lookup_container(&id, docker).or_else(|| cached.container.clone());
                let out = ResolvedKind {
                    kind: ProcKind::Container,
                    container: refreshed,
                    unit: None,
                };
                self.by_pid.insert(pid, out.clone());
                return out;
            }
            return cached.clone();
        }
        let text = read_cgroup(self.proc_root.as_deref(), pid);
        let resolved = resolve_text(text.as_deref(), docker);
        self.by_pid.insert(pid, resolved.clone());
        resolved
    }
}

fn read_cgroup(proc_root: Option<&Path>, pid: u32) -> Option<String> {
    let path = match proc_root {
        Some(root) => root.join(pid.to_string()).join("cgroup"),
        None => PathBuf::from(format!("/proc/{pid}/cgroup")),
    };
    std::fs::read_to_string(path).ok()
}

fn resolve_text(text: Option<&str>, docker: &[DockerIndexEntry]) -> ResolvedKind {
    match text.map(classify_cgroup).unwrap_or(Classified::Unknown) {
        Classified::Container { id } => ResolvedKind {
            kind: ProcKind::Container,
            container: lookup_container(&id, docker).or(Some(ProcContainer {
                id: short_id(&id),
                name: short_id(&id),
                service: None,
            })),
            unit: None,
        },
        Classified::System { unit } => ResolvedKind {
            kind: ProcKind::System,
            container: None,
            unit: Some(unit),
        },
        Classified::User => ResolvedKind {
            kind: ProcKind::User,
            container: None,
            unit: None,
        },
        Classified::Unknown => ResolvedKind {
            kind: ProcKind::Unknown,
            container: None,
            unit: None,
        },
    }
}

/// Parse a `/proc/<pid>/cgroup` file. Pure; used by tests with fixtures.
pub fn classify_cgroup(text: &str) -> Classified {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return Classified::Unknown;
    }
    // cgroup v2 uses a single unified hierarchy line `0::/path`.
    // Anything else (classic v1 multi-controller) is reported as unknown.
    let v2 = lines.iter().any(|l| l.starts_with("0::"));
    if !v2 {
        return Classified::Unknown;
    }
    let paths: Vec<&str> = lines
        .iter()
        .filter_map(|line| line.splitn(3, ':').nth(2))
        .collect();
    for path in &paths {
        if let Some(id) = extract_docker_id(path) {
            return Classified::Container { id };
        }
    }
    for path in &paths {
        if let Some(unit) = extract_system_unit(path) {
            return Classified::System { unit };
        }
    }
    Classified::User
}

fn extract_docker_id(path: &str) -> Option<String> {
    for part in path.split('/') {
        if let Some(rest) = part.strip_prefix("docker-")
            && let Some(id) = rest.strip_suffix(".scope")
            && is_container_id(id)
        {
            return Some(id.to_ascii_lowercase());
        }
    }
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    for window in parts.windows(2) {
        if window[0] == "docker" && is_container_id(window[1]) {
            return Some(window[1].to_ascii_lowercase());
        }
    }
    None
}

fn extract_system_unit(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.contains(&"user.slice") {
        return None;
    }
    if !parts.contains(&"system.slice") {
        return None;
    }
    // A docker scope under system.slice is a container, already handled above.
    if parts
        .iter()
        .any(|p| p.starts_with("docker-") && p.ends_with(".scope"))
    {
        return None;
    }
    let last = *parts.last()?;
    if is_service_unit(last) {
        Some(last.to_owned())
    } else {
        None
    }
}

fn is_container_id(id: &str) -> bool {
    let len = id.len();
    (12..=64).contains(&len) && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_service_unit(unit: &str) -> bool {
    unit.ends_with(".service")
        && unit.len() <= 128
        && !unit.starts_with('-')
        && unit
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.@:-".contains(&c))
}

fn short_id(id: &str) -> String {
    id.chars().take(12).collect()
}

fn lookup_container(cgroup_id: &str, docker: &[DockerIndexEntry]) -> Option<ProcContainer> {
    let needle = cgroup_id.to_ascii_lowercase();
    docker.iter().find_map(|c| {
        let full = c.full_id.to_ascii_lowercase();
        let short = c.id.to_ascii_lowercase();
        let hit = full == needle
            || short == needle
            || full.starts_with(&needle)
            || needle.starts_with(&full)
            || needle.starts_with(&short);
        hit.then(|| ProcContainer {
            id: if c.id.is_empty() {
                short_id(&c.full_id)
            } else {
                c.id.clone()
            },
            name: c.name.clone(),
            service: c.service.clone(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_systemd_scope() {
        let id = "a".repeat(64);
        let text = format!("0::/system.slice/docker-{id}.scope\n");
        assert_eq!(
            classify_cgroup(&text),
            Classified::Container { id: id.clone() }
        );
        // Nested under docker.service (common on Ubuntu).
        let text = format!("0::/system.slice/docker.service/docker-{id}.scope\n");
        assert_eq!(classify_cgroup(&text), Classified::Container { id });
    }

    #[test]
    fn docker_cgroupfs_driver() {
        let id = "b".repeat(64);
        let text = format!("0::/docker/{id}\n");
        assert_eq!(
            classify_cgroup(&text),
            Classified::Container { id: id.clone() }
        );
        // Short id form also accepted.
        let short = "b".repeat(12);
        assert_eq!(
            classify_cgroup(&format!("0::/docker/{short}\n")),
            Classified::Container { id: short }
        );
    }

    #[test]
    fn systemd_system_service() {
        assert_eq!(
            classify_cgroup("0::/system.slice/nginx.service\n"),
            Classified::System {
                unit: "nginx.service".into()
            }
        );
        assert_eq!(
            classify_cgroup("0::/system.slice/docker.service\n"),
            Classified::System {
                unit: "docker.service".into()
            }
        );
    }

    #[test]
    fn user_session_is_user() {
        assert_eq!(
            classify_cgroup("0::/user.slice/user-1000.slice/session-3.scope\n"),
            Classified::User
        );
        assert_eq!(
            classify_cgroup(
                "0::/user.slice/user-1000.slice/user@1000.service/app.slice/nginx.service\n"
            ),
            Classified::User
        );
    }

    #[test]
    fn cgroup_v1_is_unknown() {
        let v1 = "\
12:pids:/user.slice/user-1000.slice/session-1.scope
11:memory:/user.slice/user-1000.slice/session-1.scope
10:cpu,cpuacct:/
";
        assert_eq!(classify_cgroup(v1), Classified::Unknown);
        assert_eq!(classify_cgroup(""), Classified::Unknown);
    }

    #[test]
    fn cache_reads_once_and_drops_exited_pids() {
        let dir = tempfile::tempdir().unwrap();
        let pid_dir = dir.path().join("42");
        std::fs::create_dir_all(&pid_dir).unwrap();
        let id = "c".repeat(64);
        std::fs::write(
            pid_dir.join("cgroup"),
            format!("0::/system.slice/docker-{id}.scope\n"),
        )
        .unwrap();
        let docker = vec![DockerIndexEntry {
            id: id[..12].to_owned(),
            full_id: id.clone(),
            name: "web-app".into(),
            service: Some("web".into()),
        }];
        let mut cache = ProcKindCache::with_proc_root(dir.path());
        let a = cache.resolve(42, &docker);
        assert_eq!(a.kind, ProcKind::Container);
        assert_eq!(a.container.as_ref().unwrap().name, "web-app");
        assert_eq!(
            a.container.as_ref().unwrap().service.as_deref(),
            Some("web")
        );
        // Corrupt the file: a cache hit must not re-read.
        std::fs::write(pid_dir.join("cgroup"), "0::/system.slice/nginx.service\n").unwrap();
        let b = cache.resolve(42, &docker);
        assert_eq!(b.kind, ProcKind::Container);
        cache.retain(&[99]);
        assert!(cache.by_pid.is_empty());
        let c = cache.resolve(42, &docker);
        assert_eq!(c.kind, ProcKind::System);
        assert_eq!(c.unit.as_deref(), Some("nginx.service"));
    }

    #[test]
    fn unresolved_container_id_still_marks_kind() {
        let id = "d".repeat(64);
        let got = resolve_text(Some(&format!("0::/docker/{id}\n")), &[]);
        assert_eq!(got.kind, ProcKind::Container);
        assert_eq!(got.container.as_ref().unwrap().id, id[..12]);
        assert!(got.container.as_ref().unwrap().service.is_none());
    }
}
