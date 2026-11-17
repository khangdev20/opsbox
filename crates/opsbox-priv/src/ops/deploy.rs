//! Privileged deploy pipeline execution: source fetch, build detection, BuildKit build
//! with `--secret` mounts, release startup, loopback HTTP/TCP health check, and cleanup.
//!
//! Only `opsbox-priv` executes `git`, `docker`, `nixpacks`, or touches `/var/lib/opsbox/apps`
//! directly; every input from `serve` is re-validated here before touching the host.

use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime};

use opsbox_deploy::{
    CheckoutBuildPlan, plan_checkout_build, scrub_secrets, static_site_dockerfile,
    validate_public_repo_url,
};
use opsbox_docker::{DockerError, ServiceContainerSpec};
use opsbox_proto::{
    DeployBuildFrame, DeployBuildRequest, DeployBuildResult, DeployEnvSpec, DeployGitSource,
    DeployPullRequest, DeployStartRequest, DeployStartResult, PrivError, PrivErrorCode, PrivReply,
    PrivRequest, PrivResponse, PrivResult, frame, valid_commit_sha, valid_docker_image_ref,
    valid_docker_ref, valid_env_key, valid_git_branch, valid_health_path, valid_rel_repo_path,
    valid_service_name,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpStream, UnixStream};
use tokio::sync::mpsc;

use super::Env;
use super::docker::{block_on_fut, map_docker_err};
use super::sealed;
use crate::io::write_frame;
use crate::secrets::Secrets;

const MAX_LOG_LINE_BYTES: usize = 4096;
const KEEP_RECENT_SOURCES: usize = 5;

fn err(code: PrivErrorCode, message: impl Into<String>) -> PrivError {
    PrivError {
        code,
        message: message.into(),
    }
}

fn find_binary(custom: Option<&Path>, names: &[&str], candidates: &[&str]) -> Option<PathBuf> {
    if let Some(p) = custom.filter(|p| p.exists()) {
        return Some(p.to_path_buf());
    }
    for c in candidates {
        let p = Path::new(c);
        if p.exists() {
            return Some(p.to_path_buf());
        }
    }
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for name in names {
                let p = dir.join(name);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn validate_repo_url(
    kind: DeployGitSource,
    url: &str,
    allow_test_git: bool,
) -> Result<(), PrivError> {
    if url.is_empty()
        || url.len() > 512
        || url
            .bytes()
            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
    {
        return Err(err(
            PrivErrorCode::Invalid,
            "Repository URL contains invalid characters.",
        ));
    }
    if allow_test_git && url.starts_with("file://") {
        let path = url.trim_start_matches("file://");
        if path.is_empty() || !Path::new(path).exists() {
            return Err(err(
                PrivErrorCode::Invalid,
                "Test git repository path does not exist.",
            ));
        }
        return Ok(());
    }
    match kind {
        DeployGitSource::Github => {
            if !url.starts_with("https://github.com/") || url.contains("..") || url.contains('@') {
                return Err(err(
                    PrivErrorCode::Invalid,
                    "GitHub clone URL must use https://github.com/.",
                ));
            }
            Ok(())
        }
        DeployGitSource::GitUrl => {
            validate_public_repo_url(url)
                .map_err(|e| err(PrivErrorCode::Invalid, e.to_string()))?;
            Ok(())
        }
        DeployGitSource::DeployKey => {
            let ok = if let Some(rest) = url.strip_prefix("git@") {
                let Some((host, path)) = rest.split_once(':') else {
                    return Err(err(PrivErrorCode::Invalid, "Invalid SSH repository URL."));
                };
                !host.is_empty()
                    && !host.starts_with('-')
                    && !path.is_empty()
                    && !path.starts_with('-')
                    && !path.contains("..")
            } else if url.starts_with("ssh://") {
                !url.contains("..") && url.len() > 6
            } else {
                false
            };
            if !ok {
                return Err(err(
                    PrivErrorCode::Invalid,
                    "Deploy-key URL must be git@host:owner/repo.git or ssh://.",
                ));
            }
            Ok(())
        }
    }
}

struct OpenedVar {
    key: String,
    value: String,
    is_secret: bool,
}

fn open_deploy_vars(
    secrets: &Secrets,
    specs: &[DeployEnvSpec],
) -> Result<(Vec<OpenedVar>, Vec<String>), PrivError> {
    let mut opened = Vec::with_capacity(specs.len());
    let mut scrub = Vec::new();
    for v in specs {
        if !valid_env_key(&v.key) {
            return Err(err(
                PrivErrorCode::Invalid,
                format!("Invalid variable name '{}'.", v.key),
            ));
        }
        let sec = sealed::open_secret(secrets, &v.seal_context, &v.value_enc)?;
        let val = sec.expose().to_owned();
        if v.is_secret && !val.is_empty() {
            scrub.push(val.clone());
        }
        opened.push(OpenedVar {
            key: v.key.clone(),
            value: val,
            is_secret: v.is_secret,
        });
    }
    Ok((opened, scrub))
}

fn validate_build_req(req: &DeployBuildRequest, allow_test_git: bool) -> Result<(), PrivError> {
    if !valid_service_name(&req.service) {
        return Err(err(PrivErrorCode::Invalid, "Invalid service name."));
    }
    if req.deploy_id.is_empty() || !valid_docker_ref(&req.deploy_id) {
        return Err(err(PrivErrorCode::Invalid, "Invalid deploy ID."));
    }
    if !valid_git_branch(&req.branch) {
        return Err(err(PrivErrorCode::Invalid, "Invalid git branch name."));
    }
    if req
        .commit_sha
        .as_deref()
        .is_some_and(|s| !valid_commit_sha(s))
    {
        return Err(err(PrivErrorCode::Invalid, "Invalid commit SHA."));
    }
    if !valid_docker_image_ref(&req.image_tag) || !req.image_tag.starts_with("opsbox/") {
        return Err(err(PrivErrorCode::Invalid, "Invalid target image tag."));
    }
    if req
        .cache_from
        .as_deref()
        .is_some_and(|c| !valid_docker_image_ref(c))
    {
        return Err(err(PrivErrorCode::Invalid, "Invalid cache image tag."));
    }
    if req
        .dockerfile_path
        .as_deref()
        .is_some_and(|p| !valid_rel_repo_path(p))
    {
        return Err(err(PrivErrorCode::Invalid, "Invalid Dockerfile path."));
    }
    validate_repo_url(req.source_kind, &req.repo_url, allow_test_git)?;
    for v in &req.vars {
        if !valid_env_key(&v.key) {
            return Err(err(
                PrivErrorCode::Invalid,
                format!("Invalid variable name '{}'.", v.key),
            ));
        }
    }
    Ok(())
}

/// Ensure `/var/lib/opsbox/apps/<service>` and its `src/` subdirectory exist cleanly.
pub fn ensure_service_dir(apps_dir: &Path, service: &str) -> Result<PathBuf, PrivError> {
    if !valid_service_name(service) {
        return Err(err(PrivErrorCode::Invalid, "Invalid service name."));
    }
    std::fs::create_dir_all(apps_dir).map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Cannot prepare apps directory: {e}"),
        )
    })?;
    let svc_dir = apps_dir.join(service);
    std::fs::create_dir_all(svc_dir.join("src")).map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Cannot create service directory: {e}"),
        )
    })?;
    let _ = std::fs::set_permissions(&svc_dir, std::fs::Permissions::from_mode(0o750));
    Ok(svc_dir)
}

