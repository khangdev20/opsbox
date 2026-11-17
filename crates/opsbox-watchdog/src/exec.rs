//! Bounded child process runner with clean environment, absolute paths, and
//! process-group timeouts (no shell).

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MAX_OUTPUT: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct CmdOutput {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CmdOutput {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    pub fn combined(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

pub fn find(candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .find(|p| Path::new(p).exists())
        .map(|p| (*p).to_owned())
}

fn capture(mut pipe: impl Read) -> String {
    let mut kept = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(n) = pipe.read(&mut buf) {
        if n == 0 {
            break;
        }
        let keep = n.min(MAX_OUTPUT.saturating_sub(kept.len()));
        kept.extend_from_slice(&buf[..keep]);
        if keep < n {
            // Keep draining so the child never stalls on a full pipe.
            continue;
        }
    }
    String::from_utf8_lossy(&kept).into_owned()
}

pub fn run(program: &str, args: &[&str], limit: Duration) -> Option<CmdOutput> {
    run_with_stdin(program, args, limit, None)
}

pub fn run_with_stdin(
    program: &str,
    args: &[&str],
    limit: Duration,
    stdin_text: Option<&str>,
) -> Option<CmdOutput> {
    if !Path::new(program).exists() {
        return None;
    }
    let mut cmd = Command::new(program);
    cmd.process_group(0);
    cmd.args(args)
        .env_clear()
        .env(
            "PATH",
            "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        )
        .env("LC_ALL", "C")
        .stdin(if stdin_text.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().ok()?;
    if let (Some(text), Some(mut stdin)) = (stdin_text, child.stdin.take()) {
        let _ = stdin.write_all(text.as_bytes());
    }
    let out = child.stdout.take()?;
    let err = child.stderr.take()?;
    let t_out = std::thread::spawn(move || capture(out));
    let t_err = std::thread::spawn(move || capture(err));

    let started = Instant::now();
    let status = loop {
        match child.try_wait().ok()? {
            Some(st) => break st,
            None if started.elapsed() > limit => {
                if let Ok(pid) = i32::try_from(child.id()) {
                    let _ = nix::sys::signal::killpg(
                        nix::unistd::Pid::from_raw(pid),
                        nix::sys::signal::Signal::SIGKILL,
                    );
                }
                let _ = child.kill();
                let _ = child.wait();
                let _ = (t_out.join(), t_err.join());
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };

    Some(CmdOutput {
        code: status.code(),
        stdout: t_out.join().unwrap_or_default(),
        stderr: t_err.join().unwrap_or_default(),
    })
}

pub fn tail(text: &str, n: usize) -> Vec<String> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..]
        .iter()
        .map(|l| (*l).to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_and_times_out_cleanly() {
        let ok = run(
            "/bin/sh",
            &["-c", "echo hi; exit 0"],
            Duration::from_secs(5),
        )
        .unwrap();
        assert!(ok.ok());
        assert_eq!(ok.stdout.trim(), "hi");

        let timed = run("/bin/sleep", &["5"], Duration::from_millis(100));
        assert!(timed.is_none());
    }
}
