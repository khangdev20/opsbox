//! Raw machine counters, as `priv` reads them (docs/BACKEND_SPEC.md 7.3).
//!
//! `priv` stays stateless: it returns cumulative counters and `serve`
//! (`opsbox-metrics`) turns consecutive samples into rates and percentages.

use serde::{Deserialize, Serialize};

/// Cumulative CPU time in clock ticks, from the `cpu` lines of `/proc/stat`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuTimes {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
}

impl CpuTimes {
    pub fn idle_all(&self) -> u64 {
        self.idle + self.iowait
    }
    pub fn total(&self) -> u64 {
        self.user
            + self.nice
            + self.system
            + self.idle
            + self.iowait
            + self.irq
            + self.softirq
            + self.steal
    }
}

/// Bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemInfo {
    pub total: u64,
    pub available: u64,
    pub free: u64,
    pub buffers: u64,
    pub cached: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskUsage {
    pub mount: String,
    pub fstype: String,
    pub total: u64,
    pub used: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskIo {
    pub device: String,
    pub read_bytes: u64,
    pub write_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetIf {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawProc {
    pub pid: u32,
    pub name: String,
    /// utime + stime, in clock ticks.
    pub ticks: u64,
    pub rss_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawSample {
    /// Milliseconds since boot, from a monotonic source; used to turn counter
    /// differences into rates.
    pub uptime_ms: u64,
    pub ticks_per_sec: u64,
    pub cpu_total: CpuTimes,
    pub cpu_cores: Vec<CpuTimes>,
    pub load: [f64; 3],
    pub mem: MemInfo,
    pub disks: Vec<DiskUsage>,
    pub disk_io: Vec<DiskIo>,
    pub net: Vec<NetIf>,
    /// Established TCP connections.
    pub tcp_conns: u32,
    /// Present only when asked for: scanning every process is the costly part.
    pub procs: Option<Vec<RawProc>>,
}

/// Cheap facts for the "needs attention" list (docs/BACKEND_SPEC.md 7.3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemFacts {
    pub reboot_required: bool,
    pub failed_units: Vec<String>,
    /// Package updates waiting, when the package manager could be asked.
    pub updates_total: Option<u32>,
    pub updates_security: Option<u32>,
}
