//! The version this build reports: `opsbox --version`, `/health`, `/updates/self`,
//! the privileged helper's `Ping` and the update slot names all use it.
//!
//! A published release reports its tag (`0.1.0-alpha.21`). A local build reports
//! `0.1.0-dev+<sha>` and is never offered updates unless the user picks a version.

pub const VERSION: &str = env!("OPSBOX_VERSION");

/// Whether `version` is a local development build (`0.1.0-dev` or `0.1.0-dev+<sha>`).
pub fn is_dev(version: &str) -> bool {
    let v = version.trim_start_matches('v');
    let no_meta = v.split('+').next().unwrap_or(v);
    no_meta
        .split_once('-')
        .is_some_and(|(_, pre)| pre == "dev" || pre.starts_with("dev."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_versions_are_recognised() {
        assert!(is_dev("0.1.0-dev"));
        assert!(is_dev("0.1.0-dev+abc123def"));
        assert!(is_dev("v0.1.0-dev+abc123def"));
        assert!(!is_dev("0.1.0"));
        assert!(!is_dev("0.1.0-alpha.21"));
        assert!(!is_dev("0.1.0-alpha.21+dev"));
        assert!(!is_dev("0.1.0-devices.1"));
    }

    #[test]
    fn this_build_has_a_version() {
        assert!(!VERSION.is_empty());
        assert!(VERSION.starts_with(|c: char| c.is_ascii_digit()));
    }
}
