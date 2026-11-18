//! Privileged runtime execution for built-in service templates and Docker Compose stacks
//! (docs/BACKEND_SPEC.md 7.6).

use std::collections::HashSet;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use opsbox_deploy::compose::{
    extract_compose_services, inspect_compose_risks, precheck_compose_yaml,
};
use opsbox_deploy::pipeline::scrub_secrets;
use opsbox_deploy::templates::{
    compute_rotated_vars, get_template, random_password, rotated_secret_key, template_volume_name,
};
use opsbox_docker::{DockerError, PortBindMode, TemplateContainerSpec};
use opsbox_proto::{
    ComposeDeployRequest, ComposeDeployResult, ComposeDeployedService, ComposeValidateRequest,
    ComposeValidationResult, DatabaseRotateRequest, DatabaseRotateResult, DeployEnvSpec,
    DeployStartResult, DockerExecRequest, PrivError, PrivErrorCode, PrivReply, Secret,
    TemplateStartRequest, valid_compose_service_name, valid_db_identifier, valid_docker_image_ref,
    valid_docker_ref, valid_service_name,
};
use serde_json::Value;
use tempfile::NamedTempFile;

use super::Env;
use super::deploy::{
    err, find_binary, http_health_probe_host, open_and_resolve_deploy_vars, open_deploy_vars,
    safe_env_base, short_deploy_suffix,
};
use super::docker::{block_on_fut, map_docker_err};
use super::sealed;

const MAX_COMPOSE_LOG_LINES: usize = 500;

fn map_deploy_err(e: opsbox_deploy::DeployError) -> PrivError {
    let code = match e {
        opsbox_deploy::DeployError::Validation { .. } => PrivErrorCode::Invalid,
        opsbox_deploy::DeployError::Unauthorized(_) => PrivErrorCode::Forbidden,
        opsbox_deploy::DeployError::NotFound(_) => PrivErrorCode::NotFound,
        _ => PrivErrorCode::Internal,
    };
    err(code, e.to_string())
}

/// Drop guard that removes a freshly-created stack directory unless `keep = true`.
struct StackDirGuard {
    path: PathBuf,
    keep: bool,
}

