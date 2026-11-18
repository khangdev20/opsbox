//! Growing the filesystem after the disk was enlarged (docs/prompts 16h).
//!
//! Detection reads `findmnt -J` and `lsblk -J -b` plus `/sys/class/block/<part>/start`, and
//! turns them into a [`DiskVolume`] per filesystem opsbox cares about. Growing is one typed
//! step at a time: the caller names a mount point and a [`DiskStep`], and every device name
//! used here comes from a fresh detection, never from the caller. Commands run with fixed
//! argument lists (no shell). The parsing and planning are pure so fixtures can test them,
//! and the runner is a trait so tests can check the exact commands without a disk.

use std::collections::HashMap;
use std::time::Duration;

use opsbox_proto::{
    DiskLayout, DiskState, DiskStep, DiskVolume, PrivError, PrivErrorCode, PrivReply,
};
use serde_json::Value;

use super::Env;
use super::exec::{self, Output, tail};

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;
/// Free space after a partition (or in an LVM group) below this is alignment and GPT
/// bookkeeping, not an enlarged disk.
const MIN_GROW: u64 = 64 * MIB;

const FINDMNT: &[&str] = &["/usr/bin/findmnt", "/bin/findmnt"];
const LSBLK: &[&str] = &["/usr/bin/lsblk", "/bin/lsblk"];
const GROWPART: &[&str] = &["/usr/bin/growpart", "/usr/sbin/growpart", "/sbin/growpart"];
const RESIZE2FS: &[&str] = &[
    "/usr/sbin/resize2fs",
    "/sbin/resize2fs",
    "/usr/bin/resize2fs",
];
const XFS_GROWFS: &[&str] = &[
    "/usr/sbin/xfs_growfs",
    "/sbin/xfs_growfs",
    "/usr/bin/xfs_growfs",
];
const BTRFS: &[&str] = &[
    "/usr/bin/btrfs",
    "/usr/sbin/btrfs",
    "/sbin/btrfs",
    "/bin/btrfs",
];
const PVRESIZE: &[&str] = &["/usr/sbin/pvresize", "/sbin/pvresize", "/usr/bin/pvresize"];
const LVEXTEND: &[&str] = &["/usr/sbin/lvextend", "/sbin/lvextend", "/usr/bin/lvextend"];
const APT_GET: &[&str] = &["/usr/bin/apt-get"];
const DNF: &[&str] = &["/usr/bin/dnf"];

fn err(code: PrivErrorCode, msg: impl Into<String>) -> PrivError {
    PrivError {
        code,
        message: msg.into(),
    }
}

// ------------------------------------------------------------------ runner

/// What detection and the steps need from the machine.
pub(crate) trait Host {
    fn find(&self, candidates: &[&str]) -> Option<String>;
    fn run(
        &self,
        program: &str,
        args: &[&str],
        limit: Duration,
        env: &[(&str, &str)],
    ) -> Option<Output>;
    /// First sector of a partition from `/sys/class/block/<name>/start`.
    fn partition_start(&self, name: &str) -> Option<u64>;
}

pub(crate) struct Machine;

impl Host for Machine {
    fn find(&self, candidates: &[&str]) -> Option<String> {
        exec::find(candidates)
    }
    fn run(
        &self,
        program: &str,
        args: &[&str],
        limit: Duration,
        env: &[(&str, &str)],
    ) -> Option<Output> {
        exec::run_with(program, args, limit, None, env)
    }
    fn partition_start(&self, name: &str) -> Option<u64> {
        if name.is_empty() || name.contains('/') || name.starts_with('.') {
            return None;
        }
        std::fs::read_to_string(format!("/sys/class/block/{name}/start"))
            .ok()?
            .trim()
            .parse()
            .ok()
    }
}

// ----------------------------------------------------------------- parsing

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Mount {
    pub source: String,
    pub target: String,
    pub fstype: String,
    pub size: u64,
}

