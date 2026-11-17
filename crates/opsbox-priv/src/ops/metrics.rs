//! `MetricsSample`: cumulative counters from `/proc` (docs/BACKEND_SPEC.md 7.3).
//!
//! The parsers are pure functions over text so they are tested against fixtures on
//! any OS. Reading the real files happens only on Linux; other systems get a coarse
//! sample from `sysinfo`, enough to develop the UI on a laptop.

// The parsers are used by the Linux sampler only; elsewhere they are still tested.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use opsbox_proto::{
    CpuTimes, DiskIo, DiskUsage, MemInfo, NetIf, PrivError, PrivErrorCode, PrivReply, RawProc,
    RawSample,
};

fn internal(msg: impl Into<String>) -> PrivError {
    PrivError {
        code: PrivErrorCode::Internal,
        message: msg.into(),
    }
}

pub(super) fn run(procs: bool) -> Result<PrivReply, PrivError> {
    sample(procs).map(|s| PrivReply::Metrics(Box::new(s)))
}

// ------------------------------------------------------------------ parsers

fn cpu_line(line: &str) -> Option<CpuTimes> {
    let mut it = line
        .split_whitespace()
        .skip(1)
        .map(|v| v.parse::<u64>().ok());
    let mut next = || it.next().flatten();
    Some(CpuTimes {
        user: next()?,
        nice: next()?,
        system: next()?,
        idle: next()?,
        // Very old kernels lack the later columns; treat them as zero.
        iowait: next().unwrap_or(0),
        irq: next().unwrap_or(0),
        softirq: next().unwrap_or(0),
        steal: next().unwrap_or(0),
    })
}

/// `/proc/stat`: the aggregate `cpu` line, then one `cpuN` line per core.
pub fn parse_stat(text: &str) -> Option<(CpuTimes, Vec<CpuTimes>)> {
    let mut total = None;
    let mut cores = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("cpu") {
            if rest.starts_with(' ') {
                total = cpu_line(line);
            } else if rest.starts_with(|c: char| c.is_ascii_digit()) {
                cores.push(cpu_line(line)?);
            }
        }
    }
    Some((total?, cores))
}

/// `/proc/meminfo` (values are in kB).
pub fn parse_meminfo(text: &str) -> MemInfo {
    let mut m = MemInfo::default();
    let mut available = None;
    for line in text.lines() {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let Some(kb) = rest
            .split_whitespace()
            .next()
            .and_then(|v| v.parse::<u64>().ok())
        else {
            continue;
        };
        let bytes = kb * 1024;
        match key {
            "MemTotal" => m.total = bytes,
            "MemFree" => m.free = bytes,
            "MemAvailable" => available = Some(bytes),
            "Buffers" => m.buffers = bytes,
            "Cached" => m.cached = bytes,
            "SwapTotal" => m.swap_total = bytes,
            "SwapFree" => m.swap_free = bytes,
            _ => {}
        }
    }
    // Kernels before 3.14 have no MemAvailable; approximate it.
    m.available = available.unwrap_or(m.free + m.buffers + m.cached);
    m
}

pub fn parse_loadavg(text: &str) -> Option<[f64; 3]> {
    let mut it = text.split_whitespace().map(|v| v.parse::<f64>().ok());
    Some([it.next()??, it.next()??, it.next()??])
}

/// `/proc/net/dev`, skipping the loopback interface.
pub fn parse_net_dev(text: &str) -> Vec<NetIf> {
    text.lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once(':')?;
            let name = name.trim();
            if name == "lo" {
                return None;
            }
            let f: Vec<u64> = rest
                .split_whitespace()
                .filter_map(|v| v.parse().ok())
                .collect();
            (f.len() >= 9).then(|| NetIf {
                name: name.to_owned(),
                rx_bytes: f[0],
                tx_bytes: f[8],
            })
        })
        .collect()
}