async fn write_build_frame(stream: &mut UnixStream, frame_msg: &DeployBuildFrame) -> bool {
    let Ok(encoded) = frame::encode(frame_msg) else {
        return false;
    };
    stream.write_all(&encoded).await.is_ok() && stream.flush().await.is_ok()
}

fn sanitize_line(raw: &str, scrub: &[&str]) -> String {
    let clean = scrub_secrets(raw, scrub);
    if clean.len() <= MAX_LOG_LINE_BYTES {
        return clean;
    }
    let mut end = MAX_LOG_LINE_BYTES;
    while !clean.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    format!("{}...", &clean[..end])
}

async fn send_log(stream: &mut UnixStream, line: &str, scrub: &[&str]) -> bool {
    write_build_frame(
        stream,
        &DeployBuildFrame::Log {
            line: sanitize_line(line, scrub),
        },
    )
    .await
}

fn kill_child_group(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id()
        && let Ok(raw) = i32::try_from(pid)
    {
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(raw),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
    let _ = child.start_kill();
}

/// Run an external build/git command with clean environment, streaming output lines,
/// timeout enforcement, process-group kill on timeout/abort, and secret scrubbing.
async fn run_streaming_command(
    mut cmd: tokio::process::Command,
    stream: &mut UnixStream,
    timeout: Duration,
    scrub: &[&str],
) -> Result<(bool, String), PrivError> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    unsafe_free_process_group(&mut cmd);

    let mut child = cmd.spawn().map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Failed to start command: {e}"),
        )
    })?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let (tx, mut rx) = mpsc::channel::<String>(256);
    if let Some(out) = stdout {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if tx.send(line).await.is_err() {
                    break;
                }
            }
        });
    }
    if let Some(err_io) = stderr {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(err_io).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if tx.send(line).await.is_err() {
                    break;
                }
            }
        });
    }
    drop(tx);

    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);

    let mut tail_buf: Vec<String> = Vec::new();
    let mut sink = [0u8; 64];
    let mut channel_open = true;
    let mut timed_out = false;
    let mut peer_closed = false;

    let exit_status = loop {
        tokio::select! {
            _ = &mut deadline => {
                timed_out = true;
                kill_child_group(&mut child);
                break child.wait().await.ok();
            }
            read_res = stream.read(&mut sink) => {
                match read_res {
                    Ok(0) | Err(_) => {
                        peer_closed = true;
                        kill_child_group(&mut child);
                        break child.wait().await.ok();
                    }
                    Ok(_) => {}
                }
            }
            maybe_line = rx.recv(), if channel_open => {
                match maybe_line {
                    Some(line) => {
                        let clean = sanitize_line(&line, scrub);
                        if !clean.is_empty() {
                            if tail_buf.len() >= 15 {
                                tail_buf.remove(0);
                            }
                            tail_buf.push(clean.clone());
                            if !write_build_frame(stream, &DeployBuildFrame::Log { line: clean }).await {
                                peer_closed = true;
                                kill_child_group(&mut child);
                                break child.wait().await.ok();
                            }
                        }
                    }
                    None => {
                        channel_open = false;
                    }
                }
            }
            status_res = child.wait(), if !channel_open => {
                break status_res.ok();
            }
        }
    };

    // Drain any remaining buffered lines on normal exit.
    if !timed_out && !peer_closed {
        while let Some(line) = rx.recv().await {
            let clean = sanitize_line(&line, scrub);
            if !clean.is_empty() {
                if tail_buf.len() >= 15 {
                    tail_buf.remove(0);
                }
                tail_buf.push(clean.clone());
                let _ = write_build_frame(stream, &DeployBuildFrame::Log { line: clean }).await;
            }
        }
    }

    if peer_closed {
        return Err(err(
            PrivErrorCode::Internal,
            "Build cancelled because the caller disconnected.",
        ));
    }
    if timed_out {
        return Err(err(
            PrivErrorCode::Internal,
            format!("Build timed out after {} seconds.", timeout.as_secs()),
        ));
    }

    let ok = exit_status.is_some_and(|s| s.success());
    Ok((ok, tail_buf.join(" | ")))
}