/// A number that `lsblk` and `findmnt` print as a JSON number or, in older versions, a string.
fn num(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn text(v: &Value) -> Option<String> {
    v.as_str().map(str::to_owned).filter(|s| !s.is_empty())
}

/// `findmnt -J -b -T <path> -o SOURCE,TARGET,FSTYPE,SIZE`.
pub(crate) fn parse_findmnt(json: &str) -> Result<Vec<Mount>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("findmnt output: {e}"))?;
    let list = v
        .get("filesystems")
        .and_then(Value::as_array)
        .ok_or("findmnt output has no filesystems")?;
    let mut out = Vec::new();
    for f in list {
        let (Some(source), Some(target)) = (
            f.get("source").and_then(text),
            f.get("target").and_then(text),
        ) else {
            continue;
        };
        out.push(Mount {
            // btrfs prints `/dev/sda2[/@rootfs]`: the device is the part before the bracket.
            source: source.split('[').next().unwrap_or("").to_owned(),
            target,
            fstype: f.get("fstype").and_then(text).unwrap_or_default(),
            size: f.get("size").and_then(num).unwrap_or(0),
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Block {
    pub name: String,
    pub path: String,
    /// `disk`, `part`, `lvm`, `crypt`, `raid1`, `loop`, ...
    pub kind: String,
    pub size: u64,
    pub partn: Option<u32>,
    pub mounts: Vec<String>,
    pub parent: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Tree {
    pub blocks: Vec<Block>,
}

impl Tree {
    fn children(&self, i: usize) -> impl Iterator<Item = (usize, &Block)> {
        self.blocks
            .iter()
            .enumerate()
            .filter(move |(_, b)| b.parent == Some(i))
    }
}

/// `lsblk -J -b -o NAME,PATH,TYPE,SIZE,PARTN,MOUNTPOINTS` (or `MOUNTPOINT` on old versions).
/// A device with several parents (a volume spread over two disks) appears once under each.
pub(crate) fn parse_lsblk(json: &str) -> Result<Tree, String> {
    fn walk(list: &[Value], parent: Option<usize>, out: &mut Vec<Block>) {
        for n in list {
            let Some(name) = n.get("name").and_then(text) else {
                continue;
            };
            let mut mounts: Vec<String> = n
                .get("mountpoints")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(text).collect())
                .unwrap_or_default();
            if let Some(m) = n.get("mountpoint").and_then(text) {
                mounts.push(m);
            }
            let idx = out.len();
            out.push(Block {
                path: n
                    .get("path")
                    .and_then(text)
                    .unwrap_or_else(|| format!("/dev/{name}")),
                name,
                kind: n.get("type").and_then(text).unwrap_or_default(),
                size: n.get("size").and_then(num).unwrap_or(0),
                partn: n
                    .get("partn")
                    .and_then(num)
                    .and_then(|p| u32::try_from(p).ok()),
                mounts,
                parent,
            });
            if let Some(kids) = n.get("children").and_then(Value::as_array) {
                walk(kids, Some(idx), out);
            }
        }
    }
    let v: Value = serde_json::from_str(json).map_err(|e| format!("lsblk output: {e}"))?;
    let list = v
        .get("blockdevices")
        .and_then(Value::as_array)
        .ok_or("lsblk output has no blockdevices")?;
    let mut blocks = Vec::new();
    walk(list, None, &mut blocks);
    Ok(Tree { blocks })
}

// ---------------------------------------------------------------- analysis

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fs {
    Ext,
    Xfs,
    Btrfs,
}

fn fs_kind(fstype: &str) -> Option<Fs> {
    match fstype {
        "ext2" | "ext3" | "ext4" => Some(Fs::Ext),
        "xfs" => Some(Fs::Xfs),
        "btrfs" => Some(Fs::Btrfs),
        _ => None,
    }
}

/// A volume plus what the steps need to build their commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Detail {
    pub volume: DiskVolume,
    fs: Option<Fs>,
    partn: Option<u32>,
}

/// Slack inside a container that is not growable space: filesystem metadata and the
/// journal take a few percent, so only a large gap means "the filesystem was not grown".
fn slack(container: u64, used: u64) -> u64 {
    let gap = container.saturating_sub(used);
    if gap >= GIB.max(container / 10) {
        gap
    } else {
        0
    }
}

fn refuse(mut d: Detail, why: impl Into<String>) -> Detail {
    d.volume.refusal = Some(why.into());
    d.volume.growable = false;
    d.volume.steps.clear();
    d
}

/// Decide what is under `m` and what growing it would take. `starts` maps partition names to
/// their first sector.
pub(crate) fn analyze(
    m: &Mount,
    tree: &Tree,
    starts: &HashMap<String, u64>,
    growpart_installed: bool,
    growpart_installable: bool,
) -> Detail {
    let mut d = Detail {
        volume: DiskVolume {
            mount: m.target.clone(),
            device: m.source.clone(),
            fs: m.fstype.clone(),
            layout: DiskLayout::Other,
            partition: None,
            disk: None,
            fs_bytes: m.size,
            partition_bytes: None,
            disk_bytes: None,
            growable_bytes: 0,
            growable: false,
            steps: vec![],
            refusal: None,
        },
        fs: fs_kind(&m.fstype),
        partn: None,
    };
    let Some(fs) = d.fs else {
        let what = if m.fstype.is_empty() {
            "an unknown filesystem".to_owned()
        } else {
            m.fstype.clone()
        };
        return refuse(
            d,
            format!(
                "This volume uses {what}. opsbox grows ext4, xfs and btrfs only; grow it by hand."
            ),
        );
    };

    let hits: Vec<usize> = {
        let by_mount: Vec<usize> = tree
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.mounts.iter().any(|x| x == &m.target))
            .map(|(i, _)| i)
            .collect();
        if by_mount.is_empty() {
            tree.blocks
                .iter()
                .enumerate()
                .filter(|(_, b)| b.path == m.source)
                .map(|(i, _)| i)
                .collect()
        } else {
            by_mount
        }
    };
    let node = match hits.as_slice() {
        [one] => *one,
        [] => {
            return refuse(
                d,
                "opsbox cannot tell which disk this volume is on. Grow it by hand.",
            );
        }
        _ => {
            return refuse(
                d,
                "This volume spreads over more than one disk. Grow it by hand.",
            );
        }
    };
    let n = &tree.blocks[node];
    d.volume.device = n.path.clone();

    // The container whose end can move: the partition, or the whole disk.
    // `container` = (index, is a partition). For LVM it is the physical volume.
    let (container, lv_size) = match n.kind.as_str() {
        "part" | "disk" => (node, None),
        "lvm" => {
            let Some(pv) = n.parent else {
                return refuse(d, "opsbox cannot find the disk under this volume.");
            };
            if !matches!(tree.blocks[pv].kind.as_str(), "part" | "disk") {
                return refuse(
                    d,
                    format!(
                        "This volume sits on {} storage. opsbox does not grow that; grow it by hand.",
                        tree.blocks[pv].kind
                    ),
                );
            }
            if fs == Fs::Btrfs {
                return refuse(
                    d,
                    "btrfs inside LVM is not grown automatically. Grow it by hand.",
                );
            }
            (pv, Some(n.size))
        }
        other => {
            return refuse(
                d,
                format!(
                    "This volume is on {other} storage. opsbox does not grow that; grow it by hand."
                ),
            );
        }
    };
    let c = &tree.blocks[container];
    let on_partition = c.kind == "part";
    let disk_idx = if on_partition {
        c.parent
    } else {
        Some(container)
    };
    let Some(disk_idx) = disk_idx.filter(|i| tree.blocks[*i].kind == "disk") else {
        return refuse(d, "opsbox cannot find the disk under this volume.");
    };
    let disk = &tree.blocks[disk_idx];
    d.volume.layout = if lv_size.is_some() {
        DiskLayout::Lvm
    } else if on_partition {
        DiskLayout::Partition
    } else {
        DiskLayout::Disk
    };
    d.volume.disk = Some(disk.path.clone());
    d.volume.disk_bytes = Some(disk.size);
    if on_partition || lv_size.is_some() {
        d.volume.partition = Some(c.path.clone());
        d.volume.partition_bytes = Some(c.size);
    }
    if on_partition {
        d.partn = c.partn;
    }

    // Free space after the partition: only if it is the last one on the disk.
    let tail_free = if on_partition {
        let mine = starts.get(&c.name).copied();
        let last = tree
            .children(disk_idx)
            .filter(|(_, b)| b.kind == "part")
            .all(|(_, b)| match (mine, starts.get(&b.name)) {
                (Some(m), Some(o)) => *o <= m,
                _ => false,
            });
        match (mine, last) {
            (Some(start), true) => {
                let end = (start + c.size / 512) * 512;
                disk.size.saturating_sub(end)
            }
            _ => 0,
        }
    } else {
        0
    };
    let tail_free = if tail_free >= MIN_GROW { tail_free } else { 0 };

    let mut steps = Vec::new();
    let mut growable = tail_free;
    if tail_free > 0 {
        steps.push(DiskStep::GrowPartition);
    }
    match lv_size {
        Some(lv) => {
            // What the physical volume has that no logical volume uses.
            let used: u64 = tree
                .children(container)
                .filter(|(_, b)| b.kind == "lvm")
                .map(|(_, b)| b.size)
                .sum();
            let pv_free = c.size.saturating_sub(used);
            let pv_free = if pv_free >= MIN_GROW { pv_free } else { 0 };
            let fs_gap = slack(lv, m.size);
            growable += pv_free + fs_gap;
            if tail_free > 0 || pv_free > 0 {
                steps.push(DiskStep::ResizePv);
            }
            if tail_free > 0 || pv_free > 0 || fs_gap > 0 {
                steps.push(DiskStep::ResizeFs);
            }
        }
        None => {
            let fs_gap = slack(if on_partition { c.size } else { disk.size }, m.size);
            growable += fs_gap;
            if tail_free > 0 || fs_gap > 0 {
                steps.push(DiskStep::ResizeFs);
            }
        }
    }
    d.volume.growable_bytes = growable;

    if steps.contains(&DiskStep::GrowPartition) && !growpart_installed {
        if growpart_installable {
            steps.insert(0, DiskStep::InstallGrowpart);
        } else {
            return refuse(
                d,
                "The tool that grows partitions (growpart, package cloud-guest-utils) is missing and opsbox cannot install it here. Install it, then try again.",
            );
        }
    }
    d.volume.growable = !steps.is_empty();
    d.volume.steps = steps;
    d
}

// --------------------------------------------------------------- detection

pub(crate) struct Detection {
    pub details: Vec<Detail>,
    pub growpart_installed: bool,
    pub growpart_installable: bool,
}

impl Detection {
    pub fn state(&self) -> DiskState {
        DiskState {
            volumes: self.details.iter().map(|d| d.volume.clone()).collect(),
            growpart_installed: self.growpart_installed,
            growpart_installable: self.growpart_installable,
        }
    }
}

fn read_lsblk(h: &dyn Host) -> Result<Tree, PrivError> {
    let bin = h.find(LSBLK).ok_or_else(|| {
        err(
            PrivErrorCode::Unsupported,
            "lsblk is not available on this machine.",
        )
    })?;
    // Older util-linux has MOUNTPOINT only.
    for cols in [
        "NAME,PATH,TYPE,SIZE,PARTN,MOUNTPOINTS",
        "NAME,PATH,TYPE,SIZE,PARTN,MOUNTPOINT",
    ] {
        let out = h.run(
            &bin,
            &["-J", "-b", "-o", cols],
            Duration::from_secs(20),
            &[],
        );
        if let Some(o) = out.filter(Output::ok) {
            return parse_lsblk(&o.stdout).map_err(|m| err(PrivErrorCode::Internal, m));
        }
    }
    Err(err(
        PrivErrorCode::Internal,
        "Could not read the disk layout (lsblk failed).",
    ))
}

fn read_mount(h: &dyn Host, bin: &str, path: &str) -> Option<Mount> {
    let out = h
        .run(
            bin,
            &["-J", "-b", "-T", path, "-o", "SOURCE,TARGET,FSTYPE,SIZE"],
            Duration::from_secs(20),
            &[],
        )
        .filter(Output::ok)?;
    parse_findmnt(&out.stdout).ok()?.into_iter().next()
}

fn pkg_manager(h: &dyn Host) -> Option<&'static str> {
    if h.find(APT_GET).is_some() {
        Some("apt")
    } else if h.find(DNF).is_some() {
        Some("dnf")
    } else {
        None
    }
}

