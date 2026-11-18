//! All capacity defaults, in MiB. Swap is a safety margin, never database RAM.
pub const OS_MIN_MB: u64 = 192;
pub const OPSBOX_MB: u64 = 128;
pub const DOCKER_MB: u64 = 150;
pub const RESERVED_CPU_MILLIS: u64 = 350; // OS 250 + opsbox 100.
pub const DISK_MIN_MB: u64 = 2048;
pub const EXTENSION_MB: u64 = 64;
pub const BUILD_MB: u64 = 1536;
pub const POOLER_MB: u64 = 128;
pub const DEFAULT_MB: u64 = 128;
pub const RECOMMENDED_MB: u64 = 256;
pub const PRESSURE_SECONDS: u64 = 120;
pub const PROFILE_SECONDS: u64 = 600;
pub const BACKUP_DELAY_SECONDS: u64 = 1800;
pub const CONFIRMATION: &str = "I understand this server may run out of memory";

use crate::Feature;
/// (feature, minimum RAM/headroom, recommended RAM/headroom, free disk).
pub const FEATURES: &[(Feature, u64, u64, u64)] = &[
    (Feature::Install, 512, 1024, 2048),
    (Feature::Docker, 768, 1024, 0),
    (Feature::App, DEFAULT_MB, RECOMMENDED_MB, 0),
    (Feature::Build, BUILD_MB, 2048, 5120),
    (Feature::Database, 256, 512, 0),
    (Feature::Template, DEFAULT_MB, RECOMMENDED_MB, 0),
    (Feature::Backup, 256, 256, 0),
    (Feature::ImagePull, 0, 0, 0),
];
pub fn os_mb(ram: u64) -> u64 {
    OS_MIN_MB.max(ram / 10)
}
pub fn disk_mb(total: u64) -> u64 {
    DISK_MIN_MB.max(total / 10)
}
pub fn default_mb() -> u64 {
    DEFAULT_MB
}
pub fn recommended_mb() -> u64 {
    RECOMMENDED_MB
}
