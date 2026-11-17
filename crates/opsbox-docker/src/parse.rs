//! Pure parsers and formatters for Docker Engine API payloads.

use std::collections::{BTreeSet, HashMap};

use bollard::container::LogOutput;
use bollard::models::{ContainerCpuStats, ContainerMemoryStats, PortSummary};
use opsbox_proto::{DockerEnvVar, DockerLogLine, DockerLogStream, DockerPort};

/// Return the short 12-character display ID (stripping any `sha256:` prefix).
pub fn short_id(id: &str) -> String {
    let stripped = id.strip_prefix("sha256:").unwrap_or(id);
    stripped.chars().take(12).collect()
}

/// Strip Docker's historic leading `/` from a container name.
pub fn clean_name(names: Option<&[String]>, fallback_id: &str) -> String {
    names
        .and_then(|v| v.first())
        .map(|s| s.trim_start_matches('/').to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| short_id(fallback_id))
}

/// Extract `(managed, service, deploy)` from `opsbox.service` / `opsbox.deploy` labels.
pub fn opsbox_labels(labels: &HashMap<String, String>) -> (bool, Option<String>, Option<String>) {
    let service = labels
        .get("opsbox.service")
        .filter(|s| !s.is_empty())
        .cloned();
    let deploy = labels
        .get("opsbox.deploy")
        .filter(|s| !s.is_empty())
        .cloned();
    let managed = service.is_some();
    (managed, service, deploy)
}

/// Convert bollard port summaries to `DockerPort` and a readable display string.
pub fn map_ports(ports: Option<&[PortSummary]>) -> (Vec<DockerPort>, String) {
    let Some(ports) = ports else {
        return (Vec::new(), "–".to_owned());
    };
    let mut list = Vec::with_capacity(ports.len());
    let mut seen = BTreeSet::new();
    let mut labels = Vec::new();

    for p in ports {
        let priv_port = p.private_port;
        let pub_port = p.public_port;
        let proto = p
            .typ
            .as_ref()
            .map_or_else(|| "tcp".to_owned(), |t| t.to_string());
        let label = match pub_port {
            Some(pub_p) if pub_p == priv_port => {
                if proto == "tcp" {
                    format!("{pub_p}")
                } else {
                    format!("{pub_p}/{proto}")
                }
            }
            Some(pub_p) => format!("{pub_p}→{priv_port}/{proto}"),
            None => format!("{priv_port}/{proto}"),
        };
        if seen.insert(label.clone()) {
            labels.push(label);
        }
        list.push(DockerPort {
            ip: p.ip.clone(),
            private_port: priv_port,
            public_port: pub_port,
            protocol: proto,
        });
    }

    let summary = if labels.is_empty() {
        "–".to_owned()
    } else {
        labels.join(", ")
    };
    (list, summary)
}

/// Split an image's `RepoTags` entry like `ghcr.io/org/app:1.2` into `(name, tag)`.
pub fn split_repo_tag(repo_tags: &[String]) -> (String, String) {
    let Some(first) = repo_tags.first() else {
        return ("<none>".to_owned(), "<none>".to_owned());
    };
    if first == "<none>:<none>" {
        return ("<none>".to_owned(), "<none>".to_owned());
    }
    let after_slash = first.rfind('/').map_or(0, |i| i + 1);
    if let Some(colon) = first[after_slash..].rfind(':') {
        let idx = after_slash + colon;
        (first[..idx].to_owned(), first[idx + 1..].to_owned())
    } else {
        (first.clone(), "latest".to_owned())
    }
}

fn is_sensitive_env_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    [
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "PASS",
        "TOKEN",
        "KEY",
        "AUTH",
        "CREDENTIAL",
        "PRIVATE",
        "DATABASE_URL",
        "REDIS_URL",
        "MYSQL_URL",
        "MONGO_URI",
        "DSN",
    ]
    .iter()
    .any(|needle| upper.contains(needle))
}

/// Parse `KEY=VALUE` environment entries, masking secret-looking values.
pub fn mask_env(entries: &[String]) -> Vec<DockerEnvVar> {
    entries
        .iter()
        .map(|raw| match raw.split_once('=') {
            Some((k, v)) => {
                let masked = is_sensitive_env_key(k);
                DockerEnvVar {
                    key: k.to_owned(),
                    value: if masked {
                        "••••".to_owned()
                    } else {
                        v.to_owned()
                    },
                    masked,
                }
            }
            None => DockerEnvVar {
                key: raw.clone(),
                value: String::new(),
                masked: false,
            },
        })
        .collect()
}

/// Parse a `LogOutput` chunk from Docker logs (splitting optional leading RFC3339 timestamp).
pub fn parse_log_chunk(chunk: LogOutput) -> Vec<DockerLogLine> {
    let (stream, bytes) = match chunk {
        LogOutput::StdOut { message } => (DockerLogStream::Stdout, message),
        LogOutput::StdErr { message } => (DockerLogStream::Stderr, message),
        LogOutput::Console { message } | LogOutput::StdIn { message } => {
            (DockerLogStream::Console, message)
        }
    };
    let text = String::from_utf8_lossy(&bytes);
    text.lines()
        .map(|line| parse_log_line(stream, line))
        .collect()
}