/// Look at the filesystems holding `paths` (`/` always comes first in the answer).
pub(crate) fn detect(h: &dyn Host, paths: &[String]) -> Result<Detection, PrivError> {
    let tree = read_lsblk(h)?;
    let findmnt = h.find(FINDMNT).ok_or_else(|| {
        err(
            PrivErrorCode::Unsupported,
            "findmnt is not available on this machine.",
        )
    })?;
    let mut mounts: Vec<Mount> = Vec::new();
    for (i, p) in paths.iter().enumerate() {
        match read_mount(h, &findmnt, p) {
            Some(m) if !mounts.iter().any(|x| x.target == m.target) => {
                // Memory-backed and overlay mounts have no disk; only `/` is always reported.
                if m.source.starts_with("/dev/") || (i == 0 && m.target == "/") {
                    mounts.push(m);
                }
            }
            Some(_) => {}
            None if i == 0 => {
                return Err(err(
                    PrivErrorCode::Internal,
                    "Could not read which filesystem holds /.",
                ));
            }
            None => {}
        }
    }
    let starts: HashMap<String, u64> = tree
        .blocks
        .iter()
        .filter(|b| b.kind == "part")
        .filter_map(|b| Some((b.name.clone(), h.partition_start(&b.name)?)))
        .collect();
    let installed = h.find(GROWPART).is_some();
    let installable = pkg_manager(h).is_some();
    let details = mounts
        .iter()
        .map(|m| analyze(m, &tree, &starts, installed, installable))
        .collect();
    Ok(Detection {
        details,
        growpart_installed: installed,
        growpart_installable: installable,
    })
}

// ------------------------------------------------------------------- steps

/// A path that is safe to pass as an argument: absolute, printable, no surprises.
fn arg_ok(p: &str) -> bool {
    p.starts_with('/') && p.len() < 4096 && !p.bytes().any(|b| b < 32 || b == 127)
}

fn missing(what: &str) -> PrivError {
    err(
        PrivErrorCode::Unsupported,
        format!("{what} is not available on this machine."),
    )
}

fn failed(what: &str, o: Option<Output>) -> PrivError {
    match o {
        Some(o) => err(
            PrivErrorCode::Internal,
            format!("{what} failed: {}", tail(&o.combined(), 2).join(" ")),
        ),
        None => err(
            PrivErrorCode::Internal,
            format!("{what} did not finish in time."),
        ),
    }
}