fn unsafe_free_process_group(cmd: &mut tokio::process::Command) {
    // `std::os::unix::process::CommandExt::process_group` is safe Rust (no unsafe block).
    cmd.as_std_mut().process_group(0);
}

fn safe_env_base(cmd: &mut tokio::process::Command, extra_env: &[(&str, &str)]) {
    cmd.env_clear();
    let path = std::env::var_os("PATH")
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into());
    cmd.env("PATH", path);
    cmd.env("LC_ALL", "C");
    cmd.stdin(Stdio::null());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
}

fn prune_old_source_dirs(src_root: &Path, keep: usize, current_sha: &str) {
    let Ok(entries) = std::fs::read_dir(src_root) else {
        return;
    };
    let mut dirs: Vec<(SystemTime, PathBuf, String)> = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("_work-") {
            let _ = std::fs::remove_dir_all(&p);
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        dirs.push((modified, p, name));
    }
    dirs.sort_by_key(|a| std::cmp::Reverse(a.0));
    let mut kept = 0usize;
    for (_, p, name) in dirs {
        if name == current_sha || kept < keep {
            kept += 1;
        } else {
            let _ = std::fs::remove_dir_all(p);
        }
    }
}

async fn probe_docker_build_flags(
    docker_bin: &Path,
    docker_socket: &Path,
) -> (Vec<String>, bool, bool, bool) {
    let custom_sock = docker_socket != Path::new(opsbox_docker::DEFAULT_SOCKET_PATH);
    let docker_host_val = format!("unix://{}", docker_socket.display());

    let mut bx_cmd = tokio::process::Command::new(docker_bin);
    bx_cmd.args(["buildx", "version"]);
    safe_env_base(&mut bx_cmd, &[]);
    if custom_sock {
        bx_cmd.env("DOCKER_HOST", &docker_host_val);
    }
    let has_buildx = tokio::time::timeout(Duration::from_secs(5), bx_cmd.output())
        .await
        .ok()
        .and_then(Result::ok)
        .is_some_and(|o| o.status.success());

    let sub_args: Vec<String> = if has_buildx {
        vec!["buildx".into(), "build".into(), "--load".into()]
    } else {
        vec!["build".into()]
    };

    let mut help_cmd = tokio::process::Command::new(docker_bin);
    if has_buildx {
        help_cmd.args(["buildx", "build", "--help"]);
    } else {
        help_cmd.args(["build", "--help"]);
    }
    safe_env_base(&mut help_cmd, &[]);
    let help_text = tokio::time::timeout(Duration::from_secs(5), help_cmd.output())
        .await
        .ok()
        .and_then(Result::ok)
        .map(|o| {
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            )
        })
        .unwrap_or_default();

    let has_progress = help_text.contains("--progress");
    let has_memory = help_text.contains("--memory");
    (sub_args, !has_buildx, has_progress, has_memory)
}