/// Parse a single log line that may start with Docker's RFC3339 timestamp prefix.
pub fn parse_log_line(stream: DockerLogStream, line: &str) -> DockerLogLine {
    let clean = line.trim_end_matches('\r');
    if let Some((head, rest)) = clean.split_once(' ')
        && looks_like_rfc3339(head)
    {
        return DockerLogLine {
            ts: Some(head.to_owned()),
            stream,
            text: rest.to_owned(),
        };
    }
    DockerLogLine {
        ts: None,
        stream,
        text: clean.to_owned(),
    }
}

fn looks_like_rfc3339(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 20
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
        && b[..4].iter().all(u8::is_ascii_digit)
        && (s.ends_with('Z') || s.contains('+') || s[19..].contains('-'))
}

/// Extract container memory usage in bytes matching Docker CLI (`usage - cache`).
pub fn compute_mem_used(mem: Option<&ContainerMemoryStats>) -> (u64, u64) {
    let Some(mem) = mem else {
        return (0, 0);
    };
    let usage = mem.usage.unwrap_or(0);
    let cache = mem
        .stats
        .as_ref()
        .and_then(|s| s.get("inactive_file").or_else(|| s.get("cache")))
        .copied()
        .unwrap_or(0);
    (usage.saturating_sub(cache), mem.limit.unwrap_or(0))
}

/// Extract cumulative CPU counters `(cpu_total_ns, system_cpu_ns, online_cpus)`.
pub fn extract_cpu_counters(cpu: Option<&ContainerCpuStats>) -> (u64, u64, u32) {
    let Some(cpu) = cpu else {
        return (0, 0, 1);
    };
    let total = cpu
        .cpu_usage
        .as_ref()
        .and_then(|u| u.total_usage)
        .unwrap_or(0);
    let sys = cpu.system_cpu_usage.unwrap_or(0);
    let percpu_len = cpu
        .cpu_usage
        .as_ref()
        .and_then(|u| u.percpu_usage.as_ref())
        .and_then(|v| u32::try_from(v.len()).ok())
        .unwrap_or(0);
    let cpus = cpu.online_cpus.unwrap_or(percpu_len).max(1);
    (total, sys, cpus)
}

/// Compute container CPU percentage from current and previous cumulative counters.
pub fn compute_cpu_pct(
    cpu_total_ns: u64,
    system_cpu_ns: u64,
    prev_cpu_total_ns: u64,
    prev_system_cpu_ns: u64,
    online_cpus: u32,
) -> f64 {
    let cpu_delta = cpu_total_ns.saturating_sub(prev_cpu_total_ns);
    let sys_delta = system_cpu_ns.saturating_sub(prev_system_cpu_ns);
    if cpu_delta == 0 || sys_delta == 0 || prev_system_cpu_ns == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let pct = (cpu_delta as f64 / sys_delta as f64) * f64::from(online_cpus.max(1)) * 100.0;
    if pct.is_finite() && pct >= 0.0 {
        pct
    } else {
        0.0
    }
}

/// Parsed Docker `/system/df` summary compatible with both API < 1.52 and API >= 1.52.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DfSummary {
    pub build_cache_reclaimable: u64,
    pub volumes_reclaimable: u64,
    pub volumes_total_count: usize,
}

