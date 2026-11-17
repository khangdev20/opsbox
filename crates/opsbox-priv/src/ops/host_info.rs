//! `HostInfo`: hostname, OS, kernel, uptime, CPU count and total memory.

use opsbox_proto::{HostInfo, PrivError, PrivErrorCode};
use sysinfo::{CpuRefreshKind, System};

pub(super) fn run() -> Result<HostInfo, PrivError> {
    let internal = |what: &str, e: sysinfo::Error| PrivError {
        code: PrivErrorCode::Internal,
        message: format!("could not read {what}: {e}"),
    };

    let mut sys = System::new().map_err(|e| internal("system information", e))?;
    sys.refresh_cpu_list(CpuRefreshKind::nothing());
    sys.refresh_memory();

    let os = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| parse_os_release(&text))
        .or_else(|| System::long_os_version().ok())
        .or_else(|| System::name().ok())
        .unwrap_or_else(|| "unknown".to_owned());

    Ok(HostInfo {
        hostname: System::host_name().map_err(|e| internal("the hostname", e))?,
        os,
        kernel: System::kernel_version().map_err(|e| internal("the kernel version", e))?,
        uptime_secs: System::uptime().map_err(|e| internal("the uptime", e))?,
        cpu_count: u32::try_from(sys.cpus().len()).unwrap_or(u32::MAX),
        mem_total_bytes: sys.total_memory(),
    })
}

/// Pick a display name out of `/etc/os-release`: `PRETTY_NAME`, else `NAME` plus
/// `VERSION`. Values may be quoted with single or double quotes.
fn parse_os_release(text: &str) -> Option<String> {
    let get = |key: &str| {
        text.lines().find_map(|line| {
            let (k, v) = line.trim().split_once('=')?;
            if k != key {
                return None;
            }
            let v = v.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                .unwrap_or(v);
            (!v.is_empty()).then(|| v.to_owned())
        })
    };
    get("PRETTY_NAME").or_else(|| match (get("NAME"), get("VERSION")) {
        (Some(n), Some(v)) => Some(format!("{n} {v}")),
        (Some(n), None) => Some(n),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_name_wins() {
        let text = "NAME=\"Ubuntu\"\nVERSION=\"24.04.1 LTS (Noble Numbat)\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nID=ubuntu\n";
        assert_eq!(parse_os_release(text).unwrap(), "Ubuntu 24.04.1 LTS");
    }

    #[test]
    fn falls_back_to_name_and_version() {
        assert_eq!(
            parse_os_release("NAME='Alpine Linux'\nVERSION=3.20\n").unwrap(),
            "Alpine Linux 3.20"
        );
        assert_eq!(parse_os_release("NAME=Arch\n").unwrap(), "Arch");
    }

    #[test]
    fn unquoted_and_empty() {
        assert_eq!(parse_os_release("PRETTY_NAME=Debian\n").unwrap(), "Debian");
        assert!(parse_os_release("PRETTY_NAME=\"\"\nFOO=bar\n").is_none());
        assert!(parse_os_release("").is_none());
    }

    #[test]
    fn collects_real_facts_on_this_machine() {
        let info = run().expect("host info");
        assert!(!info.hostname.is_empty());
        assert!(!info.kernel.is_empty());
        assert!(info.cpu_count >= 1);
        assert!(info.mem_total_bytes > 0);
    }
}