/// Handle a streaming `PrivOp::DeployBuild` connection over the `priv` Unix socket.
pub(crate) async fn handle_deploy_build_stream(
    env: &Env,
    stream: &mut UnixStream,
    request: &PrivRequest,
    req: &DeployBuildRequest,
) {
    if let Err(e) = validate_build_req(req, env.allow_test_git) {
        let _ = write_frame(
            stream,
            &PrivResponse {
                id: request.id.clone(),
                result: PrivResult::Failure { error: e },
            },
        )
        .await;
        return;
    }

    let (opened_vars, mut scrub_owned) = match open_deploy_vars(&env.secrets, &req.vars) {
        Ok(v) => v,
        Err(e) => {
            let _ = write_frame(
                stream,
                &PrivResponse {
                    id: request.id.clone(),
                    result: PrivResult::Failure { error: e },
                },
            )
            .await;
            return;
        }
    };

    let opened_deploy_key = if let Some(enc) = &req.deploy_key_enc {
        let Some(ctx) = req.deploy_key_context.as_deref() else {
            let _ = write_frame(
                stream,
                &PrivResponse::failure(
                    request.id.clone(),
                    PrivErrorCode::Invalid,
                    "Missing deploy key sealing context.",
                ),
            )
            .await;
            return;
        };
        match sealed::open_secret(&env.secrets, ctx, enc) {
            Ok(s) => {
                let txt = s.expose().to_owned();
                scrub_owned.push(txt.clone());
                Some(txt)
            }
            Err(e) => {
                let _ = write_frame(
                    stream,
                    &PrivResponse {
                        id: request.id.clone(),
                        result: PrivResult::Failure { error: e },
                    },
                )
                .await;
                return;
            }
        }
    } else {
        None
    };

    if let Some(tok) = &req.github_token {
        scrub_owned.push(tok.expose().to_owned());
    }
    let scrub_refs: Vec<&str> = scrub_owned.iter().map(String::as_str).collect();

    let svc_dir = match ensure_service_dir(&env.apps_dir, &req.service) {
        Ok(d) => d,
        Err(e) => {
            let _ = write_frame(
                stream,
                &PrivResponse {
                    id: request.id.clone(),
                    result: PrivResult::Failure { error: e },
                },
            )
            .await;
            return;
        }
    };

    // Handshake: switch connection to DeployBuildFrame stream.
    let started = PrivResponse::success(
        request.id.clone(),
        PrivReply::DeployBuildStreamStarted {
            deploy_id: req.deploy_id.clone(),
        },
    );
    if write_frame(stream, &started).await.is_err() {
        return;
    }

    match run_deploy_build_pipeline(
        env,
        stream,
        req,
        &svc_dir,
        &opened_vars,
        opened_deploy_key.as_deref(),
        &scrub_refs,
    )
    .await
    {
        Ok(res) => {
            let _ = write_build_frame(stream, &DeployBuildFrame::Done(res)).await;
        }
        Err(mut e) => {
            e.message = scrub_secrets(&e.message, &scrub_refs);
            let _ = write_build_frame(stream, &DeployBuildFrame::Error(e)).await;
        }
    }
}