/// Run `step` for the volume found by `detail`. Returns what the tool printed.
pub(crate) fn run_step(h: &dyn Host, d: &Detail, step: DiskStep) -> Result<Vec<String>, PrivError> {
    let v = &d.volume;
    if let Some(why) = &v.refusal {
        return Err(err(PrivErrorCode::Invalid, why.clone()));
    }
    let long = Duration::from_secs(20 * 60);
    match step {
        DiskStep::InstallGrowpart => {
            if h.find(GROWPART).is_some() {
                return Ok(vec!["growpart is already installed.".into()]);
            }
            let env = [
                ("DEBIAN_FRONTEND", "noninteractive"),
                ("NEEDRESTART_MODE", "a"),
            ];
            match pkg_manager(h) {
                Some("apt") => {
                    let apt = h.find(APT_GET).ok_or_else(|| missing("apt-get"))?;
                    let upd = h.run(&apt, &["-q", "update"], Duration::from_secs(300), &env);
                    if !upd.as_ref().is_some_and(Output::ok) {
                        return Err(failed("Refreshing the package lists", upd));
                    }
                    let o = h.run(
                        &apt,
                        &["-q", "-y", "install", "cloud-guest-utils"],
                        long,
                        &env,
                    );
                    match o {
                        Some(o) if o.ok() => Ok(tail(&o.combined(), 2)),
                        o => Err(failed("Installing cloud-guest-utils", o)),
                    }
                }
                Some(_) => {
                    let dnf = h.find(DNF).ok_or_else(|| missing("dnf"))?;
                    let o = h.run(&dnf, &["-y", "install", "cloud-utils-growpart"], long, &env);
                    match o {
                        Some(o) if o.ok() => Ok(tail(&o.combined(), 2)),
                        o => Err(failed("Installing cloud-utils-growpart", o)),
                    }
                }
                None => Err(err(
                    PrivErrorCode::Unsupported,
                    "No supported package manager (apt or dnf) was found on this machine.",
                )),
            }
        }
        DiskStep::GrowPartition => {
            let (Some(disk), Some(n)) = (v.disk.as_deref(), d.partn) else {
                return Err(err(
                    PrivErrorCode::Invalid,
                    "This volume has no partition to grow.",
                ));
            };
            if !arg_ok(disk) {
                return Err(err(PrivErrorCode::Invalid, "Unexpected disk name."));
            }
            let bin = h.find(GROWPART).ok_or_else(|| missing("growpart"))?;
            let num = n.to_string();
            let o = h.run(&bin, &[disk, &num], Duration::from_secs(120), &[]);
            match o {
                Some(o) if o.ok() => Ok(tail(&o.combined(), 2)),
                // growpart exits 1 with NOCHANGE when the partition already fills the disk.
                Some(o) if o.code == Some(1) && o.combined().contains("NOCHANGE") => {
                    Ok(vec!["The partition already fills the disk.".into()])
                }
                o => Err(failed("Growing the partition", o)),
            }
        }
        DiskStep::ResizePv => {
            if v.layout != DiskLayout::Lvm {
                return Err(err(PrivErrorCode::Invalid, "This volume does not use LVM."));
            }
            let pv = v.partition.as_deref().unwrap_or_default();
            if !arg_ok(pv) {
                return Err(err(PrivErrorCode::Invalid, "Unexpected device name."));
            }
            let bin = h.find(PVRESIZE).ok_or_else(|| missing("pvresize"))?;
            let o = h.run(&bin, &[pv], Duration::from_secs(120), &[]);
            match o {
                Some(o) if o.ok() => Ok(tail(&o.combined(), 2)),
                o => Err(failed("Resizing the LVM physical volume", o)),
            }
        }
        DiskStep::ResizeFs => {
            let Some(fs) = d.fs else {
                return Err(err(PrivErrorCode::Invalid, "Unsupported filesystem."));
            };
            if !arg_ok(&v.device) || !arg_ok(&v.mount) {
                return Err(err(PrivErrorCode::Invalid, "Unexpected device name."));
            }
            if v.layout == DiskLayout::Lvm {
                let bin = h.find(LVEXTEND).ok_or_else(|| missing("lvextend"))?;
                let o = h.run(&bin, &["-r", "-l", "+100%FREE", &v.device], long, &[]);
                return match o {
                    // Nothing free to add: the volume is already as big as it can be.
                    Some(o) if !o.ok() && o.combined().contains("matches existing size") => {
                        Ok(vec!["The volume already uses all free space.".into()])
                    }
                    Some(o) if o.ok() => Ok(tail(&o.combined(), 2)),
                    o => Err(failed("Growing the logical volume", o)),
                };
            }
            let (bin, args): (String, Vec<&str>) = match fs {
                Fs::Ext => (
                    h.find(RESIZE2FS).ok_or_else(|| missing("resize2fs"))?,
                    vec![v.device.as_str()],
                ),
                Fs::Xfs => (
                    h.find(XFS_GROWFS).ok_or_else(|| missing("xfs_growfs"))?,
                    vec![v.mount.as_str()],
                ),
                Fs::Btrfs => (
                    h.find(BTRFS).ok_or_else(|| missing("btrfs"))?,
                    vec!["filesystem", "resize", "max", v.mount.as_str()],
                ),
            };
            let o = h.run(&bin, &args, long, &[]);
            match o {
                Some(o) if o.ok() => Ok(tail(&o.combined(), 2)),
                o => Err(failed("Growing the filesystem", o)),
            }
        }
    }
}

// ---------------------------------------------------------------- the ops

/// `/`, the opsbox data directory and Docker's directory.
fn watched_paths(env: &Env) -> Vec<String> {
    let mut paths = vec!["/".to_owned()];
    let data = env
        .db_path
        .parent()
        .map(|p| p.to_string_lossy().into_owned());
    for p in data.into_iter().chain([
        env.apps_dir.to_string_lossy().into_owned(),
        "/var/lib/docker".to_owned(),
    ]) {
        if std::path::Path::new(&p).exists() && !paths.contains(&p) {
            paths.push(p);
        }
    }
    paths
}

pub(super) fn state(env: &Env) -> Result<PrivReply, PrivError> {
    let d = detect(&Machine, &watched_paths(env))?;
    Ok(PrivReply::Disk(Box::new(d.state())))
}

pub(super) fn grow(env: &Env, mount: &str, step: DiskStep) -> Result<PrivReply, PrivError> {
    grow_with(&Machine, &watched_paths(env), mount, step)
}