impl Drop for StackDirGuard {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// Pull an official template image, ensure its named Docker volumes exist, launch the
/// container on the project network (with optional loopback binding), and wait for its health check.
pub fn start_template_with(
    env: &Env,
    req: &TemplateStartRequest,
) -> Result<DeployStartResult, PrivError> {
    if !valid_service_name(&req.project)
        || !valid_service_name(&req.service)
        || !valid_docker_ref(&req.deploy_id)
        || !valid_docker_image_ref(&req.image_ref)
    {
        return Err(err(
            PrivErrorCode::Invalid,
            "Invalid template instance parameters.",
        ));
    }

    let tmpl = get_template(&req.template_id).ok_or_else(|| {
        err(
            PrivErrorCode::Invalid,
            format!("Unknown service template '{}'.", req.template_id),
        )
    })?;
    if !tmpl.versions.iter().any(|v| v == &req.version) {
        return Err(err(
            PrivErrorCode::Invalid,
            format!(
                "Version '{}' is not allowed for template '{}'.",
                req.version, tmpl.id
            ),
        ));
    }
    let expected_image = format!("{}:{}", tmpl.image, req.version);
    if req.image_ref != expected_image {
        return Err(err(
            PrivErrorCode::Invalid,
            "Template image reference does not match template specification.",
        ));
    }

    if req.public_port && req.internal_only {
        return Err(err(
            PrivErrorCode::Invalid,
            "Cannot publish a public port while internal-only is enabled.",
        ));
    }
    if req.public_port && !req.confirm_public_port {
        return Err(err(
            PrivErrorCode::ConfirmRequired,
            format!(
                "Publishing {} port {} to the internet (0.0.0.0) exposes it to external network and brute-force attacks. Keep it on the private project network or bind to 127.0.0.1 instead, or confirm to publish publicly.",
                req.service, tmpl.port
            ),
        ));
    }

    let (opened_vars, scrub_owned) =
        open_and_resolve_deploy_vars(&env.secrets, &req.service, &req.vars, &req.ref_sources)?;
    let scrub_refs: Vec<&str> = scrub_owned.iter().map(String::as_str).collect();

    // Keep client connection helper vars (e.g. PGHOST/PGDATABASE/DATABASE_URL on Postgres)
    // out of container env unless explicitly part of the template's `[env]` list.
    let template_env_keys: HashSet<&str> = tmpl.env.iter().map(|e| e.key.as_str()).collect();
    let conn_only_keys: HashSet<&str> = tmpl
        .connection
        .vars
        .iter()
        .map(|c| c.key.as_str())
        .filter(|k| !template_env_keys.contains(k))
        .collect();
    let env_strings: Vec<String> = opened_vars
        .into_iter()
        .filter(|v| !conn_only_keys.contains(v.key.as_str()))
        // Per-app logins and pooler credentials live on the service for apps to reference;
        // the database container itself must never see them.
        .filter(|v| !v.key.starts_with(opsbox_proto::INTERNAL_VAR_PREFIX))
        .map(|v| format!("{}={}", v.key, v.value))
        .collect();

    if let Some(vol) = req.data_volume.as_deref()
        && (!valid_docker_ref(vol) || tmpl.volumes.is_empty())
    {
        return Err(err(PrivErrorCode::Invalid, "Invalid data volume name."));
    }
    let volume_binds: Vec<(String, String)> = tmpl
        .volumes
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let name = match (i, req.data_volume.as_deref()) {
                (0, Some(custom)) => custom.to_owned(),
                _ => template_volume_name(&req.service, &v.suffix),
            };
            (name, v.container_path.clone())
        })
        .collect();

    let suffix = short_deploy_suffix(&req.deploy_id);
    let container_name = format!("opsbox-{}-{suffix}", req.service);
    let network_name = format!("opsbox-{}", req.project);
    let socket_path = env.docker.socket_path();

    super::capacity::check_image(env, &req.image_ref)?;
    block_on_fut(env.docker.pull_image(&req.image_ref))?.map_err(|mut e| {
        if let DockerError::Internal(ref mut m) = e {
            *m = scrub_secrets(m, &scrub_refs);
        }
        map_docker_err(e, socket_path)
    })?;

    let spec = TemplateContainerSpec {
        project: &req.project,
        service: &req.service,
        template_id: &req.template_id,
        deploy_id: &req.deploy_id,
        container_name: &container_name,
        network_name: &network_name,
        image_ref: &req.image_ref,
        env: &env_strings,
        cmd: &tmpl.cmd,
        volume_binds: &volume_binds,
        container_port: tmpl.port,
        port_bind: if req.internal_only {
            PortBindMode::InternalOnly
        } else if req.public_port {
            PortBindMode::Public
        } else {
            PortBindMode::Loopback
        },
        memory_limit_mb: Some(req.memory_limit_mb.unwrap_or(tmpl.min_memory_mb as i64)),
        cpu_limit_millis: req.cpu_limit_millis,
    };

    let (cid, host_port, container_ip) =
        block_on_fut(env.docker.create_and_start_template_container(&spec))?.map_err(|mut e| {
            if let DockerError::Internal(ref mut m) = e {
                *m = scrub_secrets(m, &scrub_refs);
            }
            map_docker_err(e, socket_path)
        })?;

    let check_outcome: Result<(), String> = block_on_fut(wait_for_template_health(
        &env.docker,
        &cid,
        tmpl,
        req.internal_only,
        host_port,
        container_ip.as_deref(),
        &scrub_refs,
    ))?;

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

pub(crate) async fn wait_for_template_health(
    docker: &opsbox_docker::DockerClient,
    cid: &str,
    tmpl: &opsbox_deploy::templates::TemplateSpec,
    internal_only: bool,
    host_port: u16,
    container_ip: Option<&str>,
    scrub_refs: &[&str],
) -> Result<(), String> {
    let mut last_err = format!("{} container did not become healthy.", tmpl.name);
    let max_attempts = 40u8;
    for attempt in 0..max_attempts {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        if let Ok(false) = docker.is_container_running(cid).await {
            return Err(format!("{} container exited during startup.", tmpl.name));
        }

        if !tmpl.health.exec.is_empty() {
            let exec_req = DockerExecRequest {
                id: cid.to_owned(),
                cmd: tmpl.health.exec.clone(),
                env: Vec::new(),
                user: None,
                working_dir: None,
                stdin: None,
            };
            match docker.exec(exec_req).await {
                Ok(out) if out.exit_code == 0 => return Ok(()),
                Ok(out) => {
                    let detail = if !out.stderr.trim().is_empty() {
                        out.stderr.trim().to_owned()
                    } else {
                        format!("exit code {}", out.exit_code)
                    };
                    last_err = scrub_secrets(&detail, scrub_refs);
                }
                Err(e) => last_err = scrub_secrets(&e.to_string(), scrub_refs),
            }
            continue;
        }

        let probe_target = if !internal_only && host_port > 0 {
            Some(("127.0.0.1", host_port))
        } else if let Some(ip) = container_ip {
            let port = tmpl.health.tcp_port.unwrap_or(tmpl.port);
            (port > 0).then_some((ip, port))
        } else {
            None
        };

        if let Some((host, port)) = probe_target {
            if let Some(http_path) = tmpl.health.http_path.as_deref() {
                match http_health_probe_host(host, port, http_path).await {
                    Ok(code) if (200..400).contains(&code) => return Ok(()),
                    Ok(code) => {
                        last_err = format!("Health check {http_path} returned HTTP {code}.")
                    }
                    Err(e) => last_err = e,
                }
                continue;
            }
            if tmpl.health.tcp_port.is_some() {
                match http_health_probe_host(host, port, "tcp").await {
                    Ok(_) => return Ok(()),
                    Err(e) => last_err = e,
                }
                continue;
            }
        }

        // Fallback when container is internal-only with no bridge IP reported yet.
        if attempt >= 2 {
            return Ok(());
        }
    }
    Err(last_err)
}