async fn run_deploy_build_pipeline(
    env: &Env,
    stream: &mut UnixStream,
    req: &DeployBuildRequest,
    svc_dir: &Path,
    opened_vars: &[OpenedVar],
    deploy_key_pem: Option<&str>,
    scrub: &[&str],
) -> Result<DeployBuildResult, PrivError> {
    let ephem = tempfile::Builder::new()
        .prefix("opsbox-build-")
        .tempdir()
        .map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Cannot create build tempdir: {e}"),
            )
        })?;
    let _ = std::fs::set_permissions(ephem.path(), std::fs::Permissions::from_mode(0o700));

    let git_bin = find_binary(
        None,
        &["git"],
        &[
            "/usr/bin/git",
            "/usr/local/bin/git",
            "/opt/homebrew/bin/git",
        ],
    )
    .ok_or_else(|| {
        err(
            PrivErrorCode::Unsupported,
            "git is not installed on this server.",
        )
    })?;

    let src_root = svc_dir.join("src");
    let work_dir = src_root.join(format!("_work-{}", req.deploy_id));
    if work_dir.exists() {
        let _ = std::fs::remove_dir_all(&work_dir);
    }

    if !send_log(
        stream,
        &format!("Cloning {} ({})...", req.repo_url, req.branch),
        scrub,
    )
    .await
    {
        return Err(err(PrivErrorCode::Internal, "Caller disconnected."));
    }

    let mut git_extra_env: Vec<(String, String)> = vec![
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ("HOME".into(), ephem.path().display().to_string()),
    ];

    if req.source_kind == DeployGitSource::Github
        && let Some(tok) = &req.github_token
    {
        let tok_file = ephem.path().join("gh_token");
        std::fs::write(&tok_file, tok.expose().as_bytes()).map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Credential write failed: {e}"),
            )
        })?;
        let _ = std::fs::set_permissions(&tok_file, std::fs::Permissions::from_mode(0o600));
        let askpass = ephem.path().join("askpass.sh");
        let script = format!(
            "#!/bin/sh\ncase \"$1\" in\n  *Username*|*username*) printf '%s\\n' 'x-access-token' ;;\n  *) exec cat '{}' ;;\nesac\n",
            tok_file.display()
        );
        std::fs::write(&askpass, script.as_bytes()).map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Askpass write failed: {e}"),
            )
        })?;
        let _ = std::fs::set_permissions(&askpass, std::fs::Permissions::from_mode(0o700));
        git_extra_env.push(("GIT_ASKPASS".into(), askpass.display().to_string()));
    } else if req.source_kind == DeployGitSource::DeployKey {
        let pem = deploy_key_pem.ok_or_else(|| {
            err(
                PrivErrorCode::Invalid,
                "Deploy key credentials are missing.",
            )
        })?;
        let key_file = ephem.path().join("id_deploy");
        let known_hosts = ephem.path().join("known_hosts");
        std::fs::write(&key_file, pem.as_bytes()).map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("SSH key write failed: {e}"),
            )
        })?;
        let _ = std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600));
        let ssh_cmd = format!(
            "ssh -i '{}' -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile='{}'",
            key_file.display(),
            known_hosts.display()
        );
        git_extra_env.push(("GIT_SSH_COMMAND".into(), ssh_cmd));
    }

    let mut clone_cmd = tokio::process::Command::new(&git_bin);
    clone_cmd.args([
        "clone",
        "--depth",
        "1",
        "--branch",
        &req.branch,
        "--single-branch",
        &req.repo_url,
    ]);
    clone_cmd.arg(&work_dir);
    let env_pairs: Vec<(&str, &str)> = git_extra_env
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    safe_env_base(&mut clone_cmd, &env_pairs);

    let clone_res = run_streaming_command(clone_cmd, stream, Duration::from_secs(120), scrub).await;
    match clone_res {
        Ok((true, _)) => {}
        Ok((false, tail)) => {
            let _ = std::fs::remove_dir_all(&work_dir);
            return Err(err(
                PrivErrorCode::Internal,
                format!("Git clone failed: {tail}"),
            ));
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&work_dir);
            return Err(e);
        }
    }

    // Check out exact commit if pinned and different from HEAD.
    if let Some(target_sha) = &req.commit_sha {
        let mut head_cmd = tokio::process::Command::new(&git_bin);
        head_cmd
            .arg("-C")
            .arg(&work_dir)
            .args(["rev-parse", "HEAD"]);
        safe_env_base(&mut head_cmd, &env_pairs);
        let head_out = head_cmd.output().await.ok();
        let current_head = head_out
            .as_ref()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default();
        if !current_head.starts_with(target_sha) {
            let mut fetch_cmd = tokio::process::Command::new(&git_bin);
            fetch_cmd
                .arg("-C")
                .arg(&work_dir)
                .args(["fetch", "--depth", "1", "origin", target_sha]);
            safe_env_base(&mut fetch_cmd, &env_pairs);
            let _ = run_streaming_command(fetch_cmd, stream, Duration::from_secs(60), scrub).await;

            let mut co_cmd = tokio::process::Command::new(&git_bin);
            co_cmd
                .arg("-C")
                .arg(&work_dir)
                .args(["checkout", "--detach", target_sha]);
            safe_env_base(&mut co_cmd, &env_pairs);
            match run_streaming_command(co_cmd, stream, Duration::from_secs(30), scrub).await {
                Ok((true, _)) => {}
                Ok((false, tail)) => {
                    let _ = std::fs::remove_dir_all(&work_dir);
                    return Err(err(
                        PrivErrorCode::Internal,
                        format!("Cannot check out commit {target_sha}: {tail}"),
                    ));
                }
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&work_dir);
                    return Err(e);
                }
            }
        }
    }

    // Resolve HEAD SHA and commit subject.
    let mut rev_cmd = tokio::process::Command::new(&git_bin);
    rev_cmd.arg("-C").arg(&work_dir).args(["rev-parse", "HEAD"]);
    safe_env_base(&mut rev_cmd, &env_pairs);
    let rev_out = rev_cmd.output().await.map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("git rev-parse failed: {e}"),
        )
    })?;
    if !rev_out.status.success() {
        let _ = std::fs::remove_dir_all(&work_dir);
        return Err(err(
            PrivErrorCode::Internal,
            "Could not inspect git commit.",
        ));
    }
    let resolved_sha = String::from_utf8_lossy(&rev_out.stdout).trim().to_owned();
    if !valid_commit_sha(&resolved_sha) {
        let _ = std::fs::remove_dir_all(&work_dir);
        return Err(err(PrivErrorCode::Internal, "Invalid git commit SHA."));
    }

    let mut msg_cmd = tokio::process::Command::new(&git_bin);
    msg_cmd
        .arg("-C")
        .arg(&work_dir)
        .args(["log", "-1", "--pretty=%s"]);
    safe_env_base(&mut msg_cmd, &env_pairs);
    let msg_out = msg_cmd.output().await.ok();
    let commit_message = msg_out
        .as_ref()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Deploy {}", &resolved_sha[..resolved_sha.len().min(7)]));

    // Move work dir to canonical /var/lib/opsbox/apps/<svc>/src/<commit>.
    let commit_dir = src_root.join(&resolved_sha);
    if commit_dir.exists() {
        let _ = std::fs::remove_dir_all(&commit_dir);
    }
    std::fs::rename(&work_dir, &commit_dir).map_err(|e| {
        let _ = std::fs::remove_dir_all(&work_dir);
        err(
            PrivErrorCode::Internal,
            format!("Cannot store source checkout: {e}"),
        )
    })?;
    prune_old_source_dirs(&src_root, KEEP_RECENT_SOURCES, &resolved_sha);

    let plan = plan_checkout_build(
        &commit_dir,
        &req.build_method,
        req.dockerfile_path.as_deref(),
    )
    .map_err(|e| err(PrivErrorCode::Invalid, e.to_string()))?;

    let detected_label = plan.label().to_owned();
    if !send_log(
        stream,
        &format!(
            "Checked out {} · build method: {detected_label}",
            &resolved_sha[..resolved_sha.len().min(7)]
        ),
        scrub,
    )
    .await
    {
        return Err(err(PrivErrorCode::Internal, "Caller disconnected."));
    }

    let build_timeout = Duration::from_secs(req.timeout_secs.clamp(10, 3600));

    // Nixpacks path: optional CLI on host.
    if let CheckoutBuildPlan::Nixpacks { label } = &plan {
        let nixpacks = find_binary(
            None,
            &["nixpacks"],
            &[
                "/usr/local/bin/nixpacks",
                "/usr/bin/nixpacks",
                "/opt/homebrew/bin/nixpacks",
            ],
        )
        .ok_or_else(|| {
            err(
                PrivErrorCode::Unsupported,
                format!(
                    "Detected '{label}', which requires Nixpacks, but `nixpacks` is not installed on this server. Add a Dockerfile or deploy a static site instead."
                ),
            )
        })?;
        let mut np_cmd = tokio::process::Command::new(nixpacks);
        np_cmd
            .arg("build")
            .arg(&commit_dir)
            .args(["--name", &req.image_tag]);
        for v in opened_vars {
            if !v.is_secret {
                np_cmd.args(["--env", &format!("{}={}", v.key, v.value)]);
            }
        }
        let custom_sock = env.docker.socket_path() != Path::new(opsbox_docker::DEFAULT_SOCKET_PATH);
        let dh = format!("unix://{}", env.docker.socket_path().display());
        let mut extra = vec![("DOCKER_BUILDKIT", "1")];
        if custom_sock {
            extra.push(("DOCKER_HOST", &dh));
        }
        safe_env_base(&mut np_cmd, &extra);
        match run_streaming_command(np_cmd, stream, build_timeout, scrub).await? {
            (true, _) => {
                return Ok(DeployBuildResult {
                    commit_sha: resolved_sha,
                    commit_message,
                    detected_build_method: detected_label,
                    image_tag: req.image_tag.clone(),
                    src_dir: commit_dir.display().to_string(),
                });
            }
            (false, tail) => {
                return Err(err(
                    PrivErrorCode::Internal,
                    format!("Nixpacks build failed: {tail}"),
                ));
            }
        }
    }

    // Docker BuildKit build (Dockerfile or generated static-site Dockerfile).
    let docker_bin = find_binary(
        env.docker_bin.as_deref(),
        &["docker"],
        &["/usr/bin/docker", "/usr/local/bin/docker"],
    )
    .ok_or_else(|| {
        err(
            PrivErrorCode::Unsupported,
            "Docker CLI (`docker`) is not installed on this server.",
        )
    })?;

    let dockerfile_abs = match &plan {
        CheckoutBuildPlan::Dockerfile { rel_path, .. } => commit_dir.join(rel_path),
        CheckoutBuildPlan::StaticHtml { .. } | CheckoutBuildPlan::StaticNodeBuild { .. } => {
            let contents = static_site_dockerfile(&plan, req.port).ok_or_else(|| {
                err(
                    PrivErrorCode::Internal,
                    "Cannot generate static site Dockerfile.",
                )
            })?;
            let path = ephem.path().join("Dockerfile.opsbox");
            std::fs::write(&path, contents.as_bytes()).map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Cannot write temporary Dockerfile: {e}"),
                )
            })?;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            path
        }
        CheckoutBuildPlan::Nixpacks { .. } => unreachable!(),
    };

    let (base_args, needs_env_buildkit, has_progress, has_memory) =
        probe_docker_build_flags(&docker_bin, env.docker.socket_path()).await;

    let mut d_cmd = tokio::process::Command::new(&docker_bin);
    d_cmd.args(&base_args);
    if has_progress {
        d_cmd.arg("--progress=plain");
    }
    if has_memory && let Some(mb) = req.memory_limit_mb.filter(|m| *m >= 64) {
        d_cmd.arg(format!("--memory={mb}m"));
    }
    if let Some(prev) = &req.cache_from {
        d_cmd.args(["--cache-from", prev]);
        d_cmd.args(["--build-arg", "BUILDKIT_INLINE_CACHE=1"]);
    }

    let secrets_dir = ephem.path().join("secrets");
    std::fs::create_dir_all(&secrets_dir).map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Cannot prepare BuildKit secret directory: {e}"),
        )
    })?;
    let _ = std::fs::set_permissions(&secrets_dir, std::fs::Permissions::from_mode(0o700));

    for v in opened_vars {
        if v.is_secret {
            let sec_file = secrets_dir.join(&v.key);
            std::fs::write(&sec_file, v.value.as_bytes()).map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Cannot write BuildKit secret file: {e}"),
                )
            })?;
            let _ = std::fs::set_permissions(&sec_file, std::fs::Permissions::from_mode(0o600));
            d_cmd.arg("--secret");
            d_cmd.arg(format!("id={},src={}", v.key, sec_file.display()));
        } else {
            d_cmd.arg("--build-arg");
            d_cmd.arg(format!("{}={}", v.key, v.value));
        }
    }

    d_cmd.arg("-f").arg(&dockerfile_abs);
    d_cmd.args(["-t", &req.image_tag]);
    d_cmd.args(["--label", &format!("opsbox.service={}", req.service)]);
    d_cmd.args(["--label", &format!("opsbox.deploy={}", req.deploy_id)]);
    d_cmd.arg(&commit_dir);

    let custom_sock = env.docker.socket_path() != Path::new(opsbox_docker::DEFAULT_SOCKET_PATH);
    let dh = format!("unix://{}", env.docker.socket_path().display());
    let mut env_vec: Vec<(&str, &str)> = Vec::new();
    if needs_env_buildkit {
        env_vec.push(("DOCKER_BUILDKIT", "1"));
    }
    if custom_sock {
        env_vec.push(("DOCKER_HOST", &dh));
    }
    safe_env_base(&mut d_cmd, &env_vec);

    if !send_log(
        stream,
        &format!("Building image {} with BuildKit...", req.image_tag),
        scrub,
    )
    .await
    {
        return Err(err(PrivErrorCode::Internal, "Caller disconnected."));
    }

    match run_streaming_command(d_cmd, stream, build_timeout, scrub).await? {
        (true, _) => Ok(DeployBuildResult {
            commit_sha: resolved_sha,
            commit_message,
            detected_build_method: detected_label,
            image_tag: req.image_tag.clone(),
            src_dir: commit_dir.display().to_string(),
        }),
        (false, tail) => Err(err(
            PrivErrorCode::Internal,
            format!("Docker build failed: {tail}"),
        )),
    }
}

