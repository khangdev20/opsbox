//! `opsbox cleanup --safe`: what the weekly timer runs. It acts directly (it is root),
//! only on the scanners marked safe, and reports to the journal through stdout.

use nix::unistd::geteuid;
use opsbox_priv::cleanup;
use opsbox_proto::PrivReply;

/// Run a fresh cleanup scan and print items (and per-scanner timings when `--timings` is passed).
pub fn scan(timings: bool) -> bool {
    if !geteuid().is_root() {
        eprintln!(
            "opsbox cleanup scan needs root to inspect system caches (try: sudo opsbox cleanup scan --timings)."
        );
        return false;
    }
    let (scan, map) = cleanup::scan_with_timings();
    for item in &scan.items {
        if timings {
            let ms = item.duration_ms.unwrap_or(0);
            println!("{:<20} {:>10} bytes  ({} ms)", item.id, item.bytes, ms);
        } else {
            println!("{:<20} {:>10} bytes  {}", item.id, item.bytes, item.title);
        }
    }
    if timings {
        println!("\nscanner group timings:");
        for (group, ms) in &map {
            println!("  {group:<14} {ms} ms");
        }
    }
    true
}

/// Returns `true` if nothing went wrong.
pub fn run() -> bool {
    if !geteuid().is_root() {
        eprintln!(
            "opsbox cleanup frees system files, so it must run as root (try: sudo opsbox cleanup --safe)."
        );
        return false;
    }
    match cleanup::run_ids(&cleanup::safe_ids()) {
        Ok(PrivReply::CleanupFreed(freed)) => {
            let mut ok = true;
            let mut total = 0u64;
            for f in &freed {
                match &f.error {
                    None => println!("{}: freed {} bytes", f.id, f.bytes),
                    Some(e) => {
                        ok = false;
                        println!("{}: failed: {e}", f.id);
                    }
                }
                total += f.bytes;
            }
            println!("total freed: {total} bytes");
            ok
        }
        Ok(other) => {
            eprintln!("unexpected answer: {other:?}");
            false
        }
        Err(e) => {
            eprintln!("{}", e.message);
            false
        }
    }
}