fn require_docker_cli(env: &Env) -> Result<PathBuf, PrivError> {
    find_binary(
        env.docker_bin.as_deref(),
        &["docker"],
        &["/usr/bin/docker", "/usr/local/bin/docker"],
    )
    .ok_or_else(|| {
        err(
            PrivErrorCode::Unsupported,
            "Docker CLI is not installed on this server.",
        )
    })
}

struct ComposeCmdCtx<'a> {
    docker_bin: &'a Path,
    socket_path: &'a Path,
    compose_file: &'a Path,
    project_dir: &'a Path,
    compose_project: &'a str,
    extra_env: &'a [(&'a str, &'a str)],
    scrub_refs: &'a [&'a str],
}

async fn run_compose_cmd(
    ctx: &ComposeCmdCtx<'_>,
    subargs: &[&str],
    timeout: Duration,
) -> Result<(i32, String, String), PrivError> {
    let docker_host = format!("unix://{}", ctx.socket_path.display());
    let mut env_pairs: Vec<(&str, &str)> = Vec::with_capacity(ctx.extra_env.len() + 2);
    env_pairs.push(("DOCKER_HOST", &docker_host));
    env_pairs.push(("COMPOSE_PROJECT_NAME", ctx.compose_project));
    env_pairs.extend_from_slice(ctx.extra_env);

    let mut cmd = tokio::process::Command::new(ctx.docker_bin);
    safe_env_base(&mut cmd, &env_pairs);
    cmd.kill_on_drop(true);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.arg("compose")
        .arg("-f")
        .arg(ctx.compose_file)
        .arg("--project-directory")
        .arg(ctx.project_dir)
        .arg("-p")
        .arg(ctx.compose_project)
        .args(subargs);

    let out = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| {
            err(
                PrivErrorCode::Internal,
                format!(
                    "docker compose timed out after {} seconds.",
                    timeout.as_secs()
                ),
            )
        })?
        .map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Failed to execute docker compose: {e}"),
            )
        })?;

    let code = out.status.code().unwrap_or(-1);
    let stdout = scrub_secrets(&String::from_utf8_lossy(&out.stdout), ctx.scrub_refs);
    let stderr = scrub_secrets(&String::from_utf8_lossy(&out.stderr), ctx.scrub_refs);
    Ok((code, stdout, stderr))
}

struct PreparedCompose {
    validation: ComposeValidationResult,
    canon_dir: PathBuf,
    raw_stack_dir: PathBuf,
    existed_before: bool,
}

