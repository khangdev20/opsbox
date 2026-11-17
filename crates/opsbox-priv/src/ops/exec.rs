//! Running external tools: fixed argument lists (never a shell), a clean environment,
//! absolute paths and a time limit.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }
    /// stdout then stderr, for tools that print to either.
    pub fn combined(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

/// The first of `candidates` that exists.
pub fn find(candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .find(|p| Path::new(p).exists())
        .map(|p| (*p).to_owned())
}

/// Run `program args...`. `None` if it is missing, cannot start or runs past `limit`
/// (it is killed then). `stdin_text` is written to its standard input, if given.
pub fn run(program: &str, args: &[&str], limit: Duration) -> Option<Output> {
    run_with(program, args, limit, None, &[])
}

pub fn run_with(
    program: &str,
    args: &[&str],
    limit: Duration,
    stdin_text: Option<&str>,
    env: &[(&str, &str)],
) -> Option<Output> {
    if !Path::new(program).exists() {
        return None;
    }
    let mut cmd = Command::new(program);
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
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().ok()?;
    if let (Some(text), Some(mut stdin)) = (stdin_text, child.stdin.take()) {
        use std::io::Write;
        let _ = stdin.write_all(text.as_bytes());
    }
    // Read both pipes on threads so a chatty tool cannot fill a pipe and stall.
    let mut out = child.stdout.take()?;
    let mut err = child.stderr.take()?;
    let t_out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let t_err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait().ok()? {
            Some(st) => break st,
            None if started.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = (t_out.join(), t_err.join());
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    Some(Output {
        code: status.code(),
        stdout: t_out.join().unwrap_or_default(),
        stderr: t_err.join().unwrap_or_default(),
    })
}

/// The last `n` non-empty lines.
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
    fn captures_output_and_exit_status() {
        let o = run(
            "/bin/sh",
            &["-c", "echo out; echo err >&2; exit 3"],
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(
            (o.code, o.stdout.as_str(), o.stderr.as_str()),
            (Some(3), "out\n", "err\n")
        );
        assert!(!o.ok());
        assert!(
            run("/bin/sh", &["-c", "exit 0"], Duration::from_secs(5))
                .unwrap()
                .ok()
        );
    }

    #[test]
    fn environment_is_clean_and_stdin_is_delivered() {
        let o = run_with(
            "/bin/sh",
            &["-c", "echo \"[$HOME][$FOO]\"; cat"],
            Duration::from_secs(5),
            Some("piped"),
            &[("FOO", "bar")],
        )
        .unwrap();
        assert!(
            o.stdout.starts_with("[][bar]\n") && o.stdout.ends_with("piped"),
            "{:?}",
            o.stdout
        );
    }

    #[test]
    fn missing_and_hung_programs_are_none() {
        assert!(run("/nonexistent/tool", &[], Duration::from_secs(1)).is_none());
        let t = Instant::now();
        assert!(run("/bin/sleep", &["5"], Duration::from_millis(200)).is_none());
        assert!(t.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn large_output_does_not_stall_the_child() {
        let o = run(
            "/bin/sh",
            &["-c", "head -c 3000000 /dev/zero | tr '\\0' 'a'"],
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(o.stdout.len(), 3_000_000);
    }

    #[test]
    fn tail_keeps_the_last_non_empty_lines() {
        assert_eq!(tail("a\n\nb\nc\n", 2), ["b", "c"]);
        assert_eq!(tail("", 3), Vec::<String>::new());
    }
}