/// Pull and tag a prebuilt Docker image under `opsbox/<service>:<deploy_id>`.
pub fn pull_image_with(env: &Env, req: &DeployPullRequest) -> Result<(), PrivError> {
    if !valid_docker_image_ref(&req.source_image)
        || !valid_docker_image_ref(&req.target_tag)
        || !req.target_tag.starts_with("opsbox/")
    {
        return Err(err(
            PrivErrorCode::Invalid,
            "Invalid Docker image reference.",
        ));
    }
    block_on_fut(
        env.docker
            .pull_and_tag_image(&req.source_image, &req.target_tag),
    )?
    .map_err(|e| map_docker_err(e, env.docker.socket_path()))
}

fn short_deploy_suffix(deploy_id: &str) -> String {
    deploy_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(12)
        .collect::<String>()
        .to_ascii_lowercase()
}

async fn http_health_probe(port: u16, path: &str) -> Result<u16, String> {
    let mut stream = tokio::time::timeout(
        Duration::from_secs(2),
        TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .map_err(|_| "TCP connect timed out".to_owned())?
    .map_err(|e| format!("TCP connect failed: {e}"))?;

    if path == "tcp" {
        return Ok(200);
    }

    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    tokio::time::timeout(Duration::from_secs(2), async {
        stream
            .write_all(req.as_bytes())
            .await
            .map_err(|e| format!("HTTP probe write failed: {e}"))?;
        let mut buf = [0u8; 512];
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|e| format!("HTTP probe read failed: {e}"))?;
        let text = String::from_utf8_lossy(&buf[..n]);
        let first_line = text.lines().next().unwrap_or("");
        let code = first_line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse::<u16>().ok())
            .ok_or_else(|| format!("Unexpected HTTP probe reply: {first_line}"))?;
        Ok(code)
    })
    .await
    .map_err(|_| "HTTP health probe timed out".to_owned())?
}

