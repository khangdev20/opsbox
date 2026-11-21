//! Typed proxy listener state and narrowly scoped port-conflict actions.
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortContainer {
    pub id: String,
    pub name: String,
    pub image: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortHolder {
    pub port: u16,
    pub pid: u32,
    pub process: String,
    pub unit: Option<String>,
    pub container: Option<PortContainer>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyNetwork {
    pub running: bool,
    pub port_conflict: bool,
    pub listen_http: Option<SocketAddr>,
    pub listen_https: Option<SocketAddr>,
    pub primary_http: Option<SocketAddr>,
    pub primary_https: Option<SocketAddr>,
    pub conflict_ports: Vec<u16>,
    pub holders: Vec<PortHolder>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum PortTarget {
    Container(String),
    Unit(String),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortTakeover {
    pub id: String,
    pub target: PortTarget,
    pub previous_policy: String,
    pub was_running: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListenerAction {
    KeepFallback,
    YieldPrimary,
    ClaimPrimary,
}

/// Independent of app routes: app/panel routes always take precedence.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdoptedRoutes {
    pub domains: Vec<AdoptedDomain>,
    pub everything_else: bool,
    pub http_port: u16,
    pub https_port: u16,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdoptedDomain {
    pub domain: String,
    pub served_by: String,
    pub certificate_expiry: Option<String>,
    #[serde(default)]
    pub has_certificate: bool,
    pub reachable_now: bool,
    #[serde(default)]
    pub terminate_tls: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebPlan {
    pub target: PortTarget,
    pub holder: String,
    pub image: Option<String>,
    pub compose_project: Option<String>,
    pub working_dir: Option<String>,
    pub config_files: Vec<String>,
    pub routes: AdoptedRoutes,
    pub changes: Vec<WebDiff>,
    pub supported: bool,
    pub explanation: String,
    pub confirmation: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebDiff {
    pub path: String,
    pub before: String,
    pub after: String,
}
/// Public receipt: never config contents, environment variables or private keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebMove {
    pub id: String,
    pub holder: String,
    pub files: Vec<String>,
    pub http_port: u16,
    pub https_port: u16,
    pub steps: Vec<String>,
    pub active: bool,
}

/// Whether opsbox owns public proxy listeners and certificate management.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxyMode {
    #[default]
    Builtin,
    External,
    Off,
}

impl std::fmt::Display for ProxyMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Builtin => "builtin",
            Self::External => "external",
            Self::Off => "off",
        })
    }
}
