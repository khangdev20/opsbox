//! Decides the version string every opsbox binary reports.
//!
//! The release workflow sets `OPSBOX_BUILD_VERSION` to the tag without its `v`
//! (for example `0.1.0-alpha.21`). Any other build is a development build and gets
//! `<crate version>-dev+<short git sha>`, so it never looks like a published release.

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=OPSBOX_BUILD_VERSION");
    // Cargo does not watch .git, so name the files that change when HEAD moves.
    let git_dir = Path::new("../../.git");
    println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
    if let Ok(head) = std::fs::read_to_string(git_dir.join("HEAD"))
        && let Some(r) = head.strip_prefix("ref: ")
    {
        println!(
            "cargo:rerun-if-changed={}",
            git_dir.join(r.trim()).display()
        );
    }

    let stamped = std::env::var("OPSBOX_BUILD_VERSION")
        .ok()
        .map(|v| v.trim().trim_start_matches('v').to_owned())
        .filter(|v| !v.is_empty());
    let version = stamped.unwrap_or_else(|| {
        let pkg = std::env::var("CARGO_PKG_VERSION").expect("cargo sets CARGO_PKG_VERSION");
        // `0.1.0-alpha.3` in Cargo.toml still becomes `0.1.0-dev`: only CI stamps releases.
        let base = pkg.split(['-', '+']).next().unwrap_or(&pkg);
        match short_sha() {
            Some(sha) => format!("{base}-dev+{sha}"),
            None => format!("{base}-dev"),
        }
    });
    println!("cargo:rustc-env=OPSBOX_VERSION={version}");
}

fn short_sha() -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--short=9", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (!sha.is_empty()).then_some(sha)
}
