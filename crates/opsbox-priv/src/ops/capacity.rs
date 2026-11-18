//! Host reads stay in priv. Static facts cache for ten minutes; pressure stays live.
use super::{
    Env,
    docker::{block_on_fut, map_docker_err},
};
use opsbox_capacity::{Disk, Profile, Reservation, Snapshot, meminfo_mb, parse_psi, requirements};
use opsbox_proto::{PrivError, PrivErrorCode};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

#[derive(Default)]
pub(crate) struct State {
    cache: Mutex<Option<(Instant, Profile)>>,
    pressure: Mutex<Option<Instant>>,
}
fn read(path: impl AsRef<Path>) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}
fn disk(path: &Path) -> Result<Disk, PrivError> {
    let mut probe = path;
    while !probe.exists() {
        probe = probe.parent().ok_or_else(|| {
            super::deploy::err(
                PrivErrorCode::Internal,
                "Cannot inspect capacity filesystem.",
            )
        })?;
    }
    let s = nix::sys::statvfs::statvfs(probe).map_err(|_| {
        super::deploy::err(
            PrivErrorCode::Internal,
            "Cannot inspect capacity filesystem.",
        )
    })?;
    #[allow(clippy::useless_conversion)]
    Ok(Disk {
        path: path.display().to_string(),
        total_mb: u64::from(s.blocks()) * u64::from(s.fragment_size()) / (1 << 20),
        free_mb: u64::from(s.blocks_available()) * u64::from(s.fragment_size()) / (1 << 20),
    })
}
/// Resolve the process's own cgroup, including a namespaced root; never accept a caller path.
fn cgroup_path(proc_root: &Path, sys_root: &Path) -> PathBuf {
    let text = read(proc_root.join("self/cgroup"));
    let group = text
        .lines()
        .find_map(|l| l.strip_prefix("0::"))
        .unwrap_or("/");
    let base = sys_root.join("fs/cgroup");
    let candidate = base.join(group.trim_start_matches('/'));
    if candidate.is_dir() { candidate } else { base }
}
fn limits(proc_root: &Path, sys_root: &Path, mut memory: u64, mut cpu: u64) -> (u64, u64, u64) {
    let base = sys_root.join("fs/cgroup");
    let mut cg = cgroup_path(proc_root, sys_root);
    let mut available = memory;
    loop {
        let text = read(cg.join("cpu.max"));
        let mut parts = text.split_whitespace();
        let quota = parts.next().and_then(|s| s.parse::<u64>().ok());
        let period = parts
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|n| *n > 0);
        if let Some((q, p)) = quota.zip(period) {
            cpu = cpu.min(q.saturating_mul(1000) / p);
        }
        if let Ok(max) = read(cg.join("memory.max")).trim().parse::<u64>() {
            let mb = max / (1 << 20);
            memory = memory.min(mb);
            if let Ok(current) = read(cg.join("memory.current")).trim().parse::<u64>() {
                available = available.min(mb.saturating_sub(current.div_ceil(1 << 20)));
            }
        }
        if cg == base || !cg.pop() || !cg.starts_with(&base) {
            break;
        }
    }
    (memory, cpu, available.min(memory))
}
fn hardware(proc_root: &Path, sys_root: &Path) -> Result<Profile, PrivError> {
    let mem = read(proc_root.join("meminfo"));
    let host_memory = meminfo_mb(&mem, "MemTotal");
    if host_memory == 0 {
        return Err(super::deploy::err(
            PrivErrorCode::Unsupported,
            "Capacity requires Linux procfs; host profile unavailable.",
        ));
    }
    let online = read(sys_root.join("devices/system/cpu/online"));
    let online_cpus = online
        .trim()
        .split(',')
        .filter_map(|s| {
            if let Some((a, b)) = s.split_once('-') {
                Some(b.parse::<u64>().ok()?.saturating_sub(a.parse().ok()?) + 1)
            } else {
                s.parse().ok().map(|_: u64| 1)
            }
        })
        .sum::<u64>()
        .max(1);
    let (memory_mb, cpu_millis, available) =
        limits(proc_root, sys_root, host_memory, online_cpus * 1000);
    Ok(Profile {
        memory_mb,
        available_mb: meminfo_mb(&mem, "MemAvailable").min(available),
        swap_mb: meminfo_mb(&mem, "SwapTotal"),
        swap_free_mb: meminfo_mb(&mem, "SwapFree"),
        zram: read(proc_root.join("swaps")).contains("/zram"),
        online_cpus,
        cpu_millis,
        cgroup_v2: sys_root.join("fs/cgroup/cgroup.controllers").exists(),
        ..Default::default()
    })
}
pub(crate) fn collect(env: &Env) -> Result<Snapshot, PrivError> {
    if let Some(p) = &env.capacity_profile {
        return Ok(Snapshot {
            profile: p.clone(),
            reservations: vec![],
        });
    }
    let now = Instant::now();
    let proc_root = &env.network_proc_root;
    let mut cache = env
        .capacity
        .cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if cache.as_ref().is_none_or(|(t, p)| {
        t.elapsed().as_secs() >= requirements::PROFILE_SECONDS
            || p.docker != opsbox_docker::is_docker_installed(env.docker.socket_path())
    }) {
        let mut p = hardware(proc_root, Path::new("/sys"))?;
        let docker = opsbox_docker::is_docker_installed(env.docker.socket_path());
        let root = if docker {
            Some(
                block_on_fut(env.docker.data_root())?
                    .map_err(|e| map_docker_err(e, env.docker.socket_path()))?,
            )
        } else {
            None
        };
        let mut disks = vec![
            disk(Path::new("/"))?,
            disk(env.db_path.parent().unwrap_or(&env.apps_dir))?,
            disk(&env.apps_dir)?,
        ];
        if let Some(ref root) = root {
            disks.push(disk(Path::new(root))?);
        }
        p.docker = docker;
        p.docker_data_root = root;
        p.disks = disks;
        p.below_minimum = Path::new("/var/lib/opsbox/below-minimum").exists();
        *cache = Some((now, p));
    }
    let mut p = cache.as_ref().expect("profile cached").1.clone();
    drop(cache);
    let mem = read(proc_root.join("meminfo"));
    p.available_mb = meminfo_mb(&mem, "MemAvailable").min(p.memory_mb);
    let (_, _, available) = limits(proc_root, Path::new("/sys"), p.memory_mb, p.cpu_millis);
    p.available_mb = p.available_mb.min(available);
    p.swap_mb = meminfo_mb(&mem, "SwapTotal");
    p.swap_free_mb = meminfo_mb(&mem, "SwapFree");
    p.memory_psi_avg60 = parse_psi(&read(proc_root.join("pressure/memory")));
    p.cpu_psi_avg60 = parse_psi(&read(proc_root.join("pressure/cpu")));
    let mut pressure = env
        .capacity
        .pressure
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if p.memory_psi_avg60.unwrap_or(0) > 2000 {
        p.pressure_seconds = pressure.get_or_insert(now).elapsed().as_secs();
    } else {
        *pressure = None;
    }
    // Disk headroom and Docker reservations must be fresh at admission, not ten minutes old.
    p.disks = p
        .disks
        .iter()
        .map(|d| disk(Path::new(&d.path)))
        .collect::<Result<_, _>>()?;
    let mut reservations = if p.docker {
        block_on_fut(env.docker.capacity_reservations())?
            .map_err(|e| map_docker_err(e, env.docker.socket_path()))?
    } else {
        Vec::<Reservation>::new()
    };
    for reservation in &mut reservations {
        if reservation.memory_mb == u64::MAX {
            reservation.memory_mb = p.memory_mb;
        }
    }
    Ok(Snapshot {
        profile: p,
        reservations,
    })
}
pub(crate) fn add_swap(env: &Env, size_mb: u64) -> Result<(), PrivError> {
    let s = collect(env)?;
    if !matches!(size_mb, 1024 | 2048) || s.swap_offer_mb() != Some(size_mb) {
        return Err(super::deploy::err(
            PrivErrorCode::Invalid,
            "Choose the offered 1 GB or 2 GB swap size on a server with at most 2 GB RAM and no swap.",
        ));
    }
    let d = &s.profile.disks[0];
    if d.free_mb.saturating_sub(size_mb) < requirements::disk_mb(d.total_mb) {
        return Err(super::deploy::err(
            PrivErrorCode::Conflict,
            "Adding swap would break the disk reserve.",
        ));
    }
    super::hostset::set_swap(size_mb)?;
    super::pkg::write_atomic("/etc/sysctl.d/90-opsbox-swap.conf", "vm.swappiness=10\n")?;
    let sysctl = super::hostset::needed(&["/usr/sbin/sysctl", "/sbin/sysctl"], "sysctl")?;
    super::hostset::ok_or(
        super::exec::run(
            &sysctl,
            &["-w", "vm.swappiness=10"],
            std::time::Duration::from_secs(10),
        ),
        "Setting swap priority",
    )?;
    *env.capacity
        .cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    Ok(())
}