/// Re-detect, check that `step` makes sense for what is mounted at `mount` now, run it.
pub(crate) fn grow_with(
    h: &dyn Host,
    paths: &[String],
    mount: &str,
    step: DiskStep,
) -> Result<PrivReply, PrivError> {
    let det = detect(h, paths)?;
    let d = det
        .details
        .iter()
        .find(|d| d.volume.mount == mount)
        .ok_or_else(|| {
            err(
                PrivErrorCode::Invalid,
                "That is not a volume opsbox can grow.",
            )
        })?;
    run_step(h, d, step).map(|log| PrivReply::DiskStep { log })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    const GIB: u64 = 1 << 30;

    // Ubuntu cloud image, disk enlarged from 20 to 30 GiB: root is the last partition.
    const UBUNTU_EXT4: &str = r#"{"blockdevices":[
      {"name":"sda","path":"/dev/sda","type":"disk","size":32212254720,"partn":null,"mountpoints":[null],"children":[
        {"name":"sda1","path":"/dev/sda1","type":"part","size":21237088256,"partn":1,"mountpoints":["/"]},
        {"name":"sda14","path":"/dev/sda14","type":"part","size":4194304,"partn":14,"mountpoints":[null]},
        {"name":"sda15","path":"/dev/sda15","type":"part","size":111149056,"partn":15,"mountpoints":["/boot/efi"]}]}]}"#;
    const UBUNTU_STARTS: &[(&str, u64)] = &[("sda1", 227_328), ("sda14", 2048), ("sda15", 10_240)];

    // Amazon Linux on NVMe: xfs on nvme0n1p1, 100 GiB disk, root partition 50 GiB.
    const NVME_XFS: &str = r#"{"blockdevices":[
      {"name":"nvme0n1","path":"/dev/nvme0n1","type":"disk","size":107374182400,"partn":null,"mountpoints":[null],"children":[
        {"name":"nvme0n1p1","path":"/dev/nvme0n1p1","type":"part","size":53687091200,"partn":1,"mountpoints":["/"]},
        {"name":"nvme0n1p127","path":"/dev/nvme0n1p127","type":"part","size":1048576,"partn":127,"mountpoints":[null]},
        {"name":"nvme0n1p128","path":"/dev/nvme0n1p128","type":"part","size":10485760,"partn":128,"mountpoints":["/boot/efi"]}]}]}"#;
    const NVME_STARTS: &[(&str, u64)] = &[
        ("nvme0n1p1", 24_576),
        ("nvme0n1p127", 22_528),
        ("nvme0n1p128", 2048),
    ];

    // Ubuntu Server with LVM: 50 GiB disk, PV partition still 20 GiB, LV 10 GiB.
    const LVM: &str = r#"{"blockdevices":[
      {"name":"sda","path":"/dev/sda","type":"disk","size":53687091200,"partn":null,"mountpoints":[null],"children":[
        {"name":"sda1","path":"/dev/sda1","type":"part","size":1048576,"partn":1,"mountpoints":[null]},
        {"name":"sda2","path":"/dev/sda2","type":"part","size":2147483648,"partn":2,"mountpoints":["/boot"]},
        {"name":"sda3","path":"/dev/sda3","type":"part","size":21474836480,"partn":3,"mountpoints":[null],"children":[
          {"name":"ubuntu--vg-ubuntu--lv","path":"/dev/mapper/ubuntu--vg-ubuntu--lv","type":"lvm","size":10737418240,"partn":null,"mountpoints":["/"]}]}]}]}"#;
    const LVM_STARTS: &[(&str, u64)] = &[("sda1", 2048), ("sda2", 4096), ("sda3", 4_198_400)];

    // Extra data disk with the filesystem straight on it (no partition table).
    const WHOLE_DISK: &str = r#"{"blockdevices":[
      {"name":"sda","path":"/dev/sda","type":"disk","size":32212254720,"partn":null,"mountpoints":[null],"children":[
        {"name":"sda1","path":"/dev/sda1","type":"part","size":21237088256,"partn":1,"mountpoints":["/"]}]},
      {"name":"sdb","path":"/dev/sdb","type":"disk","size":107374182400,"partn":null,"mountpoints":["/var/lib/docker"]}]}"#;

    fn mount(source: &str, target: &str, fstype: &str, size: u64) -> Mount {
        Mount {
            source: source.into(),
            target: target.into(),
            fstype: fstype.into(),
            size,
        }
    }

    fn starts(list: &[(&str, u64)]) -> HashMap<String, u64> {
        list.iter().map(|(n, s)| ((*n).to_owned(), *s)).collect()
    }

    fn analyze_with(m: &Mount, json: &str, st: &[(&str, u64)]) -> Detail {
        analyze(m, &parse_lsblk(json).unwrap(), &starts(st), true, true)
    }

    // ------------------------------------------------------------- parsing

    #[test]
    fn findmnt_numbers_and_btrfs_subvolume_sources() {
        let ms = parse_findmnt(
            r#"{"filesystems":[{"source":"/dev/sda2[/@rootfs]","target":"/","fstype":"btrfs","size":"21474836480"}]}"#,
        )
        .unwrap();
        assert_eq!(ms, vec![mount("/dev/sda2", "/", "btrfs", 21_474_836_480)]);
        let ms = parse_findmnt(
            r#"{"filesystems":[{"source":"/dev/nvme0n1p1","target":"/","fstype":"xfs","size":53617000000}]}"#,
        )
        .unwrap();
        assert_eq!(ms[0].source, "/dev/nvme0n1p1");
        assert!(parse_findmnt("not json").is_err());
        assert!(parse_findmnt("{}").is_err());
    }

    #[test]
    fn lsblk_tree_handles_nvme_names_and_old_string_format() {
        let t = parse_lsblk(NVME_XFS).unwrap();
        let p1 = t.blocks.iter().find(|b| b.name == "nvme0n1p1").unwrap();
        assert_eq!((p1.partn, p1.kind.as_str()), (Some(1), "part"));
        assert_eq!(p1.mounts, vec!["/"]);
        assert_eq!(t.blocks[p1.parent.unwrap()].path, "/dev/nvme0n1");
        // util-linux 2.31: sizes as strings, one `mountpoint`, no `path` column.
        let old = parse_lsblk(
            r#"{"blockdevices":[{"name":"vda","type":"disk","size":"10737418240","partn":null,"mountpoint":null,
            "children":[{"name":"vda1","type":"part","size":"10736369664","partn":"1","mountpoint":"/"}]}]}"#,
        )
        .unwrap();
        assert_eq!(old.blocks[1].size, 10_736_369_664);
        assert_eq!(old.blocks[1].path, "/dev/vda1");
        assert_eq!(old.blocks[1].partn, Some(1));
        assert_eq!(old.blocks[1].mounts, vec!["/"]);
        assert!(parse_lsblk("[]").is_err());
    }

    // ------------------------------------------------------------ analysis

    #[test]
    fn ext4_on_a_partition_with_a_bigger_disk() {
        let m = mount("/dev/sda1", "/", "ext4", 20_767_420_416);
        let d = analyze_with(&m, UBUNTU_EXT4, UBUNTU_STARTS).volume;
        assert_eq!(d.layout, DiskLayout::Partition);
        assert_eq!(d.partition.as_deref(), Some("/dev/sda1"));
        assert_eq!(d.disk.as_deref(), Some("/dev/sda"));
        assert_eq!(d.disk_bytes, Some(32_212_254_720));
        // 30 GiB disk minus the end of sda1.
        assert_eq!(d.growable_bytes, 10_858_774_528);
        assert!(d.growable && d.refusal.is_none());
        assert_eq!(d.steps, vec![DiskStep::GrowPartition, DiskStep::ResizeFs]);
    }

    #[test]
    fn xfs_on_nvme_partition() {
        let m = mount("/dev/nvme0n1p1", "/", "xfs", 53_617_000_000);
        let d = analyze_with(&m, NVME_XFS, NVME_STARTS).volume;
        assert_eq!(d.layout, DiskLayout::Partition);
        assert_eq!(d.device, "/dev/nvme0n1p1");
        assert_eq!(d.disk.as_deref(), Some("/dev/nvme0n1"));
        assert_eq!(d.growable_bytes, 53_674_508_288);
        assert_eq!(d.steps, vec![DiskStep::GrowPartition, DiskStep::ResizeFs]);
    }

    #[test]
    fn lvm_grows_partition_then_pv_then_lv() {
        let m = mount(
            "/dev/mapper/ubuntu--vg-ubuntu--lv",
            "/",
            "ext4",
            10_447_081_472,
        );
        let d = analyze_with(&m, LVM, LVM_STARTS).volume;
        assert_eq!(d.layout, DiskLayout::Lvm);
        assert_eq!(d.partition.as_deref(), Some("/dev/sda3"));
        assert_eq!(d.disk.as_deref(), Some("/dev/sda"));
        // 30 GiB behind the partition plus 10 GiB of the physical volume no LV uses.
        assert_eq!(d.growable_bytes, 30_062_673_920 + 10 * GIB);
        assert_eq!(
            d.steps,
            vec![
                DiskStep::GrowPartition,
                DiskStep::ResizePv,
                DiskStep::ResizeFs
            ]
        );
    }

    #[test]
    fn lvm_with_free_group_space_needs_no_partition_step() {
        // Partition already grown by hand to the end of the disk; the LV is 10 GiB of the
        // 40 GiB physical volume.
        let json = LVM
            .replace("21474836480", "42949672960")
            .replace("53687091200", "45100302336");
        let m = mount(
            "/dev/mapper/ubuntu--vg-ubuntu--lv",
            "/",
            "ext4",
            10_447_081_472,
        );
        let d = analyze_with(&m, &json, LVM_STARTS).volume;
        assert_eq!(d.growable_bytes, 42_949_672_960 - 10 * GIB);
        assert_eq!(d.steps, vec![DiskStep::ResizePv, DiskStep::ResizeFs]);
    }

    #[test]
    fn filesystem_directly_on_a_disk() {
        let m = mount("/dev/sdb", "/var/lib/docker", "ext4", 53_000_000_000);
        let d = analyze_with(&m, WHOLE_DISK, UBUNTU_STARTS).volume;
        assert_eq!(d.layout, DiskLayout::Disk);
        assert_eq!(
            (d.partition.clone(), d.disk.as_deref()),
            (None, Some("/dev/sdb"))
        );
        assert_eq!(d.growable_bytes, 107_374_182_400 - 53_000_000_000);
        assert_eq!(d.steps, vec![DiskStep::ResizeFs]);
    }

    #[test]
    fn nothing_to_grow_when_the_disk_is_not_larger() {
        // Same disk as the ext4 fixture before it was enlarged: only GPT bookkeeping is left.
        let json = UBUNTU_EXT4.replace("32212254720", "21355577344");
        let m = mount("/dev/sda1", "/", "ext4", 20_767_420_416);
        let d = analyze_with(&m, &json, UBUNTU_STARTS).volume;
        assert_eq!((d.growable, d.growable_bytes), (false, 0));
        assert!(d.steps.is_empty() && d.refusal.is_none());
    }

    #[test]
    fn a_partition_that_is_not_last_cannot_grow_into_the_disk() {
        let json = UBUNTU_EXT4;
        let m = mount("/dev/sda1", "/", "ext4", 20_767_420_416);
        // sda15 now starts after sda1.
        let d = analyze_with(
            &m,
            json,
            &[("sda1", 227_328), ("sda14", 2048), ("sda15", 99_999_999)],
        )
        .volume;
        assert_eq!((d.growable, d.growable_bytes), (false, 0));
        // Unknown start sectors are treated the same way: do nothing we cannot check.
        let d = analyze_with(&m, json, &[]).volume;
        assert!(!d.growable);
    }

    #[test]
    fn a_partition_grown_without_the_filesystem_is_still_growable() {
        let json = UBUNTU_EXT4.replace("32212254720", "21355577344");
        let m = mount("/dev/sda1", "/", "ext4", 8 * GIB);
        let d = analyze_with(&m, &json, UBUNTU_STARTS).volume;
        assert_eq!(d.steps, vec![DiskStep::ResizeFs]);
        assert!(d.growable_bytes > 10 * GIB);
    }

    #[test]
    fn unknown_filesystems_are_refused_with_a_reason() {
        for fstype in ["zfs", "f2fs", "ntfs", ""] {
            let m = mount("/dev/sda1", "/", fstype, 20 * GIB);
            let d = analyze_with(&m, UBUNTU_EXT4, UBUNTU_STARTS).volume;
            assert!(!d.growable && d.steps.is_empty(), "{fstype}");
            let why = d.refusal.expect("a reason");
            assert!(why.contains("ext4, xfs and btrfs"), "{why}");
        }
        for ok in ["ext2", "ext3", "ext4", "xfs", "btrfs"] {
            let m = mount("/dev/sda1", "/", ok, 20_767_420_416);
            assert!(
                analyze_with(&m, UBUNTU_EXT4, UBUNTU_STARTS)
                    .volume
                    .refusal
                    .is_none(),
                "{ok}"
            );
        }
    }

    #[test]
    fn encrypted_raid_and_multi_disk_layouts_are_refused() {
        let crypt = UBUNTU_EXT4.replace(
            r#""type":"part","size":21237088256"#,
            r#""type":"crypt","size":21237088256"#,
        );
        let m = mount("/dev/sda1", "/", "ext4", 20_767_420_416);
        let d = analyze_with(&m, &crypt, UBUNTU_STARTS).volume;
        assert!(!d.growable);
        assert!(d.refusal.unwrap().contains("crypt"));

        let two = r#"{"blockdevices":[
          {"name":"sda","path":"/dev/sda","type":"disk","size":53687091200,"children":[
            {"name":"sda3","path":"/dev/sda3","type":"part","size":21474836480,"partn":3,"children":[
              {"name":"vg-root","path":"/dev/mapper/vg-root","type":"lvm","size":40000000000,"mountpoints":["/"]}]}]},
          {"name":"sdb","path":"/dev/sdb","type":"disk","size":21474836480,"children":[
            {"name":"vg-root","path":"/dev/mapper/vg-root","type":"lvm","size":40000000000,"mountpoints":["/"]}]}]}"#;
        let m = mount("/dev/mapper/vg-root", "/", "ext4", 39_000_000_000);
        let d = analyze_with(&m, two, &[("sda3", 2048)]).volume;
        assert!(!d.growable);
        assert!(d.refusal.unwrap().contains("more than one disk"));

        let lvm_btrfs = mount("/dev/mapper/ubuntu--vg-ubuntu--lv", "/", "btrfs", 10 * GIB);
        let d = analyze_with(&lvm_btrfs, LVM, LVM_STARTS).volume;
        assert!(!d.growable && d.refusal.unwrap().contains("btrfs inside LVM"));

        let ghost = mount("/dev/vdz9", "/srv", "ext4", GIB);
        let d = analyze_with(&ghost, LVM, LVM_STARTS).volume;
        assert!(!d.growable && d.refusal.unwrap().contains("cannot tell"));
    }

    #[test]
    fn missing_growpart_is_installed_first_or_refused() {
        let m = mount("/dev/sda1", "/", "ext4", 20_767_420_416);
        let t = parse_lsblk(UBUNTU_EXT4).unwrap();
        let st = starts(UBUNTU_STARTS);
        let d = analyze(&m, &t, &st, false, true).volume;
        assert_eq!(
            d.steps,
            vec![
                DiskStep::InstallGrowpart,
                DiskStep::GrowPartition,
                DiskStep::ResizeFs
            ]
        );
        let d = analyze(&m, &t, &st, false, false).volume;
        assert!(!d.growable && d.refusal.unwrap().contains("growpart"));
        // A bare disk never needs growpart.
        let b = mount("/dev/sdb", "/var/lib/docker", "ext4", 53_000_000_000);
        let d = analyze(&b, &parse_lsblk(WHOLE_DISK).unwrap(), &st, false, false).volume;
        assert_eq!(d.steps, vec![DiskStep::ResizeFs]);
    }

    // ---------------------------------------------------- fake machine, ops

    struct Fake {
        lsblk: String,
        mounts: Vec<(String, String)>,
        starts: HashMap<String, u64>,
        apt: bool,
        growpart: Cell<bool>,
        calls: RefCell<Vec<String>>,
        results: RefCell<HashMap<&'static str, (i32, &'static str)>>,
    }

    impl Fake {
        fn new(lsblk: &str, mounts: &[(&str, String)], st: &[(&str, u64)]) -> Self {
            Self {
                lsblk: lsblk.into(),
                mounts: mounts
                    .iter()
                    .map(|(p, j)| ((*p).to_owned(), j.clone()))
                    .collect(),
                starts: starts(st),
                apt: true,
                growpart: Cell::new(true),
                calls: RefCell::default(),
                results: RefCell::default(),
            }
        }
        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    fn fm(source: &str, target: &str, fstype: &str, size: u64) -> String {
        format!(
            r#"{{"filesystems":[{{"source":"{source}","target":"{target}","fstype":"{fstype}","size":{size}}}]}}"#
        )
    }

    fn base(p: &str) -> &str {
        p.rsplit('/').next().unwrap_or(p)
    }

    impl Host for Fake {
        fn find(&self, candidates: &[&str]) -> Option<String> {
            let first = candidates[0];
            if candidates == GROWPART && !self.growpart.get() {
                return None;
            }
            if candidates == DNF || (candidates == APT_GET && !self.apt) {
                return None;
            }
            Some(first.to_owned())
        }
        fn run(
            &self,
            program: &str,
            args: &[&str],
            _l: Duration,
            _e: &[(&str, &str)],
        ) -> Option<Output> {
            let name = base(program);
            let ok = |s: &str| {
                Some(Output {
                    code: Some(0),
                    stdout: s.into(),
                    stderr: String::new(),
                })
            };
            match name {
                "lsblk" => return ok(&self.lsblk),
                "findmnt" => {
                    let path = args[args.iter().position(|a| *a == "-T").unwrap() + 1];
                    return self
                        .mounts
                        .iter()
                        .find(|(p, _)| p == path)
                        .and_then(|(_, j)| ok(j));
                }
                _ => {}
            }
            self.calls
                .borrow_mut()
                .push(format!("{name} {}", args.join(" ")));
            if name == "apt-get" && args.contains(&"install") {
                self.growpart.set(true);
            }
            match self.results.borrow().get(name) {
                Some((code, out)) => Some(Output {
                    code: Some(*code),
                    stdout: (*out).into(),
                    stderr: String::new(),
                }),
                None => ok("done"),
            }
        }
        fn partition_start(&self, name: &str) -> Option<u64> {
            self.starts.get(name).copied()
        }
    }

    /// Run every step the plan lists for `mount`, one typed step at a time.
    fn run_plan(h: &Fake, paths: &[&str], mount: &str) -> Result<(), PrivError> {
        let paths: Vec<String> = paths.iter().map(|p| (*p).to_owned()).collect();
        let steps = detect(h, &paths)?
            .details
            .iter()
            .find(|d| d.volume.mount == mount)
            .unwrap()
            .volume
            .steps
            .clone();
        for s in steps {
            grow_with(h, &paths, mount, s)?;
        }
        Ok(())
    }

    #[test]
    fn ext4_sequence() {
        let h = Fake::new(
            UBUNTU_EXT4,
            &[("/", fm("/dev/sda1", "/", "ext4", 20_767_420_416))],
            UBUNTU_STARTS,
        );
        run_plan(&h, &["/"], "/").unwrap();
        assert_eq!(
            h.calls(),
            vec!["growpart /dev/sda 1", "resize2fs /dev/sda1"]
        );
    }

    #[test]
    fn xfs_nvme_sequence_uses_the_mount_point() {
        let h = Fake::new(
            NVME_XFS,
            &[("/", fm("/dev/nvme0n1p1", "/", "xfs", 53_617_000_000))],
            NVME_STARTS,
        );
        run_plan(&h, &["/"], "/").unwrap();
        assert_eq!(h.calls(), vec!["growpart /dev/nvme0n1 1", "xfs_growfs /"]);
    }

    #[test]
    fn btrfs_sequence() {
        let h = Fake::new(
            UBUNTU_EXT4,
            &[("/", fm("/dev/sda1[/@rootfs]", "/", "btrfs", 20_767_420_416))],
            UBUNTU_STARTS,
        );
        run_plan(&h, &["/"], "/").unwrap();
        assert_eq!(
            h.calls(),
            vec!["growpart /dev/sda 1", "btrfs filesystem resize max /"]
        );
    }

    #[test]
    fn lvm_sequence() {
        let h = Fake::new(
            LVM,
            &[(
                "/",
                fm(
                    "/dev/mapper/ubuntu--vg-ubuntu--lv",
                    "/",
                    "ext4",
                    10_447_081_472,
                ),
            )],
            LVM_STARTS,
        );
        run_plan(&h, &["/"], "/").unwrap();
        assert_eq!(
            h.calls(),
            vec![
                "growpart /dev/sda 3",
                "pvresize /dev/sda3",
                "lvextend -r -l +100%FREE /dev/mapper/ubuntu--vg-ubuntu--lv",
            ]
        );
    }

    #[test]
    fn whole_disk_on_another_mount_and_dedupe_of_shared_mounts() {
        let h = Fake::new(
            WHOLE_DISK,
            &[
                ("/", fm("/dev/sda1", "/", "ext4", 20_767_420_416)),
                ("/data", fm("/dev/sda1", "/", "ext4", 20_767_420_416)),
                (
                    "/var/lib/docker",
                    fm("/dev/sdb", "/var/lib/docker", "ext4", 53_000_000_000),
                ),
            ],
            UBUNTU_STARTS,
        );
        let paths: Vec<String> = ["/", "/data", "/var/lib/docker", "/gone"]
            .map(String::from)
            .into();
        let st = detect(&h, &paths).unwrap().state();
        let mounts: Vec<_> = st.volumes.iter().map(|v| v.mount.as_str()).collect();
        assert_eq!(mounts, vec!["/", "/var/lib/docker"]);
        run_plan(&h, &["/var/lib/docker"], "/var/lib/docker").unwrap();
        assert_eq!(h.calls(), vec!["resize2fs /dev/sdb"]);
    }

    #[test]
    fn overlay_mounts_are_left_out_but_root_is_always_reported() {
        let h = Fake::new(
            UBUNTU_EXT4,
            &[
                ("/", fm("/dev/sda1", "/", "ext4", 20_767_420_416)),
                ("/data", fm("overlay", "/data", "overlay", GIB)),
            ],
            UBUNTU_STARTS,
        );
        let paths: Vec<String> = ["/", "/data"].map(String::from).into();
        assert_eq!(detect(&h, &paths).unwrap().details.len(), 1);
        let h = Fake::new(UBUNTU_EXT4, &[], UBUNTU_STARTS);
        assert!(detect(&h, &["/".to_owned()]).is_err());
    }

    #[test]
    fn growpart_is_installed_through_apt_when_missing() {
        let mut h = Fake::new(
            UBUNTU_EXT4,
            &[("/", fm("/dev/sda1", "/", "ext4", 20_767_420_416))],
            UBUNTU_STARTS,
        );
        h.growpart = Cell::new(false);
        let paths = vec!["/".to_owned()];
        assert!(!detect(&h, &paths).unwrap().growpart_installed);
        run_plan(&h, &["/"], "/").unwrap();
        assert_eq!(
            h.calls(),
            vec![
                "apt-get -q update",
                "apt-get -q -y install cloud-guest-utils",
                "growpart /dev/sda 1",
                "resize2fs /dev/sda1",
            ]
        );
        h.apt = false;
        h.growpart.set(false);
        assert!(
            detect(&h, &paths).unwrap().details[0]
                .volume
                .refusal
                .is_some()
        );
    }

    #[test]
    fn steps_are_checked_against_what_is_mounted() {
        let h = Fake::new(
            WHOLE_DISK,
            &[
                ("/", fm("/dev/sda1", "/", "ext4", 20_767_420_416)),
                (
                    "/var/lib/docker",
                    fm("/dev/sdb", "/var/lib/docker", "ext4", 53_000_000_000),
                ),
            ],
            UBUNTU_STARTS,
        );
        let paths: Vec<String> = ["/", "/var/lib/docker"].map(String::from).into();
        // Not a watched mount: the caller cannot make priv touch anything else.
        let e = grow_with(&h, &paths, "/etc", DiskStep::ResizeFs).unwrap_err();
        assert_eq!(e.code, PrivErrorCode::Invalid);
        // A bare disk has no partition and no LVM.
        for s in [DiskStep::GrowPartition, DiskStep::ResizePv] {
            let e = grow_with(&h, &paths, "/var/lib/docker", s).unwrap_err();
            assert_eq!(e.code, PrivErrorCode::Invalid, "{s:?}");
        }
        // ext4 on a partition is not LVM either.
        let e = grow_with(&h, &paths, "/", DiskStep::ResizePv).unwrap_err();
        assert_eq!(e.code, PrivErrorCode::Invalid);
        assert!(h.calls().is_empty());
    }

    #[test]
    fn a_refused_volume_runs_nothing() {
        let h = Fake::new(
            UBUNTU_EXT4,
            &[("/", fm("/dev/sda1", "/", "zfs", 20 * GIB))],
            UBUNTU_STARTS,
        );
        let e = grow_with(&h, &["/".to_owned()], "/", DiskStep::ResizeFs).unwrap_err();
        assert_eq!(e.code, PrivErrorCode::Invalid);
        assert!(e.message.contains("ext4, xfs and btrfs"));
        assert!(h.calls().is_empty());
    }

    #[test]
    fn tool_outcomes_become_plain_results() {
        let h = Fake::new(
            LVM,
            &[(
                "/",
                fm(
                    "/dev/mapper/ubuntu--vg-ubuntu--lv",
                    "/",
                    "ext4",
                    10_447_081_472,
                ),
            )],
            LVM_STARTS,
        );
        let paths = vec!["/".to_owned()];
        // growpart says NOCHANGE (exit 1): nothing to do, not a failure.
        h.results.borrow_mut().insert(
            "growpart",
            (1, "NOCHANGE: partition 3 is size 1. it cannot be grown"),
        );
        let r = grow_with(&h, &paths, "/", DiskStep::GrowPartition).unwrap();
        assert!(matches!(r, PrivReply::DiskStep { log } if log[0].contains("already fills")));
        // Any other growpart failure keeps the tool's last words.
        h.results
            .borrow_mut()
            .insert("growpart", (2, "FAILED: sfdisk exploded"));
        let e = grow_with(&h, &paths, "/", DiskStep::GrowPartition).unwrap_err();
        assert!(e.message.contains("sfdisk exploded"), "{}", e.message);
        // lvextend with nothing free is fine; a real error is not.
        h.results.borrow_mut().insert(
            "lvextend",
            (
                5,
                "New size (2560 extents) matches existing size (2560 extents).",
            ),
        );
        assert!(grow_with(&h, &paths, "/", DiskStep::ResizeFs).is_ok());
        h.results
            .borrow_mut()
            .insert("lvextend", (5, "Insufficient free space"));
        let e = grow_with(&h, &paths, "/", DiskStep::ResizeFs).unwrap_err();
        assert!(e.message.contains("Insufficient free space"));
    }

    #[test]
    fn arguments_must_look_like_paths() {
        assert!(arg_ok("/dev/nvme0n1p1") && arg_ok("/var/lib/docker"));
        for bad in ["-rf", "", "dev/sda", "/dev/sda\n--force", "/a\0b"] {
            assert!(!arg_ok(bad), "{bad:?}");
        }
    }
}
