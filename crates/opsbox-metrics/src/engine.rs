//! Raw counters in, rates out.

use std::collections::{HashMap, VecDeque};

use opsbox_proto::{CpuTimes, DockerContainerStat, RawSample};
use serde::Serialize;

use crate::proc_kind::{ProcContainer, ProcKind};

/// One second of the machine's state.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Point {
    /// Unix seconds.
    pub ts: i64,
    /// Total CPU busy, 0 to 100.
    pub cpu: f32,
    /// Per core, 0 to 100.
    pub cores: Vec<u8>,
    pub mem_total: u64,
    pub mem_used: u64,
    /// Buffers plus page cache.
    pub cache: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub load: [f32; 3],
    /// Bytes per second over all interfaces except loopback.
    pub net_in: f64,
    pub net_out: f64,
    pub disk_read: f64,
    pub disk_write: f64,
    pub conns: u32,
}

impl Point {
    pub fn mem_pct(&self) -> f32 {
        if self.mem_total == 0 {
            0.0
        } else {
            (self.mem_used as f64 / self.mem_total as f64 * 100.0) as f32
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiskView {
    pub mount: String,
    pub fstype: String,
    pub used: u64,
    pub total: u64,
}

impl DiskView {
    pub fn pct(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.used as f64 / self.total as f64 * 100.0
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProcView {
    pub pid: u32,
    pub name: String,
    /// Percent of one core (can exceed 100 for a multi-threaded process).
    pub cpu: f32,
    pub rss: u64,
    /// One of opsbox's own processes.
    pub is_self: bool,
    /// From `/proc/<pid>/cgroup`: container, system, user, or unknown.
    pub kind: ProcKind,
    /// Set when `kind` is `container`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<ProcContainer>,
    /// Systemd unit when `kind` is `system` (e.g. `docker.service`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub point: Point,
    pub disks: Vec<DiskView>,
    /// Refreshed every other second; carried over in between.
    pub procs: Vec<ProcView>,
    /// Running container CPU/memory stats polled in the background.
    pub containers: Vec<DockerContainerStat>,
}

/// How many seconds of history the ring keeps.
pub const RING_SECS: usize = 3600;
/// A shorter gap between two samples than this would make rates meaningless.
const MIN_DT_SECS: f64 = 0.2;
const TOP_BY_CPU: usize = 6;
const TOP_BY_MEM: usize = 3;
const MAX_PROCS_SHOWN: usize = 8;

pub struct Engine {
    prev: Option<RawSample>,
    prev_procs: Option<(u64, HashMap<u32, u64>)>, // (uptime_ms, pid -> ticks)
    procs: Vec<ProcView>,
    ring: VecDeque<Point>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

fn pct_between(a: &CpuTimes, b: &CpuTimes) -> f32 {
    let total = b.total().saturating_sub(a.total());
    if total == 0 {
        return 0.0;
    }
    let idle = b.idle_all().saturating_sub(a.idle_all());
    ((total.saturating_sub(idle)) as f64 / total as f64 * 100.0).clamp(0.0, 100.0) as f32
}

/// Counters only go up; a smaller value means a reset (interface re-created, reboot).
fn delta(prev: u64, now: u64) -> u64 {
    now.saturating_sub(prev)
}

impl Engine {
    pub fn new() -> Self {
        Self {
            prev: None,
            prev_procs: None,
            procs: Vec::new(),
            ring: VecDeque::with_capacity(RING_SECS),
        }
    }

    /// The points of the last `secs` seconds, oldest first.
    pub fn window(&self, secs: usize) -> Vec<Point> {
        let skip = self.ring.len().saturating_sub(secs);
        self.ring.iter().skip(skip).cloned().collect()
    }

    /// Feed the next sample. The first one only primes the counters.
    pub fn ingest(&mut self, raw: RawSample, now: i64) -> Option<Snapshot> {
        let Some(prev) = self.prev.take() else {
            self.remember_procs(&raw);
            self.prev = Some(raw);
            return None;
        };
        let dt = raw.uptime_ms.saturating_sub(prev.uptime_ms) as f64 / 1000.0;
        if dt < MIN_DT_SECS {
            // Too close together (or the clock went backwards): keep the older sample.
            self.prev = Some(prev);
            return None;
        }

        // Sum per interface so one interface being reset does not hide the others.
        let per_iface = |pick: fn(&opsbox_proto::NetIf) -> u64| -> f64 {
            raw.net
                .iter()
                .map(|n| {
                    let before = prev
                        .net
                        .iter()
                        .find(|p| p.name == n.name)
                        .map_or(pick(n), pick);
                    delta(before, pick(n))
                })
                .sum::<u64>() as f64
                / dt
        };
        let per_disk = |pick: fn(&opsbox_proto::DiskIo) -> u64| -> f64 {
            raw.disk_io
                .iter()
                .map(|d| {
                    let before = prev
                        .disk_io
                        .iter()
                        .find(|p| p.device == d.device)
                        .map_or(pick(d), pick);
                    delta(before, pick(d))
                })
                .sum::<u64>() as f64
                / dt
        };

        let cores = raw
            .cpu_cores
            .iter()
            .zip(&prev.cpu_cores)
            .map(|(now_c, was)| pct_between(was, now_c).round() as u8)
            .collect();
        let mem = &raw.mem;
        let point = Point {
            ts: now,
            cpu: pct_between(&prev.cpu_total, &raw.cpu_total),
            cores,
            mem_total: mem.total,
            mem_used: mem.total.saturating_sub(mem.available),
            cache: mem.buffers + mem.cached,
            swap_total: mem.swap_total,
            swap_used: mem.swap_total.saturating_sub(mem.swap_free),
            load: [raw.load[0] as f32, raw.load[1] as f32, raw.load[2] as f32],
            net_in: per_iface(|n| n.rx_bytes),
            net_out: per_iface(|n| n.tx_bytes),
            disk_read: per_disk(|d| d.read_bytes),
            disk_write: per_disk(|d| d.write_bytes),
            conns: raw.tcp_conns,
        };

        if let Some(list) = &raw.procs {
            self.update_procs(&raw, list);
        }
        let disks = raw
            .disks
            .iter()
            .map(|d| DiskView {
                mount: d.mount.clone(),
                fstype: d.fstype.clone(),
                used: d.used,
                total: d.total,
            })
            .collect();

        if self.ring.len() == RING_SECS {
            self.ring.pop_front();
        }
        self.ring.push_back(point.clone());
        self.prev = Some(raw);
        Some(Snapshot {
            point,
            disks,
            procs: self.procs.clone(),
            containers: Vec::new(),
        })
    }

    fn remember_procs(&mut self, raw: &RawSample) {
        if let Some(list) = &raw.procs {
            self.prev_procs = Some((
                raw.uptime_ms,
                list.iter().map(|p| (p.pid, p.ticks)).collect(),
            ));
        }
    }

    fn update_procs(&mut self, raw: &RawSample, list: &[opsbox_proto::RawProc]) {
        if let Some((was_ms, was)) = &self.prev_procs {
            let dt = raw.uptime_ms.saturating_sub(*was_ms) as f64 / 1000.0;
            if dt >= MIN_DT_SECS {
                let per_sec = raw.ticks_per_sec.max(1) as f64;
                let mut all: Vec<ProcView> = list
                    .iter()
                    .map(|p| {
                        // A process we have not seen before has no baseline: 0 % this round.
                        let before = was.get(&p.pid).copied().unwrap_or(p.ticks);
                        ProcView {
                            pid: p.pid,
                            name: p.name.clone(),
                            cpu: (delta(before, p.ticks) as f64 / (per_sec * dt) * 100.0) as f32,
                            rss: p.rss_bytes,
                            is_self: p.name.starts_with("opsbox"),
                            kind: ProcKind::Unknown,
                            container: None,
                            unit: None,
                        }
                    })
                    .collect();
                self.procs = top_processes(&mut all);
            }
        }
        self.remember_procs(raw);
    }
}

/// The busiest processes by CPU, plus the biggest by memory that are not already there.
fn top_processes(all: &mut [ProcView]) -> Vec<ProcView> {
    all.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.rss.cmp(&a.rss)));
    let mut out: Vec<ProcView> = all.iter().take(TOP_BY_CPU).cloned().collect();
    let mut by_mem: Vec<&ProcView> = all.iter().collect();
    by_mem.sort_by_key(|p| std::cmp::Reverse(p.rss));
    for p in by_mem.into_iter().take(TOP_BY_MEM) {
        if !out.iter().any(|o| o.pid == p.pid) && out.len() < MAX_PROCS_SHOWN {
            out.push(p.clone());
        }
    }
    out.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.rss.cmp(&a.rss)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use opsbox_proto::{DiskIo, DiskUsage, MemInfo, NetIf, RawProc};

    fn cpu(busy: u64, idle: u64) -> CpuTimes {
        CpuTimes {
            user: busy,
            idle,
            ..CpuTimes::default()
        }
    }

    fn sample(ms: u64, busy: u64, idle: u64) -> RawSample {
        RawSample {
            uptime_ms: ms,
            ticks_per_sec: 100,
            cpu_total: cpu(busy, idle),
            cpu_cores: vec![cpu(busy / 2, idle / 2), cpu(busy / 2, idle / 2)],
            load: [0.5, 0.4, 0.3],
            mem: MemInfo {
                total: 8_000,
                available: 5_000,
                buffers: 100,
                cached: 900,
                swap_total: 1_000,
                swap_free: 600,
                ..MemInfo::default()
            },
            disks: vec![DiskUsage {
                mount: "/".into(),
                fstype: "ext4".into(),
                total: 100,
                used: 82,
            }],
            disk_io: vec![],
            net: vec![],
            tcp_conns: 7,
            procs: None,
        }
    }

    #[test]
    fn the_first_sample_only_primes_the_counters() {
        let mut e = Engine::new();
        assert!(e.ingest(sample(1_000, 0, 0), 10).is_none());
        assert!(e.ingest(sample(2_000, 25, 75), 11).is_some());
    }

    #[test]
    fn cpu_percent_comes_from_counter_differences() {
        let mut e = Engine::new();
        e.ingest(sample(1_000, 100, 900), 10);
        let s = e.ingest(sample(2_000, 130, 970), 11).unwrap(); // +30 busy, +70 idle
        assert!((s.point.cpu - 30.0).abs() < 0.01, "{}", s.point.cpu);
        assert_eq!(s.point.cores, [30, 30]);
        // Idle machine and a stalled counter are both 0, never NaN.
        assert_eq!(
            e.ingest(sample(3_000, 130, 970), 12).unwrap().point.cpu,
            0.0
        );
    }

    #[test]
    fn memory_swap_load_and_disks_are_reported_as_they_are() {
        let mut e = Engine::new();
        e.ingest(sample(1_000, 0, 100), 1);
        let s = e.ingest(sample(2_000, 10, 190), 2).unwrap();
        assert_eq!(
            (
                s.point.mem_used,
                s.point.cache,
                s.point.swap_used,
                s.point.conns
            ),
            (3_000, 1_000, 400, 7)
        );
        assert!((s.point.mem_pct() - 37.5).abs() < 0.01);
        assert_eq!(s.point.load, [0.5, 0.4, 0.3]);
        assert_eq!(s.disks.len(), 1);
        assert!((s.disks[0].pct() - 82.0).abs() < 0.01);
    }

    #[test]
    fn network_and_disk_rates_use_elapsed_time_and_survive_resets() {
        let mut a = sample(1_000, 0, 100);
        a.net = vec![
            NetIf {
                name: "eth0".into(),
                rx_bytes: 1_000,
                tx_bytes: 500,
            },
            NetIf {
                name: "eth1".into(),
                rx_bytes: 9_000,
                tx_bytes: 9_000,
            },
        ];
        a.disk_io = vec![DiskIo {
            device: "sda".into(),
            read_bytes: 10_000,
            write_bytes: 20_000,
        }];
        let mut b = sample(3_000, 10, 190); // two seconds later
        b.net = vec![
            NetIf {
                name: "eth0".into(),
                rx_bytes: 5_000,
                tx_bytes: 1_500,
            },
            NetIf {
                name: "eth1".into(),
                rx_bytes: 100,
                tx_bytes: 50,
            },
        ]; // eth1 was reset
        b.disk_io = vec![DiskIo {
            device: "sda".into(),
            read_bytes: 14_000,
            write_bytes: 20_000,
        }];
        let mut e = Engine::new();
        e.ingest(a, 1);
        let p = e.ingest(b, 3).unwrap().point;
        assert_eq!(
            (p.net_in, p.net_out),
            (2_000.0, 500.0),
            "eth1's reset counts as nothing, not a huge number"
        );
        assert_eq!((p.disk_read, p.disk_write), (2_000.0, 0.0));
    }

    #[test]
    fn a_new_interface_has_no_baseline_and_counts_as_zero() {
        let a = sample(1_000, 0, 100);
        let mut b = sample(2_000, 1, 199);
        b.net = vec![NetIf {
            name: "wg0".into(),
            rx_bytes: 1_000_000,
            tx_bytes: 1_000_000,
        }];
        let mut e = Engine::new();
        e.ingest(a, 1);
        assert_eq!(e.ingest(b, 2).unwrap().point.net_in, 0.0);
    }

    #[test]
    fn samples_too_close_together_are_ignored_without_losing_the_baseline() {
        let mut e = Engine::new();
        e.ingest(sample(1_000, 0, 100), 1);
        assert!(e.ingest(sample(1_050, 5, 105), 1).is_none());
        // The baseline is still the first sample, so the next rate spans the whole gap.
        let s = e.ingest(sample(2_000, 50, 150), 2).unwrap();
        assert!((s.point.cpu - 50.0).abs() < 0.01);
        assert!(
            e.ingest(sample(1_900, 60, 160), 3).is_none(),
            "time going backwards"
        );
    }

    fn procs(list: &[(u32, &str, u64, u64)]) -> Option<Vec<RawProc>> {
        Some(
            list.iter()
                .map(|(pid, name, ticks, rss)| RawProc {
                    pid: *pid,
                    name: (*name).into(),
                    ticks: *ticks,
                    rss_bytes: *rss,
                })
                .collect(),
        )
    }

    #[test]
    fn process_cpu_is_percent_of_one_core_and_the_list_is_the_busy_plus_the_big() {
        let mut a = sample(1_000, 0, 100);
        a.procs = procs(&[
            (1, "idle", 10, 5),
            (2, "busy", 100, 10),
            (3, "bighog", 50, 9_000),
            (4, "opsbox", 5, 20),
            (5, "half", 100, 1),
        ]);
        let mut b = sample(3_000, 10, 190); // 2 s later, 100 ticks/s
        b.procs = procs(&[
            (1, "idle", 10, 5),
            (2, "busy", 400, 10),
            (3, "bighog", 50, 9_000),
            (4, "opsbox", 7, 20),
            (5, "half", 200, 1),
            (9, "newcomer", 999, 3),
        ]);
        let mut e = Engine::new();
        e.ingest(a, 1);
        let s = e.ingest(b, 3).unwrap();
        let by = |n: &str| s.procs.iter().find(|p| p.name == n).unwrap();
        assert!(
            (by("busy").cpu - 150.0).abs() < 0.01,
            "300 ticks over 2 s at 100/s = 150 %"
        );
        assert!((by("half").cpu - 50.0).abs() < 0.01);
        assert_eq!(by("newcomer").cpu, 0.0, "no baseline yet");
        assert!(by("opsbox").is_self && !by("busy").is_self);
        assert_eq!(s.procs[0].name, "busy", "sorted by CPU");
        assert!(
            s.procs.iter().any(|p| p.name == "bighog"),
            "a memory hog shows even when idle"
        );
    }

    #[test]
    fn the_process_list_is_kept_between_scans_and_capped() {
        let mut a = sample(1_000, 0, 100);
        a.procs = Some(
            (1..=50)
                .map(|i| RawProc {
                    pid: i,
                    name: "p".into(),
                    ticks: 0,
                    rss_bytes: u64::from(i),
                })
                .collect(),
        );
        let mut b = sample(3_000, 10, 190);
        b.procs = Some(
            (1..=50)
                .map(|i| RawProc {
                    pid: i,
                    name: "p".into(),
                    ticks: u64::from(i) * 10,
                    rss_bytes: u64::from(i),
                })
                .collect(),
        );
        let mut e = Engine::new();
        e.ingest(a, 1);
        let first = e.ingest(b, 3).unwrap();
        assert!(first.procs.len() <= MAX_PROCS_SHOWN && first.procs.len() >= TOP_BY_CPU);
        // A sample without a process scan carries the previous list over.
        let next = e.ingest(sample(4_000, 20, 280), 4).unwrap();
        assert_eq!(next.procs, first.procs);
    }

    #[test]
    fn the_ring_keeps_the_newest_hour_and_serves_windows() {
        let mut e = Engine::new();
        e.ingest(sample(0, 0, 0), 0);
        for i in 1..=(RING_SECS as u64 + 10) {
            e.ingest(sample(i * 1_000, i * 10, i * 90), i as i64);
        }
        assert_eq!(e.window(usize::MAX).len(), RING_SECS);
        let w = e.window(5);
        assert_eq!(w.len(), 5);
        assert_eq!(w.last().unwrap().ts, RING_SECS as i64 + 10);
        assert!(w.windows(2).all(|p| p[0].ts < p[1].ts), "oldest first");
        assert_eq!(e.window(0).len(), 0);
    }
}