/// Whole block devices only (not partitions, loop or ram devices).
fn is_whole_disk(name: &str) -> bool {
    let digits_end = |p: &str| {
        name.strip_prefix(p)
            .is_some_and(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()))
    };
    let letters_only = |p: &str| {
        name.strip_prefix(p)
            .is_some_and(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_lowercase()))
    };
    letters_only("sd")
        || letters_only("vd")
        || letters_only("xvd")
        || digits_end("md")
        || digits_end("dm-")
        || (name.starts_with("nvme")
            && name.contains('n')
            && !name.contains('p')
            && name[4..].bytes().all(|b| b.is_ascii_digit() || b == b'n'))
        || (name.starts_with("mmcblk") && name[6..].bytes().all(|b| b.is_ascii_digit()))
}

/// `/proc/diskstats`; sectors are always 512 bytes.
pub fn parse_diskstats(text: &str) -> Vec<DiskIo> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 10 || !is_whole_disk(f[2]) {
                return None;
            }
            let (r, w) = (f[5].parse::<u64>().ok()?, f[9].parse::<u64>().ok()?);
            Some(DiskIo {
                device: f[2].to_owned(),
                read_bytes: r * 512,
                write_bytes: w * 512,
            })
        })
        .collect()
}

const REAL_FS: &[&str] = &[
    "ext2", "ext3", "ext4", "xfs", "btrfs", "f2fs", "zfs", "vfat", "exfat", "ntfs", "ntfs3", "jfs",
    "reiserfs",
];

/// `/proc/self/mounts` -> (mount point, fs type) of real filesystems, each device once.
pub fn parse_mounts(text: &str) -> Vec<(String, String)> {
    let unescape = |s: &str| {
        s.replace("\\040", " ")
            .replace("\\011", "\t")
            .replace("\\134", "\\")
    };
    let mut seen = std::collections::HashSet::new();
    text.lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let (dev, mount, fs) = (f.next()?, f.next()?, f.next()?);
            (REAL_FS.contains(&fs) && seen.insert(dev.to_owned()))
                .then(|| (unescape(mount), fs.to_owned()))
        })
        .collect()
}

/// Count ESTABLISHED (state `01`) rows of `/proc/net/tcp` or `tcp6`.
pub fn count_established(text: &str) -> u32 {
    let n = text
        .lines()
        .skip(1)
        .filter(|l| l.split_whitespace().nth(3) == Some("01"))
        .count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// One `/proc/<pid>/stat` line: (name, utime+stime ticks, rss pages).
///
/// The name is in parentheses and may itself contain spaces and parentheses, so
/// the fields are split after the *last* closing parenthesis.
pub fn parse_proc_stat(text: &str) -> Option<(String, u64, u64)> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let name = text.get(open + 1..close)?.to_owned();
    // After ") " come: state(3) ppid(4) ... utime(14) stime(15) ... rss(24)
    let rest: Vec<&str> = text.get(close + 1..)?.split_whitespace().collect();
    let utime: u64 = rest.get(11)?.parse().ok()?;
    let stime: u64 = rest.get(12)?.parse().ok()?;
    let rss: i64 = rest.get(21)?.parse().ok()?;
    Some((name, utime + stime, u64::try_from(rss).unwrap_or(0)))
}

// ------------------------------------------------------------------ Linux

#[cfg(target_os = "linux")]
fn sample(procs: bool) -> Result<RawSample, PrivError> {
    use std::fs::read_to_string as read;
    let get = |path: &str| read(path).map_err(|e| internal(format!("cannot read {path}: {e}")));

    let uptime: f64 = get("/proc/uptime")?
        .split_whitespace()
        .next()
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| internal("cannot parse /proc/uptime"))?;
    let (cpu_total, cpu_cores) =
        parse_stat(&get("/proc/stat")?).ok_or_else(|| internal("cannot parse /proc/stat"))?;
    let mem = parse_meminfo(&get("/proc/meminfo")?);
    let load = parse_loadavg(&get("/proc/loadavg")?).unwrap_or([0.0; 3]);
    let tcp = read("/proc/net/tcp")
        .map(|t| count_established(&t))
        .unwrap_or(0)
        + read("/proc/net/tcp6")
            .map(|t| count_established(&t))
            .unwrap_or(0);

    let disks = parse_mounts(&get("/proc/self/mounts")?)
        .into_iter()
        .filter_map(|(mount, fstype)| disk_usage(&mount, fstype))
        .collect();

    let ticks_per_sec = nix::unistd::sysconf(nix::unistd::SysconfVar::CLK_TCK)
        .ok()
        .flatten()
        .and_then(|v| u64::try_from(v).ok())
        .filter(|v| *v > 0)
        .unwrap_or(100);

    Ok(RawSample {
        uptime_ms: (uptime * 1000.0) as u64,
        ticks_per_sec,
        cpu_total,
        cpu_cores,
        load,
        mem,
        disks,
        disk_io: read("/proc/diskstats")
            .map(|t| parse_diskstats(&t))
            .unwrap_or_default(),
        net: read("/proc/net/dev")
            .map(|t| parse_net_dev(&t))
            .unwrap_or_default(),
        tcp_conns: tcp,
        procs: procs.then(scan_processes),
    })
}

