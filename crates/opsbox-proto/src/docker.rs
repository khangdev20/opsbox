//! Shared Docker types for `priv`, `serve`, and `opsbox-docker` (docs/BACKEND_SPEC.md 7.4).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Validate a Docker container ID or name before passing it to the Engine API.
///
/// Allows `[A-Za-z0-9][A-Za-z0-9_.-]*` up to 128 bytes and rejects path traversal (`..`).
pub fn valid_docker_ref(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id != "."
        && id != ".."
        && !id.contains("..")
        && id.starts_with(|c: char| c.is_ascii_alphanumeric())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerStatus {
    pub installed: bool,
    pub running: bool,
    pub version: Option<String>,
    pub api_version: Option<String>,
    pub can_install: bool,
    pub install_method: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerPort {
    pub ip: Option<String>,
    pub private_port: u16,
    pub public_port: Option<u16>,
    pub protocol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerContainer {
    pub id: String,
    pub full_id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub created: i64,
    pub ports: String,
    pub port_list: Vec<DockerPort>,
    pub labels: BTreeMap<String, String>,
    pub managed: bool,
    pub service: Option<String>,
    pub deploy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerMount {
    pub kind: String,
    pub source: String,
    pub destination: String,
    pub mode: String,
    pub rw: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerContainerNetwork {
    pub name: String,
    pub ip_address: String,
    pub gateway: String,
    pub mac_address: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerEnvVar {
    pub key: String,
    pub value: String,
    pub masked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerContainerInspect {
    pub id: String,
    pub full_id: String,
    pub name: String,
    pub image: String,
    pub created: String,
    pub state: String,
    pub running: bool,
    pub restarting: bool,
    pub paused: bool,
    pub oom_killed: bool,
    pub exit_code: i64,
    pub started_at: String,
    pub finished_at: String,
    pub restart_count: u32,
    pub pid: Option<i64>,
    pub cmd: Vec<String>,
    pub entrypoint: Vec<String>,
    pub working_dir: String,
    pub user: String,
    pub ports: Vec<DockerPort>,
    pub mounts: Vec<DockerMount>,
    pub networks: Vec<DockerContainerNetwork>,
    pub env: Vec<DockerEnvVar>,
    pub labels: BTreeMap<String, String>,
    pub managed: bool,
    pub service: Option<String>,
    pub deploy: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockerContainerAction {
    Start,
    Stop,
    Restart,
    Remove,
}

impl DockerContainerAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Remove => "remove",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DockerContainerStat {
    pub id: String,
    pub full_id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub cpu_pct: f64,
    pub cpu_total_ns: u64,
    pub system_cpu_ns: u64,
    pub online_cpus: u32,
    pub mem_used_bytes: u64,
    pub mem_limit_bytes: u64,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    pub pids: u64,
    pub managed: bool,
    pub service: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockerLogStream {
    Stdout,
    Stderr,
    Console,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerLogLine {
    pub ts: Option<String>,
    pub stream: DockerLogStream,
    pub text: String,
}

/// Frames sent from `priv` to `serve` after [`crate::PrivReply::DockerLogStreamStarted`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DockerLogFrame {
    Line(DockerLogLine),
    End,
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerExecRequest {
    pub id: String,
    pub cmd: Vec<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub stdin: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerExecOutput {
    pub exit_code: i64,
    pub stdout: Vec<u8>,
    pub stderr: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerImage {
    pub id: String,
    pub full_id: String,
    pub name: String,
    pub tag: String,
    pub size_bytes: u64,
    pub created: i64,
    pub used: bool,
    pub containers: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerVolume {
    pub name: String,
    pub driver: String,
    pub mountpoint: String,
    pub size_bytes: Option<u64>,
    pub used: bool,
    pub created_at: Option<String>,
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerNetwork {
    pub id: String,
    pub full_id: String,
    pub name: String,
    pub driver: String,
    pub scope: String,
    pub internal: bool,
    pub containers: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_container_refs_and_rejects_traversal() {
        for ok in [
            "nginx",
            "web-app_1.2",
            "a1b2c3d4e5f6",
            "9f8e7d6c5b4a3210",
            "opsbox.postgres-main",
        ] {
            assert!(valid_docker_ref(ok), "{ok}");
        }
        for bad in [
            "",
            ".",
            "..",
            "../etc",
            "a/../b",
            "a/b",
            "a b",
            "-leading",
            ".hidden",
            "a\0b",
            &"a".repeat(129),
        ] {
            assert!(!valid_docker_ref(bad), "{bad:?}");
        }
    }
}