fn prepare_and_validate_compose(
    env: &Env,
    stack: &str,
    yaml: &str,
    confirm: bool,
    extra_env: &[(&str, &str)],
    scrub_refs: &[&str],
) -> Result<PreparedCompose, PrivError> {
    if !valid_service_name(stack) {
        return Err(err(
            PrivErrorCode::Invalid,
            "Stack name must be lowercase letters, numbers, and hyphens.",
        ));
    }

    std::fs::create_dir_all(&env.apps_dir).map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Cannot prepare apps directory: {e}"),
        )
    })?;
    let canon_apps = std::fs::canonicalize(&env.apps_dir).map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Cannot canonicalize apps directory: {e}"),
        )
    })?;

    let raw_stack_dir = env.apps_dir.join(stack);
    let canon_dir = canon_apps.join(stack);
    let precheck = precheck_compose_yaml(yaml, &canon_dir).map_err(map_deploy_err)?;

    let docker_bin = require_docker_cli(env)?;
    let existed_before = raw_stack_dir.exists();
    if !existed_before {
        std::fs::create_dir_all(&raw_stack_dir).map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Cannot prepare stack directory: {e}"),
            )
        })?;
        let _ = std::fs::set_permissions(&raw_stack_dir, std::fs::Permissions::from_mode(0o750));
    }
    let _dir_guard = StackDirGuard {
        path: raw_stack_dir.clone(),
        keep: existed_before,
    };

    let tmp_file = NamedTempFile::new_in(&raw_stack_dir).map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Cannot stage temporary Compose file: {e}"),
        )
    })?;
    std::fs::write(tmp_file.path(), yaml).map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Cannot write temporary Compose file: {e}"),
        )
    })?;

    let compose_project = format!("opsbox-{stack}");
    let cmd_ctx = ComposeCmdCtx {
        docker_bin: &docker_bin,
        socket_path: env.docker.socket_path(),
        compose_file: tmp_file.path(),
        project_dir: &canon_dir,
        compose_project: &compose_project,
        extra_env,
        scrub_refs,
    };

    let (mut code, mut stdout, mut stderr) = block_on_fut(run_compose_cmd(
        &cmd_ctx,
        &["config", "--format", "json"],
        Duration::from_secs(30),
    ))??;

    let mut used_yaml_fallback = false;
    if code != 0 && stderr.contains("unknown flag") && stderr.contains("--format") {
        let retry = block_on_fut(run_compose_cmd(
            &cmd_ctx,
            &["config"],
            Duration::from_secs(30),
        ))??;
        code = retry.0;
        stdout = retry.1;
        stderr = retry.2;
        used_yaml_fallback = true;
    }

    if code != 0 {
        let detail = if !stderr.trim().is_empty() {
            stderr.trim().to_owned()
        } else {
            format!("docker compose config exited with status {code}")
        };
        return Err(err(
            PrivErrorCode::Invalid,
            format!("Compose validation failed: {detail}"),
        ));
    }

    let resolved_doc: Value = if used_yaml_fallback {
        opsbox_deploy::compose::parse_compose_yaml(&stdout).map_err(map_deploy_err)?
    } else {
        serde_json::from_str(&stdout).map_err(|e| {
            err(
                PrivErrorCode::Invalid,
                format!("Could not parse resolved Compose config JSON: {e}"),
            )
        })?
    };

    let pre_risks = inspect_compose_risks(&precheck.doc, &canon_dir);
    let post_risks = inspect_compose_risks(&resolved_doc, &canon_dir);
    let mut warnings = pre_risks.reasons;
    for r in post_risks.reasons {
        if !warnings.contains(&r) {
            warnings.push(r);
        }
    }
    if !warnings.is_empty() && !confirm {
        let summary = format!(
            "This Compose file requests elevated access that needs your confirmation: {}. Continue only if you trust this Compose file.",
            warnings.join("; ")
        );
        return Err(err(PrivErrorCode::ConfirmRequired, summary));
    }

    let (services, _) = extract_compose_services(&resolved_doc).map_err(map_deploy_err)?;
    for svc in &services {
        if !valid_compose_service_name(&svc.name) {
            return Err(err(
                PrivErrorCode::Invalid,
                format!("Invalid Compose service name '{}'.", svc.name),
            ));
        }
    }

    Ok(PreparedCompose {
        validation: ComposeValidationResult {
            project_name: compose_project,
            service_dir: canon_dir.display().to_string(),
            services,
            warnings,
        },
        canon_dir,
        raw_stack_dir,
        existed_before,
    })
}

/// Validate a Compose file inside `opsbox-priv` via `docker compose config` and safety policies
/// without persisting files or starting containers.
pub fn validate_compose_with(
    env: &Env,
    req: &ComposeValidateRequest,
) -> Result<ComposeValidationResult, PrivError> {
    let prepared = prepare_and_validate_compose(env, &req.stack, &req.yaml, req.confirm, &[], &[])?;
    Ok(prepared.validation)
}