/// Parse `/system/df` JSON across Docker API versions (both pre-1.52 and 1.52+).
pub fn parse_df_json(v: &serde_json::Value) -> DfSummary {
    let mut out = DfSummary::default();

    // Docker API >= 1.52 (Docker Engine 29+)
    if let Some(bc) = v.get("BuildCacheUsage").and_then(|x| x.as_object()) {
        out.build_cache_reclaimable = bc
            .get("Reclaimable")
            .and_then(serde_json::Value::as_i64)
            .and_then(|n| u64::try_from(n).ok())
            .unwrap_or(0);
    } else if let Some(arr) = v.get("BuildCache").and_then(|x| x.as_array()) {
        // Docker API < 1.52 (Docker Engine 20–28)
        out.build_cache_reclaimable = arr
            .iter()
            .filter(|item| !item.get("InUse").and_then(|b| b.as_bool()).unwrap_or(false))
            .filter_map(|item| item.get("Size").and_then(serde_json::Value::as_i64))
            .filter_map(|s| u64::try_from(s).ok())
            .sum();
    }

    if let Some(vu) = v.get("VolumeUsage").and_then(|x| x.as_object()) {
        out.volumes_reclaimable = vu
            .get("Reclaimable")
            .and_then(serde_json::Value::as_i64)
            .and_then(|n| u64::try_from(n).ok())
            .unwrap_or(0);
        out.volumes_total_count = vu
            .get("TotalCount")
            .and_then(serde_json::Value::as_i64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0);
    } else if let Some(arr) = v.get("Volumes").and_then(|x| x.as_array()) {
        out.volumes_total_count = arr.len();
        out.volumes_reclaimable = arr
            .iter()
            .filter_map(|vol| {
                let usage = vol.get("UsageData")?.as_object()?;
                let ref_count = usage.get("RefCount")?.as_i64()?;
                let size = usage.get("Size")?.as_i64()?;
                (ref_count == 0 && size > 0).then(|| u64::try_from(size).ok())?
            })
            .sum();
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::PortSummaryTypeEnum;

    #[test]
    fn repo_tags_split_with_registry_ports() {
        assert_eq!(
            split_repo_tag(&["nginx:1.27".into()]),
            ("nginx".into(), "1.27".into())
        );
        assert_eq!(
            split_repo_tag(&["localhost:5000/my/app:v2".into()]),
            ("localhost:5000/my/app".into(), "v2".into())
        );
        assert_eq!(
            split_repo_tag(&["localhost:5000/my/app".into()]),
            ("localhost:5000/my/app".into(), "latest".into())
        );
        assert_eq!(
            split_repo_tag(&["<none>:<none>".into()]),
            ("<none>".into(), "<none>".into())
        );
        assert_eq!(split_repo_tag(&[]), ("<none>".into(), "<none>".into()));
    }

    #[test]
    fn ports_format_cleanly_and_deduplicate() {
        let ports = vec![
            PortSummary {
                ip: Some("0.0.0.0".into()),
                private_port: 80,
                public_port: Some(8080),
                typ: Some(PortSummaryTypeEnum::TCP),
            },
            PortSummary {
                ip: Some("::".into()),
                private_port: 80,
                public_port: Some(8080),
                typ: Some(PortSummaryTypeEnum::TCP),
            },
            PortSummary {
                ip: None,
                private_port: 5432,
                public_port: None,
                typ: Some(PortSummaryTypeEnum::TCP),
            },
        ];
        let (list, summary) = map_ports(Some(&ports));
        assert_eq!(list.len(), 3);
        assert_eq!(summary, "8080→80/tcp, 5432/tcp");
        assert_eq!(map_ports(None).1, "–");
    }

    #[test]
    fn env_secrets_are_masked() {
        let vars = mask_env(&[
            "NODE_ENV=production".into(),
            "DATABASE_URL=postgres://user:pw@db:5432/app".into(),
            "JWT_SECRET=supersecret".into(),
            "PORT=3000".into(),
        ]);
        assert!(!vars[0].masked && vars[0].value == "production");
        assert!(vars[1].masked && vars[1].value == "••••");
        assert!(vars[2].masked && vars[2].value == "••••");
        assert!(!vars[3].masked && vars[3].value == "3000");
    }

    #[test]
    fn log_lines_split_rfc3339_timestamps() {
        let a = parse_log_line(
            DockerLogStream::Stdout,
            "2026-11-18T05:10:00.123456789Z GET /healthz 200\r",
        );
        assert_eq!(a.ts.as_deref(), Some("2026-11-18T05:10:00.123456789Z"));
        assert_eq!(a.text, "GET /healthz 200");

        let b = parse_log_line(DockerLogStream::Stderr, "plain error message");
        assert_eq!(b.ts, None);
        assert_eq!(b.text, "plain error message");
    }

    #[test]
    fn cpu_and_memory_match_docker_stats_semantics() {
        let mut stats = HashMap::new();
        stats.insert("inactive_file".into(), 100);
        let mem = ContainerMemoryStats {
            usage: Some(500),
            limit: Some(1024),
            stats: Some(stats),
            ..Default::default()
        };
        assert_eq!(compute_mem_used(Some(&mem)), (400, 1024));
        assert!((compute_cpu_pct(300, 2000, 100, 1000, 2) - 40.0).abs() < 0.01);
        assert_eq!(compute_cpu_pct(100, 1000, 100, 0, 2), 0.0);
    }

    #[test]
    fn df_json_parses_both_legacy_and_v152_shapes() {
        let legacy = serde_json::json!({
            "BuildCache": [
                {"InUse": true, "Size": 100},
                {"InUse": false, "Size": 400},
                {"InUse": false, "Size": 50}
            ],
            "Volumes": [
                {"Name": "used", "UsageData": {"RefCount": 1, "Size": 999}},
                {"Name": "unused", "UsageData": {"RefCount": 0, "Size": 300}},
                {"Name": "unknown", "UsageData": {"RefCount": -1, "Size": -1}}
            ]
        });
        assert_eq!(
            parse_df_json(&legacy),
            DfSummary {
                build_cache_reclaimable: 450,
                volumes_reclaimable: 300,
                volumes_total_count: 3,
            }
        );

        let v152 = serde_json::json!({
            "BuildCacheUsage": {"ActiveCount": 1, "TotalCount": 3, "Reclaimable": 1200, "TotalSize": 2000},
            "VolumeUsage": {"ActiveCount": 1, "TotalCount": 2, "Reclaimable": 600, "TotalSize": 1000}
        });
        assert_eq!(
            parse_df_json(&v152),
            DfSummary {
                build_cache_reclaimable: 1200,
                volumes_reclaimable: 600,
                volumes_total_count: 2,
            }
        );
    }
}
