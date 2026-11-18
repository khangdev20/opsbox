//! Types for growing the filesystem after the disk was enlarged (docs/prompts 16h).
//!
//! `priv` looks at the machine and answers with [`DiskState`]. Growing is a series of
//! [`DiskStep`]s; each one names only a mount point and a step, never a device: `priv`
//! derives the devices again from what it finds on the machine.

use serde::{Deserialize, Serialize};

/// One step of growing a filesystem, in the order they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskStep {
    /// Install `growpart` (package `cloud-guest-utils`) when it is missing.
    InstallGrowpart,
    /// `growpart <disk> <partition number>`: stretch the partition to the end of the disk.
    GrowPartition,
    /// `pvresize <partition>`: let LVM see the bigger partition.
    ResizePv,
    /// Grow the filesystem online: `resize2fs`, `xfs_growfs`, `btrfs filesystem resize max`,
    /// or `lvextend -r -l +100%FREE` for LVM.
    ResizeFs,
}

impl DiskStep {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InstallGrowpart => "install_growpart",
            Self::GrowPartition => "grow_partition",
            Self::ResizePv => "resize_pv",
            Self::ResizeFs => "resize_fs",
        }
    }

    /// What a person reads while it runs.
    pub fn label(self) -> &'static str {
        match self {
            Self::InstallGrowpart => "Installing the growpart tool",
            Self::GrowPartition => "Growing the partition",
            Self::ResizePv => "Telling LVM about the bigger partition",
            Self::ResizeFs => "Growing the filesystem",
        }
    }
}

/// How the filesystem sits on the disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskLayout {
    /// Filesystem on a partition (`sda1`, `nvme0n1p1`).
    Partition,
    /// Filesystem straight on the disk, no partition table.
    Disk,
    /// Filesystem on an LVM logical volume whose physical volume is a partition or a disk.
    Lvm,
    /// Anything else (RAID, encrypted, several disks); never touched.
    Other,
}

/// One filesystem opsbox cares about (`/`, the data directory, Docker's directory).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskVolume {
    pub mount: String,
    /// The device mounted there: `/dev/nvme0n1p1`, `/dev/mapper/ubuntu--vg-ubuntu--lv`.
    pub device: String,
    /// Raw filesystem type as the system names it: `ext4`, `xfs`, `btrfs`, ...
    pub fs: String,
    pub layout: DiskLayout,
    /// The partition (or LVM physical volume) under the filesystem; `None` for a bare disk.
    pub partition: Option<String>,
    /// The whole disk.
    pub disk: Option<String>,
    /// Size of the filesystem as the system reports it.
    pub fs_bytes: u64,
    pub partition_bytes: Option<u64>,
    pub disk_bytes: Option<u64>,
    /// Space the disk or partition has that the filesystem does not use yet.
    pub growable_bytes: u64,
    /// Grow would do something and is allowed.
    pub growable: bool,
    /// Steps Grow would run now, in order. Empty when `growable` is false.
    pub steps: Vec<DiskStep>,
    /// Why this one cannot be grown, in words for a person. `None` when it can, or when
    /// there simply is nothing to grow.
    pub refusal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskState {
    pub volumes: Vec<DiskVolume>,
    /// `growpart` is on this machine.
    pub growpart_installed: bool,
    /// opsbox can install `growpart` with the machine's package manager.
    pub growpart_installable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_use_stable_snake_case_names() {
        assert_eq!(
            serde_json::to_string(&DiskStep::GrowPartition).unwrap(),
            "\"grow_partition\""
        );
        for s in [
            DiskStep::InstallGrowpart,
            DiskStep::GrowPartition,
            DiskStep::ResizePv,
            DiskStep::ResizeFs,
        ] {
            assert_eq!(
                serde_json::to_string(&s).unwrap(),
                format!("\"{}\"", s.as_str())
            );
            assert!(!s.label().is_empty());
        }
    }
}