/// Validate, write `/var/lib/opsbox/apps/<stack>/compose.yml`, run `docker compose -p opsbox-<stack> up -d`,
/// and connect all started containers to the project's `opsbox-<project>` bridge network.
pub fn deploy_compose_with(
    env: &Env,
    req: &ComposeDeployRequest,
) -> Result<ComposeDeployResult, PrivError> {
    if !valid_service_name(&req.project) || !valid_service_name(&req.stack) {
        return Err(err(
            PrivErrorCode::Invalid,
            "Invalid project or Compose stack name.",
        ));
    }

    let mut normalized = req.clone();
    let doc = opsbox_deploy::compose::parse_compose_yaml(&req.yaml)
        .map_err(|e| err(PrivErrorCode::Invalid, e.to_string()))?;
    normalized.yaml = serde_json::to_string(
        &opsbox_deploy::compose::with_capacity_limits(&doc)
            .map_err(|e| err(PrivErrorCode::Invalid, e.to_string()))?,
    )
    .map_err(|e| err(PrivErrorCode::Internal, e.to_string()))?;
    let req = &normalized;
    let (_, _, has_builds) = opsbox_deploy::compose::capacity_requirements(&doc)
        .map_err(|e| err(PrivErrorCode::Invalid, e.to_string()))?;
    let builder = if has_builds {
        Some(block_on_fut(super::deploy::bounded_builder(
            env, &req.stack,
        ))??)
    } else {
        None
    };
    let (opened_vars, scrub_owned) =
        open_and_resolve_deploy_vars(&env.secrets, &req.stack, &req.vars, &req.ref_sources)?;
    let scrub_refs: Vec<&str> = scrub_owned.iter().map(String::as_str).collect();
    let extra_env: Vec<(&str, &str)> = opened_vars
        .iter()
        .map(|v| (v.key.as_str(), v.value.as_str()))
        .collect();

    let prepared = prepare_and_validate_compose(
        env,
        &req.stack,
        &req.yaml,
        req.confirm,
        &extra_env,
        &scrub_refs,
    )?;

    if !prepared.existed_before {
        std::fs::create_dir_all(&prepared.raw_stack_dir).map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Cannot prepare stack directory: {e}"),
            )
        })?;
        let _ = std::fs::set_permissions(
            &prepared.raw_stack_dir,
            std::fs::Permissions::from_mode(0o750),
        );
    }
    let mut dir_guard = StackDirGuard {
        path: prepared.raw_stack_dir.clone(),
        keep: prepared.existed_before,
    };

    let compose_path = prepared.raw_stack_dir.join("compose.yml");
    let mut tmp = NamedTempFile::new_in(&prepared.raw_stack_dir).map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Cannot stage compose.yml: {e}"),
        )
    })?;
    std::io::Write::write_all(&mut tmp, req.yaml.as_bytes()).map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Cannot write compose.yml: {e}"),
        )
    })?;
    let _ = std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o640));
    tmp.persist(&compose_path).map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Cannot install compose.yml: {}", e.error),
        )
    })?;

    let network_name = format!("opsbox-{}", req.project);
    let socket_path = env.docker.socket_path();
    block_on_fut(env.docker.ensure_network(&network_name, &req.project))?
        .map_err(|e| map_docker_err(e, socket_path))?;

    let docker_bin = require_docker_cli(env)?;
    let cmd_ctx = ComposeCmdCtx {
        docker_bin: &docker_bin,
        socket_path,
        compose_file: &compose_path,
        project_dir: &prepared.canon_dir,
        compose_project: &prepared.validation.project_name,
        extra_env: &extra_env,
        scrub_refs: &scrub_refs,
    };
    if let Some(builder) = &builder {
        let (code, _, _) = block_on_fut(run_compose_cmd(
            &cmd_ctx,
            &["build", "--builder", &builder.name],
            Duration::from_secs(1800),
        ))??;
        if code != 0 {
            return Err(err(
                PrivErrorCode::Internal,
                "Compose build failed inside its memory limit. Build in GitHub Actions and pull the image instead.",
            ));
        }
    }
    let (code, stdout, stderr) = block_on_fut(run_compose_cmd(
        &cmd_ctx,
        &["up", "-d", "--remove-orphans", "--no-build"],
        Duration::from_secs(300),
    ))??;

    let mut log_lines = Vec::new();
    for src in [&stdout, &stderr] {
        for line in src.lines() {
            let trimmed = line.trim_end();
            if !trimmed.is_empty() && log_lines.len() < MAX_COMPOSE_LOG_LINES {
                log_lines.push(trimmed.to_owned());
            }
        }
    }

    if code != 0 {
        if !prepared.existed_before {
            let down_ctx = ComposeCmdCtx {
                extra_env: &[],
                ..cmd_ctx
            };
            let _ = block_on_fut(run_compose_cmd(
                &down_ctx,
                &["down", "--remove-orphans"],
                Duration::from_secs(30),
            ));
        }
        let detail = log_lines
            .last()
            .cloned()
            .unwrap_or_else(|| format!("docker compose up exited with status {code}"));
        return Err(err(
            PrivErrorCode::Internal,
            format!("Compose startup failed: {detail}"),
        ));
    }

    dir_guard.keep = true;

    let runtime = block_on_fut(
        env.docker
            .find_compose_containers(&prepared.validation.project_name),
    )?
    .map_err(|e| map_docker_err(e, socket_path))?;

    for c in &runtime {
        let aliases = vec![c.service_name.clone()];
        block_on_fut(env.docker.connect_container_to_network(
            &c.id,
            &network_name,
            &req.project,
            &aliases,
        ))?
        .map_err(|e| map_docker_err(e, socket_path))?;
        if log_lines.len() < MAX_COMPOSE_LOG_LINES {
            log_lines.push(format!(
                "Attached {} to network {network_name}.",
                c.service_name
            ));
        }
    }

    let services = prepared
        .validation
        .services
        .into_iter()
        .map(|svc| {
            let found = runtime.iter().find(|r| r.service_name == svc.name);
            let host_port = found.and_then(|r| {
                if svc.port > 0 {
                    r.host_ports
                        .get(&svc.port)
                        .copied()
                        .or_else(|| r.host_ports.values().copied().next())
                } else {
                    r.host_ports.values().copied().next()
                }
            });
            ComposeDeployedService {
                name: svc.name,
                container_id: found.map(|r| r.id.clone()),
                image: svc.image,
                state: found
                    .map(|r| r.state.clone())
                    .unwrap_or_else(|| "running".to_owned()),
                port: svc.port,
                host_port,
                depends_on: svc.depends_on,
            }
        })
        .collect();

    Ok(ComposeDeployResult {
        project_name: prepared.validation.project_name,
        log_lines,
        services,
    })
}