/// Start a release container and run HTTP (`2xx`/`3xx`) or TCP / worker health checks.
/// On failure, captures redacted container logs, removes the container, and returns an error.
pub fn start_and_check_with(
    env: &Env,
    req: &DeployStartRequest,
) -> Result<DeployStartResult, PrivError> {
    if !valid_service_name(&req.project)
        || !valid_service_name(&req.service)
        || !valid_docker_ref(&req.deploy_id)
        || !valid_docker_image_ref(&req.image_ref)
        || !valid_health_path(&req.health_path)
    {
        return Err(err(
            PrivErrorCode::Invalid,
            "Invalid deploy start parameters.",
        ));
    }
    let (opened_vars, scrub_owned) = open_deploy_vars(&env.secrets, &req.vars)?;
    let scrub_refs: Vec<&str> = scrub_owned.iter().map(String::as_str).collect();
    let env_strings: Vec<String> = opened_vars
        .into_iter()
        .map(|v| format!("{}={}", v.key, v.value))
        .collect();

    let suffix = short_deploy_suffix(&req.deploy_id);
    let container_name = format!("opsbox-{}-{suffix}", req.service);
    let network_name = format!("opsbox-{}", req.project);

    let spec = ServiceContainerSpec {
        project: &req.project,
        service: &req.service,
        deploy_id: &req.deploy_id,
        container_name: &container_name,
        network_name: &network_name,
        image_ref: &req.image_ref,
        env: &env_strings,
        port: req.port,
        memory_limit_mb: req.memory_limit_mb,
        cpu_limit_millis: req.cpu_limit_millis,
    };

    let socket_path = env.docker.socket_path();
    let (cid, host_port) = block_on_fut(env.docker.create_and_start_service_container(&spec))?
        .map_err(|mut e| {
            if let DockerError::Internal(ref mut m) = e {
                *m = scrub_secrets(m, &scrub_refs);
            }
            map_docker_err(e, socket_path)
        })?;

    let retries = req.health_retries.clamp(1, 30);
    let check_outcome: Result<(), String> = block_on_fut(async {
        if req.port == 0 {
            let checks = retries.min(5);
            for _ in 0..checks {
                tokio::time::sleep(Duration::from_millis(200)).await;
                if !env.docker.is_container_running(&cid).await.unwrap_or(false) {
                    return Err("Worker container exited during startup.".to_owned());
                }
            }
            return Ok(());
        }

        let mut last_err = "Container did not pass health check.".to_owned();
        for attempt in 0..retries {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            match env.docker.is_container_running(&cid).await {
                Ok(true) => {}
                Ok(false) => {
                    return Err("Container exited before passing its health check.".to_owned());
                }
                Err(_) => {}
            }
            match http_health_probe(host_port, &req.health_path).await {
                Ok(code) if (200..400).contains(&code) => return Ok(()),
                Ok(code) => {
                    last_err = format!("Health check {} returned HTTP {code}.", req.health_path);
                }
                Err(e) => {
                    last_err = e;
                }
            }
        }
        Err(last_err)
    })?;

    if let Err(reason) = check_outcome {
        let log_tail = block_on_fut(env.docker.logs(&cid, 20))
            .ok()
            .and_then(|r| r.ok())
            .map(|lines| {
                lines
                    .into_iter()
                    .map(|l| scrub_secrets(&l.text, &scrub_refs))
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .filter(|s| !s.is_empty());
        let _ = block_on_fut(env.docker.force_stop_and_remove_container(&cid));
        let clean_reason = scrub_secrets(&reason, &scrub_refs);
        let msg = match log_tail {
            Some(tail) => format!("{clean_reason} Container logs: {tail}"),
            None => clean_reason,
        };
        return Err(err(PrivErrorCode::Internal, msg));
    }

    Ok(DeployStartResult {
        container_id: cid,
        host_port,
    })
}

pub fn stop_container_with(env: &Env, container_id: &str) -> Result<(), PrivError> {
    if !valid_docker_ref(container_id) {
        return Err(err(PrivErrorCode::Invalid, "Invalid container ID."));
    }
    block_on_fut(env.docker.force_stop_and_remove_container(container_id))?
        .map_err(|e| map_docker_err(e, env.docker.socket_path()))
}

pub fn image_exists_with(env: &Env, image_ref: &str) -> Result<bool, PrivError> {
    if !valid_docker_image_ref(image_ref) {
        return Err(err(PrivErrorCode::Invalid, "Invalid image reference."));
    }
    block_on_fut(env.docker.image_exists(image_ref))?
        .map_err(|e| map_docker_err(e, env.docker.socket_path()))
}

pub fn prune_images_with(
    env: &Env,
    service: &str,
    image_refs: &[String],
) -> Result<Vec<String>, PrivError> {
    if !valid_service_name(service) {
        return Err(err(PrivErrorCode::Invalid, "Invalid service name."));
    }
    let expected_prefix = format!("opsbox/{service}:");
    let mut removed = Vec::new();
    for img in image_refs {
        if !img.starts_with(&expected_prefix) || !valid_docker_image_ref(img) {
            continue;
        }
        if matches!(block_on_fut(env.docker.remove_image(img))?, Ok(true)) {
            removed.push(img.clone());
        }
    }
    Ok(removed)
}

pub fn delete_service_with(
    env: &Env,
    service: &str,
    container_ids: &[String],
    image_refs: &[String],
) -> Result<(), PrivError> {
    if !valid_service_name(service) {
        return Err(err(PrivErrorCode::Invalid, "Invalid service name."));
    }
    for cid in container_ids {
        if valid_docker_ref(cid) {
            let _ = block_on_fut(env.docker.force_stop_and_remove_container(cid));
        }
    }
    if let Ok(Ok(all)) = block_on_fut(env.docker.list_containers()) {
        for c in all {
            if c.labels.get("opsbox.service").map(String::as_str) == Some(service) {
                let _ = block_on_fut(env.docker.force_stop_and_remove_container(&c.full_id));
            }
        }
    }
    let _ = prune_images_with(env, service, image_refs)?;
    let svc_dir = env.apps_dir.join(service);
    if svc_dir.exists() {
        let _ = std::fs::remove_dir_all(&svc_dir);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_repo_urls_and_secret_contexts() {
        assert!(
            validate_repo_url(
                DeployGitSource::Github,
                "https://github.com/acme/web.git",
                false
            )
            .is_ok()
        );
        assert!(
            validate_repo_url(
                DeployGitSource::Github,
                "https://evil.com/acme/web.git",
                false
            )
            .is_err()
        );
        assert!(
            validate_repo_url(
                DeployGitSource::DeployKey,
                "git@github.com:acme/web.git",
                false
            )
            .is_ok()
        );
        assert!(validate_repo_url(DeployGitSource::DeployKey, "-oProxyCommand=id", false).is_err());
        assert!(
            validate_repo_url(DeployGitSource::GitUrl, "http://127.0.0.1/repo.git", false).is_err()
        );
    }

    #[test]
    fn prunes_old_source_checkouts_keeping_latest_five_and_current() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(src.join("_work-stale")).unwrap();
        for i in 1..=7 {
            std::fs::create_dir_all(src.join(format!("sha{i:07}"))).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        prune_old_source_dirs(&src, 5, "sha0000001");
        assert!(!src.join("_work-stale").exists());
        assert!(src.join("sha0000001").exists());
    }
}