#[cfg(target_os = "linux")]
fn scan_processes() -> Vec<RawProc> {
    let page = nix::unistd::sysconf(nix::unistd::SysconfVar::PAGE_SIZE)
        .ok()
        .flatten()
        .and_then(|v| u64::try_from(v).ok())
        .filter(|v| *v > 0)
        .unwrap_or(4096);
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in dir.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        // A process can exit between listing and reading: skip it quietly.
        let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        let Some((name, ticks, rss_pages)) = parse_proc_stat(&text) else {
            continue;
        };
        // Kernel threads have no memory of their own and are never interesting here.
        if rss_pages == 0 && ticks == 0 {
            continue;
        }
        out.push(RawProc {
            pid,
            name,
            ticks,
            rss_bytes: rss_pages * page,
        });
        if out.len() >= MAX_PROCS {
            break;
        }
    }
    out
}

/// Upper bound so a fork bomb cannot make a reply larger than one frame.
const MAX_PROCS: usize = 2000;

// The statvfs field types differ by platform (u32 on some 32-bit Linux targets), so the
// conversions are needed there even though they are no-ops on 64-bit systems.
#[allow(clippy::useless_conversion)]
fn disk_usage(mount: &str, fstype: String) -> Option<DiskUsage> {
    let st = nix::sys::statvfs::statvfs(mount).ok()?;
    let frsize = u64::from(st.fragment_size());
    let total = u64::from(st.blocks()) * frsize;
    let free = u64::from(st.blocks_free()) * frsize;
    (total > 0).then(|| DiskUsage {
        mount: mount.to_owned(),
        fstype,
        total,
        used: total.saturating_sub(free),
    })
}

// ------------------------------------------------------- other systems (dev)