/// Tear down an entire Docker Compose stack (`docker compose -p opsbox-<stack> down`)
/// and optionally remove `/var/lib/opsbox/apps/<stack>`.
pub fn down_compose_with(env: &Env, stack: &str, remove_files: bool) -> Result<(), PrivError> {
    if !valid_service_name(stack) {
        return Err(err(PrivErrorCode::Invalid, "Invalid Compose stack name."));
    }
    let compose_project = format!("opsbox-{stack}");
    let stack_dir = env.apps_dir.join(stack);
    let compose_file = stack_dir.join("compose.yml");

    if compose_file.exists()
        && let Ok(docker_bin) = require_docker_cli(env)
        && let Ok(canon_dir) = std::fs::canonicalize(&stack_dir)
    {
        let cmd_ctx = ComposeCmdCtx {
            docker_bin: &docker_bin,
            socket_path: env.docker.socket_path(),
            compose_file: &compose_file,
            project_dir: &canon_dir,
            compose_project: &compose_project,
            extra_env: &[],
            scrub_refs: &[],
        };
        let _ = block_on_fut(run_compose_cmd(
            &cmd_ctx,
            &["down", "--remove-orphans"],
            Duration::from_secs(90),
        ));
    }

    // Ensure any Compose-labeled containers are stopped/removed even if compose.yml was already removed.
    if let Ok(Ok(remaining)) = block_on_fut(env.docker.find_compose_containers(&compose_project)) {
        for c in remaining {
            let _ = block_on_fut(env.docker.force_stop_and_remove_container(&c.id));
        }
    }

    if remove_files
        && stack_dir.exists()
        && let (Ok(canon_apps), Ok(canon_stack)) = (
            std::fs::canonicalize(&env.apps_dir),
            std::fs::canonicalize(&stack_dir),
        )
        && canon_stack.starts_with(&canon_apps)
        && canon_stack != canon_apps
    {
        std::fs::remove_dir_all(&canon_stack).map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Failed to remove Compose stack directory: {e}"),
            )
        })?;
    }

    Ok(())
}

/// Stop and remove a single container belonging to a Compose stack.
pub fn remove_compose_service_with(env: &Env, stack: &str, service: &str) -> Result<(), PrivError> {
    if !valid_service_name(stack) || !valid_compose_service_name(service) {
        return Err(err(
            PrivErrorCode::Invalid,
            "Invalid Compose stack or service name.",
        ));
    }
    let compose_project = format!("opsbox-{stack}");
    let socket_path = env.docker.socket_path();
    let containers = block_on_fut(env.docker.find_compose_containers(&compose_project))?
        .map_err(|e| map_docker_err(e, socket_path))?;

    for c in containers {
        if c.service_name == service {
            block_on_fut(env.docker.force_stop_and_remove_container(&c.id))?
                .map_err(|e| map_docker_err(e, socket_path))?;
        }
    }
    Ok(())
}

