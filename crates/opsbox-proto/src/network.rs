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
}