#[cfg(not(target_os = "linux"))]
fn sample(procs: bool) -> Result<RawSample, PrivError> {
    use std::sync::Mutex;
    use std::time::Instant;
    use sysinfo::{
        CpuRefreshKind, MemoryRefreshKind, ProcessRefreshKind, ProcessesToUpdate, RefreshKind,
        System,
    };

    struct State {
        sys: System,
        last: Instant,
        busy: Vec<f64>,
        total: Vec<f64>,
    }
    static STATE: Mutex<Option<State>> = Mutex::new(None);
    let mut guard = STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard.is_none() {
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        )
        .map_err(|e| internal(e.to_string()))?;
        let n = sys.cpus().len();
        *guard = Some(State {
            sys,
            last: Instant::now(),
            busy: vec![0.0; n],
            total: vec![0.0; n],
        });
    }
    let st = guard.as_mut().expect("initialised");
    st.sys.refresh_cpu_usage();
    st.sys.refresh_memory();
    // Synthesize cumulative ticks (100 per second) from the instantaneous usage so the
    // same rate arithmetic works as on Linux.
    let dt = st.last.elapsed().as_secs_f64();
    st.last = Instant::now();
    for (i, c) in st.sys.cpus().iter().enumerate() {
        st.busy[i] += f64::from(c.usage()) / 100.0 * dt * 100.0;
        st.total[i] += dt * 100.0;
    }
    let times = |i: usize| CpuTimes {
        user: st.busy[i] as u64,
        idle: (st.total[i] - st.busy[i]).max(0.0) as u64,
        ..CpuTimes::default()
    };
    let cores: Vec<CpuTimes> = (0..st.busy.len()).map(times).collect();
    let mut cpu_total = CpuTimes::default();
    for c in &cores {
        cpu_total.user += c.user;
        cpu_total.idle += c.idle;
    }

    let procs = procs.then(|| {
        st.sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        st.sys
            .processes()
            .values()
            .map(|p| RawProc {
                pid: p.pid().as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                ticks: p.accumulated_cpu_time() / 10,
                rss_bytes: p.memory(),
            })
            .take(MAX_PROCS)
            .collect()
    });
    let load = System::load_average().map_err(|e| internal(e.to_string()))?;
    let uptime = System::uptime().map_err(|e| internal(e.to_string()))?;
    Ok(RawSample {
        uptime_ms: uptime * 1000 + u64::from(st.last.elapsed().subsec_millis()),
        ticks_per_sec: 100,
        cpu_total,
        cpu_cores: cores,
        load: [load.one, load.five, load.fifteen],
        mem: MemInfo {
            total: st.sys.total_memory(),
            available: st.sys.available_memory(),
            free: st.sys.free_memory(),
            swap_total: st.sys.total_swap(),
            swap_free: st.sys.free_swap(),
            ..MemInfo::default()
        },
        disks: disk_usage("/", "unknown".into()).into_iter().collect(),
        disk_io: Vec::new(),
        net: Vec::new(),
        tcp_conns: 0,
        procs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "cpu  4705 150 3120 100000 520 0 105 7 0 0\ncpu0 2400 80 1500 50000 260 0 60 3 0 0\ncpu1 2305 70 1620 50000 260 0 45 4 0 0\nintr 1234 5 6\nctxt 99999\nbtime 1700000000\nprocesses 4242\n";

    #[test]
    fn stat_gives_total_and_cores() {
        let (total, cores) = parse_stat(STAT).unwrap();
        assert_eq!(
            (
                total.user,
                total.nice,
                total.system,
                total.idle,
                total.iowait,
                total.steal
            ),
            (4705, 150, 3120, 100_000, 520, 7)
        );
        assert_eq!(cores.len(), 2);
        assert_eq!(cores[1].system, 1620);
        assert_eq!(total.total(), 4705 + 150 + 3120 + 100_000 + 520 + 105 + 7);
        assert_eq!(total.idle_all(), 100_520);
    }

    #[test]
    fn stat_tolerates_old_kernels_and_rejects_garbage() {
        let (t, _) = parse_stat("cpu  1 2 3 4\n").unwrap();
        assert_eq!((t.idle, t.iowait, t.steal), (4, 0, 0));
        assert!(parse_stat("nothing useful\n").is_none());
        assert!(parse_stat("cpu  a b c d\n").is_none());
    }

    #[test]
    fn meminfo_is_in_bytes_and_has_a_fallback_for_available() {
        let text = "MemTotal:       8000000 kB\nMemFree:        1000000 kB\nMemAvailable:   5000000 kB\nBuffers:         200000 kB\nCached:         2000000 kB\nSwapTotal:      2097148 kB\nSwapFree:       2097000 kB\nDirty: 4 kB\n";
        let m = parse_meminfo(text);
        assert_eq!(
            (m.total, m.available, m.cached, m.swap_total),
            (
                8_000_000 * 1024,
                5_000_000 * 1024,
                2_000_000 * 1024,
                2_097_148 * 1024
            )
        );
        let old = parse_meminfo("MemTotal: 100 kB\nMemFree: 10 kB\nBuffers: 5 kB\nCached: 20 kB\n");
        assert_eq!(old.available, 35 * 1024);
    }

    #[test]
    fn loadavg() {
        assert_eq!(
            parse_loadavg("0.62 0.48 0.41 2/311 12345\n"),
            Some([0.62, 0.48, 0.41])
        );
        assert_eq!(parse_loadavg("x y z"), None);
    }

    #[test]
    fn net_dev_skips_loopback_and_reads_rx_and_tx() {
        let text = "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n    lo: 1000 10 0 0 0 0 0 0 1000 10 0 0 0 0 0 0\n  eth0: 123456789 9000 0 0 0 0 0 0 987654321 8000 0 0 0 0 0 0\nens5:42 1 0 0 0 0 0 0 7 1 0 0 0 0 0 0\n";
        let n = parse_net_dev(text);
        assert_eq!(
            n.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            ["eth0", "ens5"]
        );
        assert_eq!((n[0].rx_bytes, n[0].tx_bytes), (123_456_789, 987_654_321));
        assert_eq!((n[1].rx_bytes, n[1].tx_bytes), (42, 7));
    }

    #[test]
    fn diskstats_keeps_whole_disks_in_bytes() {
        let text = "   8       0 sda 100 0 2000 50 200 0 4000 80 0 0 0\n   8       1 sda1 90 0 1800 40 190 0 3900 70 0 0 0\n 259       0 nvme0n1 10 0 100 5 20 0 300 9 0 0 0\n 259       1 nvme0n1p1 9 0 90 4 19 0 290 8 0 0 0\n   7       0 loop0 1 0 8 0 0 0 0 0 0 0 0\n 252       0 vda 5 0 40 2 6 0 48 3 0 0 0\n 253       0 dm-0 3 0 24 1 4 0 32 2 0 0 0\n";
        let d = parse_diskstats(text);
        assert_eq!(
            d.iter().map(|x| x.device.as_str()).collect::<Vec<_>>(),
            ["sda", "nvme0n1", "vda", "dm-0"]
        );
        assert_eq!(
            (d[0].read_bytes, d[0].write_bytes),
            (2000 * 512, 4000 * 512)
        );
    }

    #[test]
    fn mounts_keep_real_filesystems_once_per_device() {
        let text = "proc /proc proc rw 0 0\n/dev/vda1 / ext4 rw,relatime 0 0\ntmpfs /run tmpfs rw 0 0\n/dev/vda1 /var/lib/docker ext4 rw 0 0\n/dev/vdb1 /mnt/my\\040disk xfs rw 0 0\noverlay /merged overlay rw 0 0\n";
        assert_eq!(
            parse_mounts(text),
            [
                ("/".to_owned(), "ext4".to_owned()),
                ("/mnt/my disk".to_owned(), "xfs".to_owned())
            ]
        );
    }

    #[test]
    fn tcp_counts_only_established() {
        let text = "  sl  local_address rem_address   st tx_queue rx_queue\n   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000\n   1: 0100007F:1F90 0100007F:D2A0 01 00000000:00000000 00:00000000 00000000\n   2: 0100007F:D2A0 0100007F:1F90 01 00000000:00000000 00:00000000 00000000\n   3: 0100007F:0016 0100007F:AAAA 06 00000000:00000000 00:00000000 00000000\n";
        assert_eq!(count_established(text), 2);
        assert_eq!(count_established(""), 0);
    }

    #[test]
    fn proc_stat_survives_awkward_names() {
        let line = |name: &str| {
            format!(
                "4242 ({name}) S 1 4242 4242 0 -1 4194560 100 0 0 0 150 50 0 0 20 0 1 0 12345 1000000 777 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 0 0 0 0 0 0\n"
            )
        };
        for name in ["nginx", "Web Content", "a) b (c", "((("] {
            let (n, ticks, rss) = parse_proc_stat(&line(name)).unwrap_or_else(|| panic!("{name}"));
            assert_eq!((n.as_str(), ticks, rss), (name, 200, 777));
        }
        assert!(parse_proc_stat("garbage").is_none());
        assert!(parse_proc_stat("1 (x) S 1 2").is_none(), "too few fields");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_real_sample_on_linux_is_sane_and_counters_advance() {
        let a = sample(true).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let b = sample(false).unwrap();
        assert!(a.mem.total > 0 && a.mem.available <= a.mem.total);
        assert!(!a.cpu_cores.is_empty());
        assert!(
            b.cpu_total.total() > a.cpu_total.total(),
            "CPU counters advance"
        );
        assert!(b.uptime_ms > a.uptime_ms);
        assert!(
            a.procs
                .as_ref()
                .is_some_and(|p| p.iter().any(|p| p.pid == std::process::id())),
            "finds itself"
        );
        assert!(b.procs.is_none());
        assert!(
            a.disks
                .iter()
                .any(|d| d.mount == "/" && d.total > 0 && d.used <= d.total),
            "{:?}",
            a.disks
        );
    }
}