/// Rotate credentials inside a running managed database container (`docker exec`) and return
/// newly sealed environment/connection variables without returning plaintext to `serve`.
pub fn rotate_database_with(
    env: &Env,
    req: &DatabaseRotateRequest,
) -> Result<DatabaseRotateResult, PrivError> {
    if req.service_id.is_empty()
        || req.service_id.len() > 64
        || req.service_id.contains("..")
        || !req
            .service_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        || !valid_service_name(&req.service_name)
        || !valid_docker_ref(&req.container_id)
    {
        return Err(err(
            PrivErrorCode::Invalid,
            "Invalid database service parameters for credential rotation.",
        ));
    }

    let tmpl = get_template(&req.template_id).ok_or_else(|| {
        err(
            PrivErrorCode::Invalid,
            format!("Unknown database template '{}'.", req.template_id),
        )
    })?;
    let secret_key = rotated_secret_key(&req.template_id).ok_or_else(|| {
        err(
            PrivErrorCode::Invalid,
            format!("Credential rotation is not supported for '{}'.", tmpl.name),
        )
    })?;

    for v in &req.vars {
        let expected_ctx = format!("deploy:var:{}:{}", req.service_id, v.key);
        if v.seal_context != expected_ctx {
            return Err(err(
                PrivErrorCode::Refused,
                "Secret sealing context does not match database service.",
            ));
        }
    }

    let (opened_vars, mut scrub_owned) = open_deploy_vars(&env.secrets, &req.vars)?;
    let find_var = |key: &str| -> Option<&str> {
        opened_vars
            .iter()
            .find(|v| v.key == key)
            .map(|v| v.value.as_str())
    };

    let running = match block_on_fut(env.docker.is_container_running(&req.container_id))? {
        Ok(r) => r,
        Err(DockerError::NotFound(_)) => false,
        Err(e) => return Err(map_docker_err(e, env.docker.socket_path())),
    };
    if !running {
        return Err(err(
            PrivErrorCode::Conflict,
            "Start or deploy this database before rotating its credentials.",
        ));
    }

    let new_password = random_password(24).map_err(map_deploy_err)?;
    let current_tuples: Vec<(String, String, bool)> = opened_vars
        .iter()
        .map(|v| (v.key.clone(), v.value.clone(), v.is_secret))
        .collect();
    let updated_vars =
        compute_rotated_vars(tmpl, &req.service_name, &current_tuples, &new_password)
            .map_err(map_deploy_err)?;

    scrub_owned.push(new_password.clone());
    for (_, val, sec) in &updated_vars {
        if *sec && !val.is_empty() {
            scrub_owned.push(val.clone());
        }
    }
    let scrub_refs: Vec<&str> = scrub_owned.iter().map(String::as_str).collect();

    let exec_req = match req.template_id.as_str() {
        t if opsbox_deploy::templates::is_postgres_template(t) => {
            let user = find_var("POSTGRES_USER").unwrap_or("app");
            let db = find_var("POSTGRES_DB").unwrap_or("app");
            let old_pw = find_var(secret_key).unwrap_or("");
            if !valid_db_identifier(user) || !valid_db_identifier(db) {
                return Err(err(
                    PrivErrorCode::Invalid,
                    "Invalid PostgreSQL user or database identifier.",
                ));
            }
            DockerExecRequest {
                id: req.container_id.clone(),
                cmd: vec![
                    "psql".into(),
                    "-v".into(),
                    "ON_ERROR_STOP=1".into(),
                    "-U".into(),
                    user.to_owned(),
                    "-d".into(),
                    db.to_owned(),
                ],
                user: None,
                env: vec![format!("PGPASSWORD={old_pw}")],
                working_dir: None,
                stdin: Some(
                    format!("ALTER USER \"{user}\" WITH PASSWORD '{new_password}';\n").into_bytes(),
                ),
            }
        }
        "mysql" => {
            let user = find_var("MYSQL_USER").unwrap_or("app");
            let old_root_pw = find_var("MYSQL_ROOT_PASSWORD").unwrap_or("");
            if !valid_db_identifier(user) {
                return Err(err(
                    PrivErrorCode::Invalid,
                    "Invalid MySQL user identifier.",
                ));
            }
            DockerExecRequest {
                id: req.container_id.clone(),
                cmd: vec![
                    "mysql".into(),
                    "-h".into(),
                    "127.0.0.1".into(),
                    "-uroot".into(),
                ],
                user: None,
                env: vec![format!("MYSQL_PWD={old_root_pw}")],
                working_dir: None,
                stdin: Some(
                    format!("ALTER USER '{user}'@'%' IDENTIFIED BY '{new_password}'; FLUSH PRIVILEGES;\n")
                        .into_bytes(),
                ),
            }
        }
        "mariadb" => {
            let user = find_var("MARIADB_USER").unwrap_or("app");
            let old_root_pw = find_var("MARIADB_ROOT_PASSWORD").unwrap_or("");
            if !valid_db_identifier(user) {
                return Err(err(
                    PrivErrorCode::Invalid,
                    "Invalid MariaDB user identifier.",
                ));
            }
            DockerExecRequest {
                id: req.container_id.clone(),
                cmd: vec![
                    "sh".into(),
                    "-c".into(),
                    "exec \"$(command -v mariadb || command -v mysql)\" -h 127.0.0.1 -uroot".into(),
                ],
                user: None,
                env: vec![format!("MYSQL_PWD={old_root_pw}")],
                working_dir: None,
                stdin: Some(
                    format!("ALTER USER '{user}'@'%' IDENTIFIED BY '{new_password}'; FLUSH PRIVILEGES;\n")
                        .into_bytes(),
                ),
            }
        }
        "redis" => {
            let old_pw = find_var(secret_key).unwrap_or("");
            DockerExecRequest {
                id: req.container_id.clone(),
                cmd: vec!["redis-cli".into(), "--no-auth-warning".into()],
                user: None,
                env: vec![format!("REDISCLI_AUTH={old_pw}")],
                working_dir: None,
                stdin: Some(format!("CONFIG SET requirepass {new_password}\n").into_bytes()),
            }
        }
        "mongo" => {
            let user = find_var("MONGO_INITDB_ROOT_USERNAME").unwrap_or("app");
            let old_pw = find_var(secret_key).unwrap_or("");
            if !valid_db_identifier(user) {
                return Err(err(
                    PrivErrorCode::Invalid,
                    "Invalid MongoDB user identifier.",
                ));
            }
            DockerExecRequest {
                id: req.container_id.clone(),
                cmd: vec![
                    "mongosh".into(),
                    "mongodb://127.0.0.1:27017/admin".into(),
                    "--quiet".into(),
                ],
                user: None,
                env: vec![
                    format!("OPSBOX_DB_USER={user}"),
                    format!("OPSBOX_OLD_PW={old_pw}"),
                    format!("OPSBOX_NEW_PW={new_password}"),
                ],
                working_dir: None,
                stdin: Some(
                    b"if (!db.auth(process.env.OPSBOX_DB_USER, process.env.OPSBOX_OLD_PW)) { quit(1); }\ndb.changeUserPassword(process.env.OPSBOX_DB_USER, process.env.OPSBOX_NEW_PW);\n"
                        .to_vec(),
                ),
            }
        }
        _ => {
            return Err(err(
                PrivErrorCode::Invalid,
                "Credential rotation is not supported for this template.",
            ));
        }
    };

    let exec_out = block_on_fut(env.docker.exec(exec_req))?.map_err(|mut e| {
        if let DockerError::Internal(ref mut m) = e {
            *m = scrub_secrets(m, &scrub_refs);
        }
        map_docker_err(e, env.docker.socket_path())
    })?;

    let stdout_txt = String::from_utf8_lossy(&exec_out.stdout);
    let redis_err = req.template_id == "redis"
        && (stdout_txt.contains("ERR ") || stdout_txt.contains("NOAUTH"));
    if exec_out.exit_code != 0 || redis_err {
        let raw_detail = if !exec_out.stderr.trim().is_empty() {
            exec_out.stderr.trim()
        } else if !stdout_txt.trim().is_empty() {
            stdout_txt.trim()
        } else {
            "command returned a non-zero exit code"
        };
        let clean = scrub_secrets(raw_detail, &scrub_refs);
        return Err(err(
            PrivErrorCode::Internal,
            format!("Database credential rotation failed: {clean}"),
        ));
    }

    let mut sealed_vars = Vec::with_capacity(updated_vars.len());
    for (key, val, is_secret) in updated_vars {
        let ctx = format!("deploy:var:{}:{key}", req.service_id);
        let PrivReply::Sealed { sealed } = sealed::seal(&env.secrets, &ctx, &Secret::new(val))?
        else {
            return Err(err(
                PrivErrorCode::Internal,
                "Unexpected response while sealing rotated credential.",
            ));
        };
        sealed_vars.push(DeployEnvSpec {
            key,
            value_enc: sealed,
            seal_context: ctx,
            is_secret,
        });
    }

    Ok(DatabaseRotateResult { vars: sealed_vars })
}