/// Sum compressed layers and config for the selected manifest. Refuse unknown metadata.
fn manifest_bytes(value: &serde_json::Value) -> Option<u64> {
    let manifest = value
        .get("SchemaV2Manifest")
        .or_else(|| value.get("OCIManifest"))
        .unwrap_or(value);
    if let Some(layers) = manifest.get("layers").and_then(serde_json::Value::as_array) {
        let mut bytes = manifest
            .get("config")
            .and_then(|c| c.get("size"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        for layer in layers {
            bytes = bytes.checked_add(layer.get("size")?.as_u64()?)?;
        }
        return (bytes > 0).then_some(bytes);
    }
    if let Some(all) = value.as_array() {
        return all.iter().filter_map(manifest_bytes).max();
    }
    None
}
pub(crate) fn image_size(env: &Env, image: &str) -> Result<u64, PrivError> {
    if !opsbox_proto::valid_docker_image_ref(image) {
        return Err(super::deploy::err(
            PrivErrorCode::Invalid,
            "Invalid image reference.",
        ));
    }
    if let Some(size) = env.capacity_image_size_mb {
        return Ok(size);
    }
    let bin = super::deploy::find_binary(
        env.docker_bin.as_deref(),
        &["docker"],
        &["/usr/bin/docker", "/usr/local/bin/docker"],
    )
    .ok_or_else(|| {
        super::deploy::err(
            PrivErrorCode::Unsupported,
            "Docker CLI is required to check image disk space.",
        )
    })?;
    let image = image.to_owned();
    let output = block_on_fut(async move {
        let mut c = tokio::process::Command::new(bin);
        c.env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("HOME", "/root")
            .args(["manifest", "inspect", "--verbose", &image])
            .kill_on_drop(true);
        tokio::time::timeout(std::time::Duration::from_secs(20), c.output()).await
    })?
    .map_err(|_| {
        super::deploy::err(
            PrivErrorCode::Conflict,
            "Image size check timed out. Retry before pulling.",
        )
    })?
    .map_err(|_| super::deploy::err(PrivErrorCode::Internal, "Cannot check image size."))?;
    let bytes=output.status.success().then(||serde_json::from_slice::<serde_json::Value>(&output.stdout).ok()).flatten().and_then(|v|manifest_bytes(&v)).ok_or_else(||super::deploy::err(PrivErrorCode::Conflict,"Cannot determine image size. Check the image name and registry credentials before pulling."))?;
    Ok(bytes.div_ceil(1 << 20))
}
pub(crate) fn check_image(env: &Env, image: &str) -> Result<(), PrivError> {
    let disk_mb = image_size(env, image)?;
    let s = collect(env)?;
    let fit = s.admit(&opsbox_capacity::Request {
        feature: opsbox_capacity::Feature::ImagePull,
        disk_mb,
        ..Default::default()
    });
    if fit.blocked() {
        return Err(super::deploy::err(
            PrivErrorCode::Conflict,
            fit.reasons().join(" "),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proc_sys_fixtures_cover_online_cpus_parent_quotas_swap_zram_and_psi() {
        let d = tempfile::tempdir().unwrap();
        let proc = d.path().join("proc");
        let sys = d.path().join("sys");
        for path in [
            proc.join("self"),
            proc.join("pressure"),
            sys.join("devices/system/cpu"),
            sys.join("fs/cgroup/tenant/opsbox"),
        ] {
            std::fs::create_dir_all(path).unwrap();
        }
        std::fs::write(proc.join("meminfo"),"MemTotal: 4194304 kB\nMemAvailable: 3145728 kB\nSwapTotal: 1048576 kB\nSwapFree: 524288 kB\n").unwrap();
        std::fs::write(
            proc.join("swaps"),
            "Filename Type Size Used Priority\n/dev/zram0 partition 1048576 0 100\n",
        )
        .unwrap();
        std::fs::write(proc.join("self/cgroup"), "0::/tenant/opsbox\n").unwrap();
        std::fs::write(
            proc.join("pressure/memory"),
            "some avg10=10 avg60=21.50 avg300=1 total=1000\n",
        )
        .unwrap();
        std::fs::write(sys.join("devices/system/cpu/online"), "0-3,6,8-9\n").unwrap();
        std::fs::write(sys.join("fs/cgroup/cgroup.controllers"), "cpu memory io").unwrap();
        for (path, text) in [
            ("tenant/cpu.max", "75000 100000"),
            ("tenant/opsbox/cpu.max", "max 100000"),
            ("tenant/memory.max", "1073741824"),
            ("tenant/memory.current", "268435456"),
            ("tenant/opsbox/memory.max", "2147483648"),
            ("tenant/opsbox/memory.current", "134217728"),
        ] {
            std::fs::write(sys.join("fs/cgroup").join(path), text).unwrap();
        }
        let p = hardware(&proc, &sys).unwrap();
        assert_eq!(p.memory_mb, 1024);
        assert_eq!(p.available_mb, 768);
        assert_eq!(p.online_cpus, 7);
        assert_eq!(p.cpu_millis, 750);
        assert_eq!(p.swap_mb, 1024);
        assert_eq!(p.swap_free_mb, 512);
        assert!(p.zram && p.cgroup_v2);
        assert_eq!(parse_psi(&read(proc.join("pressure/memory"))), Some(2150));
        let disk = disk(d.path()).unwrap();
        assert!(disk.total_mb > 0 && disk.free_mb <= disk.total_mb);
    }
    #[test]
    fn manifest_layers_are_summed_and_unknown_sizes_refused() {
        use serde_json::json;
        assert_eq!(
            manifest_bytes(
                &json!({"SchemaV2Manifest":{"layers":[{"size":10},{"size":20}],"config":{"size":5}}})
            ),
            Some(35)
        );
        assert_eq!(
            manifest_bytes(
                &json!([{"layers":[{"size":20}]},{"OCIManifest":{"layers":[{"size":50}]}}])
            ),
            Some(50)
        );
        assert_eq!(manifest_bytes(&json!({"layers":[{}]})), None);
        assert_eq!(
            manifest_bytes(&json!({"layers":[{"size":u64::MAX},{"size":1}]})),
            None
        );
    }
}
