//! Docker Engine client wrapper over `/var/run/docker.sock` via `bollard`
//! (docs/BACKEND_SPEC.md 7.4, 7.11).
//!
//! Only `opsbox-priv` depends on this crate; `serve` (`opsbox-api`) never touches
//! `/var/run/docker.sock`.

pub mod parse;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bollard::container::LogOutput;
use bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use bollard::query_parameters::{
    InspectContainerOptions, ListContainersOptionsBuilder, ListImagesOptionsBuilder,
    ListNetworksOptionsBuilder, ListVolumesOptions, ListVolumesOptionsBuilder, LogsOptionsBuilder,
    PruneBuildOptionsBuilder, PruneContainersOptionsBuilder, PruneImagesOptionsBuilder,
    PruneVolumesOptionsBuilder, RemoveContainerOptionsBuilder, RestartContainerOptionsBuilder,
    StatsOptionsBuilder, StopContainerOptionsBuilder,
};
use bollard::{API_DEFAULT_VERSION, ClientVersion, Docker};
use futures_util::future::join_all;
use futures_util::{Stream, StreamExt};
use opsbox_proto::{
    DockerContainer, DockerContainerAction, DockerContainerInspect, DockerContainerNetwork,
    DockerContainerStat, DockerExecOutput, DockerExecRequest, DockerImage, DockerLogLine,
    DockerLogStream, DockerMount, DockerNetwork, DockerPort, DockerVolume, valid_docker_ref,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

pub const DEFAULT_SOCKET_PATH: &str = "/var/run/docker.sock";
const MAX_EXEC_STDOUT: usize = 8 * 1024 * 1024;
const MAX_EXEC_STDERR: usize = 1024 * 1024;
const MAX_DF_BYTES: usize = 16 * 1024 * 1024;
const EXEC_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, thiserror::Error)]
pub enum DockerError {
    #[error("Docker daemon is not reachable: {0}")]
    DaemonUnavailable(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Invalid(String),
    #[error("Docker operation timed out.")]
    Timeout,
    #[error("{0}")]
    Internal(String),
}

impl DockerError {
    fn from_bollard(err: bollard::errors::Error, context: &str) -> Self {
        match err {
            bollard::errors::Error::SocketNotFoundError(_)
            | bollard::errors::Error::HyperLegacyError { .. }
            | bollard::errors::Error::IOError { .. } => Self::DaemonUnavailable(
                "Docker daemon is not running or its socket cannot be reached.".into(),
            ),
            bollard::errors::Error::RequestTimeoutError => Self::Timeout,
            bollard::errors::Error::DockerResponseServerError {
                status_code,
                message,
            } => {
                let clean = message.trim();
                match status_code {
                    404 => Self::NotFound(if clean.is_empty() {
                        format!("{context} was not found.")
                    } else {
                        clean.to_owned()
                    }),
                    409 => Self::Conflict(if clean.is_empty() {
                        format!("{context} is in a conflicting state.")
                    } else {
                        clean.to_owned()
                    }),
                    400 => Self::Invalid(if clean.is_empty() {
                        format!("Invalid Docker request for {context}.")
                    } else {
                        clean.to_owned()
                    }),
                    _ => Self::Internal(if clean.is_empty() {
                        format!("Docker returned HTTP {status_code} for {context}.")
                    } else {
                        clean.to_owned()
                    }),
                }
            }
            other => Self::Internal(format!("{context}: {other}")),
        }
    }
}

/// Summary of reclaimable Docker disk space for Cleanup scanners.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DockerCleanupScan {
    pub dangling_images_bytes: u64,
    pub dangling_images_count: usize,
    pub build_cache_bytes: u64,
    pub stopped_containers_bytes: u64,
    pub stopped_containers_count: usize,
    pub dangling_volumes_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockerPruneKind {
    Images,
    BuildCache,
    Containers,
    Volumes,
}

/// Returns `true` if a Docker binary or socket is present on the machine.
pub fn is_docker_installed(socket_path: &Path) -> bool {
    socket_path.exists()
        || [
            "/usr/bin/dockerd",
            "/usr/local/bin/dockerd",
            "/usr/bin/docker",
            "/usr/local/bin/docker",
        ]
        .iter()
        .any(|p| Path::new(p).exists())
}

/// Wrapper around `bollard::Docker` communicating exclusively over a Unix socket.
#[derive(Clone)]
pub struct DockerClient {
    socket_path: PathBuf,
    timeout_secs: u64,
    api_version: Arc<Mutex<Option<ClientVersion>>>,
    prev_cpu: Arc<Mutex<HashMap<String, (u64, u64)>>>,
}

impl std::fmt::Debug for DockerClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DockerClient")
            .field("socket_path", &self.socket_path)
            .field("timeout_secs", &self.timeout_secs)
            .finish()
    }
}

impl Default for DockerClient {
    fn default() -> Self {
        Self::new(DEFAULT_SOCKET_PATH)
    }
}

impl DockerClient {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            timeout_secs: 120,
            api_version: Arc::new(Mutex::new(None)),
            prev_cpu: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    fn connect_raw(&self, version: &ClientVersion) -> Result<Docker, DockerError> {
        if !self.socket_path.exists() {
            return Err(DockerError::DaemonUnavailable(
                "Docker daemon is not running.".into(),
            ));
        }
        let s = self.socket_path.to_string_lossy();
        Docker::connect_with_socket(&s, self.timeout_secs, version)
            .map_err(|e| DockerError::from_bollard(e, "connect"))
    }

    async fn connect(&self) -> Result<Docker, DockerError> {
        let cached = self
            .api_version
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .to_owned();
        if let Some(ver) = cached {
            return self.connect_raw(&ver);
        }
        let docker = self.connect_raw(API_DEFAULT_VERSION)?;
        let negotiated = docker
            .negotiate_version()
            .await
            .map_err(|e| DockerError::from_bollard(e, "version negotiation"))?;
        let ver = negotiated.client_version();
        *self
            .api_version
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(ver);
        Ok(negotiated)
    }

    /// Check daemon health and return `(version, api_version)`.
    pub async fn ping_version(&self) -> Result<(String, String), DockerError> {
        let docker = self.connect_raw(API_DEFAULT_VERSION)?;
        let ver = docker
            .version()
            .await
            .map_err(|e| DockerError::from_bollard(e, "version"))?;
        let api_ver = ver.api_version.clone().unwrap_or_else(|| "1.43".into());
        let parts: Vec<usize> = api_ver
            .split('.')
            .filter_map(|p| p.parse::<usize>().ok())
            .collect();
        if parts.len() == 2 {
            let negotiated = ClientVersion {
                major_version: parts[0],
                minor_version: parts[1],
            };
            let chosen = if negotiated < *API_DEFAULT_VERSION {
                negotiated
            } else {
                *API_DEFAULT_VERSION
            };
            *self
                .api_version
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(chosen);
        }
        Ok((ver.version.unwrap_or_else(|| "unknown".into()), api_ver))
    }

    /// List all containers (running and stopped).
    pub async fn list_containers(&self) -> Result<Vec<DockerContainer>, DockerError> {
        let docker = self.connect().await?;
        let opts = ListContainersOptionsBuilder::default().all(true).build();
        let raw = docker
            .list_containers(Some(opts))
            .await
            .map_err(|e| DockerError::from_bollard(e, "list containers"))?;
        let mut out: Vec<DockerContainer> = raw
            .into_iter()
            .map(|c| {
                let full_id = c.id.unwrap_or_default();
                let id = parse::short_id(&full_id);
                let name = parse::clean_name(c.names.as_deref(), &full_id);
                let labels_map = c.labels.unwrap_or_default();
                let (managed, service, deploy) = parse::opsbox_labels(&labels_map);
                let labels: BTreeMap<String, String> = labels_map.into_iter().collect();
                let (port_list, ports) = parse::map_ports(c.ports.as_deref());
                DockerContainer {
                    id,
                    full_id,
                    name,
                    image: c.image.unwrap_or_else(|| "unknown".into()),
                    state: c.state.map_or_else(|| "unknown".into(), |s| s.to_string()),
                    status: c.status.unwrap_or_default(),
                    created: c.created.unwrap_or(0),
                    ports,
                    port_list,
                    labels,
                    managed,
                    service,
                    deploy,
                }
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Inspect one container by ID or name.
    pub async fn inspect_container(&self, id: &str) -> Result<DockerContainerInspect, DockerError> {
        if !valid_docker_ref(id) {
            return Err(DockerError::Invalid(
                "That is not a valid container name or ID.".into(),
            ));
        }
        let docker = self.connect().await?;
        let raw = docker
            .inspect_container(id, None::<InspectContainerOptions>)
            .await
            .map_err(|e| DockerError::from_bollard(e, "container"))?;

        let full_id = raw.id.unwrap_or_default();
        let short = parse::short_id(&full_id);
        let name = raw
            .name
            .as_deref()
            .map(|n| n.trim_start_matches('/').to_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| short.clone());
        let config = raw.config.unwrap_or_default();
        let state = raw.state.unwrap_or_default();
        let labels_map = config.labels.unwrap_or_default();
        let (managed, service, deploy) = parse::opsbox_labels(&labels_map);
        let labels: BTreeMap<String, String> = labels_map.into_iter().collect();

        let mut ports = Vec::new();
        if let Some(net_ports) = raw.network_settings.as_ref().and_then(|n| n.ports.as_ref()) {
            for (spec, bindings) in net_ports {
                let (port_str, proto) = spec.split_once('/').unwrap_or((spec.as_str(), "tcp"));
                let Ok(priv_port) = port_str.parse::<u16>() else {
                    continue;
                };
                if let Some(Some(list)) = bindings.as_ref().map(|b| (!b.is_empty()).then_some(b)) {
                    for b in list {
                        ports.push(DockerPort {
                            ip: b.host_ip.clone().filter(|s| !s.is_empty()),
                            private_port: priv_port,
                            public_port: b.host_port.as_deref().and_then(|p| p.parse::<u16>().ok()),
                            protocol: proto.to_owned(),
                        });
                    }
                } else {
                    ports.push(DockerPort {
                        ip: None,
                        private_port: priv_port,
                        public_port: None,
                        protocol: proto.to_owned(),
                    });
                }
            }
        }
        ports.sort_by_key(|p| (p.private_port, p.public_port));

        let mounts = raw
            .mounts
            .unwrap_or_default()
            .into_iter()
            .map(|m| DockerMount {
                kind: m.typ.map_or_else(|| "volume".into(), |t| t.to_string()),
                source: m.source.or(m.name).unwrap_or_default(),
                destination: m.destination.unwrap_or_default(),
                mode: m.mode.unwrap_or_default(),
                rw: m.rw.unwrap_or(true),
            })
            .collect();

        let mut networks: Vec<DockerContainerNetwork> = raw
            .network_settings
            .and_then(|n| n.networks)
            .unwrap_or_default()
            .into_iter()
            .map(|(net_name, ep)| DockerContainerNetwork {
                name: net_name,
                ip_address: ep.ip_address.unwrap_or_default(),
                gateway: ep.gateway.unwrap_or_default(),
                mac_address: ep.mac_address.unwrap_or_default(),
            })
            .collect();
        networks.sort_by(|a, b| a.name.cmp(&b.name));

        let env = parse::mask_env(&config.env.unwrap_or_default());

        Ok(DockerContainerInspect {
            id: short,
            full_id,
            name,
            image: config.image.or(raw.image).unwrap_or_default(),
            created: raw.created.map(|t| t.to_string()).unwrap_or_default(),
            state: state
                .status
                .map_or_else(|| "unknown".into(), |s| s.to_string()),
            running: state.running.unwrap_or(false),
            restarting: state.restarting.unwrap_or(false),
            paused: state.paused.unwrap_or(false),
            oom_killed: state.oom_killed.unwrap_or(false),
            exit_code: state.exit_code.unwrap_or(0),
            started_at: state.started_at.unwrap_or_default(),
            finished_at: state.finished_at.unwrap_or_default(),
            restart_count: raw
                .restart_count
                .and_then(|c| u32::try_from(c).ok())
                .unwrap_or(0),
            pid: state.pid,
            cmd: config.cmd.unwrap_or_default(),
            entrypoint: config.entrypoint.unwrap_or_default(),
            working_dir: config.working_dir.unwrap_or_default(),
            user: config.user.unwrap_or_default(),
            ports,
            mounts,
            networks,
            env,
            labels,
            managed,
            service,
            deploy,
        })
    }

    /// Perform start, stop, restart, or remove on a container.
    pub async fn container_action(
        &self,
        id: &str,
        action: DockerContainerAction,
    ) -> Result<(), DockerError> {
        if !valid_docker_ref(id) {
            return Err(DockerError::Invalid(
                "That is not a valid container name or ID.".into(),
            ));
        }
        let docker = self.connect().await?;
        let res = match action {
            DockerContainerAction::Start => docker.start_container(id, None).await,
            DockerContainerAction::Stop => {
                let opts = StopContainerOptionsBuilder::default().t(10).build();
                docker.stop_container(id, Some(opts)).await
            }
            DockerContainerAction::Restart => {
                let opts = RestartContainerOptionsBuilder::default().t(10).build();
                docker.restart_container(id, Some(opts)).await
            }
            DockerContainerAction::Remove => {
                let opts = RemoveContainerOptionsBuilder::default()
                    .force(false)
                    .build();
                docker.remove_container(id, Some(opts)).await
            }
        };
        match res {
            Ok(()) => Ok(()),
            // Docker returns 304 Not Modified when starting an already running container
            // or stopping an already stopped container.
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 304, ..
            }) => Ok(()),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 409, ..
            }) if action == DockerContainerAction::Remove => Err(DockerError::Conflict(
                "Stop the container before deleting it.".into(),
            )),
            Err(e) => Err(DockerError::from_bollard(e, action.as_str())),
        }
    }

    /// Fetch the last `tail` container log lines (non-following).
    pub async fn logs(&self, id: &str, tail: u32) -> Result<Vec<DockerLogLine>, DockerError> {
        if !valid_docker_ref(id) {
            return Err(DockerError::Invalid(
                "That is not a valid container name or ID.".into(),
            ));
        }
        let tail = tail.clamp(1, 5000);
        let docker = self.connect().await?;
        let opts = LogsOptionsBuilder::default()
            .follow(false)
            .stdout(true)
            .stderr(true)
            .timestamps(true)
            .tail(&tail.to_string())
            .build();
        let mut stream = docker.logs(id, Some(opts));
        let mut lines = Vec::new();
        while let Some(item) = stream.next().await {
            match item {
                Ok(chunk) => {
                    lines.extend(parse::parse_log_chunk(chunk));
                    if lines.len() >= 5000 {
                        lines.truncate(5000);
                        break;
                    }
                }
                Err(e) => return Err(DockerError::from_bollard(e, "container logs")),
            }
        }
        Ok(lines)
    }

    /// Follow container logs live as a stream.
    pub async fn logs_follow(
        &self,
        id: &str,
        tail: u32,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<DockerLogLine, DockerError>> + Send>>, DockerError>
    {
        if !valid_docker_ref(id) {
            return Err(DockerError::Invalid(
                "That is not a valid container name or ID.".into(),
            ));
        }
        let tail = tail.clamp(1, 2000);
        let docker = self.connect().await?;
        // Verify container exists before upgrading to a long-lived stream.
        docker
            .inspect_container(id, None::<InspectContainerOptions>)
            .await
            .map_err(|e| DockerError::from_bollard(e, "container"))?;

        let opts = LogsOptionsBuilder::default()
            .follow(true)
            .stdout(true)
            .stderr(true)
            .timestamps(true)
            .tail(&tail.to_string())
            .build();
        let raw_stream = docker.logs(id, Some(opts));
        let mapped = raw_stream.flat_map(|item| {
            let items: Vec<Result<DockerLogLine, DockerError>> = match item {
                Ok(chunk) => parse::parse_log_chunk(chunk).into_iter().map(Ok).collect(),
                Err(e) => vec![Err(DockerError::from_bollard(e, "container logs"))],
            };
            futures_util::stream::iter(items)
        });
        Ok(Box::pin(mapped))
    }

    /// Sample CPU, memory, network, and PIDs for all running containers concurrently.
    pub async fn stats(&self) -> Result<Vec<DockerContainerStat>, DockerError> {
        let docker = self.connect().await?;
        let opts = ListContainersOptionsBuilder::default().all(false).build();
        let running = docker
            .list_containers(Some(opts))
            .await
            .map_err(|e| DockerError::from_bollard(e, "list running containers"))?;

        if running.is_empty() {
            self.prev_cpu
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear();
            return Ok(Vec::new());
        }

        let stat_opts = StatsOptionsBuilder::default()
            .stream(false)
            .one_shot(true)
            .build();

        let futures = running.into_iter().map(|c| {
            let docker = docker.clone();
            let stat_opts = stat_opts.clone();
            let full_id = c.id.unwrap_or_default();
            let id = parse::short_id(&full_id);
            let name = parse::clean_name(c.names.as_deref(), &full_id);
            let image = c.image.unwrap_or_default();
            let state = c.state.map_or_else(|| "running".into(), |s| s.to_string());
            let status = c.status.unwrap_or_default();
            let labels = c.labels.unwrap_or_default();
            let (managed, service, _) = parse::opsbox_labels(&labels);
            async move {
                let mut stream = docker.stats(&full_id, Some(stat_opts));
                let sample = match tokio::time::timeout(Duration::from_secs(2), stream.next()).await
                {
                    Ok(Some(Ok(s))) => Some(s),
                    _ => None,
                };
                (
                    id, full_id, name, image, state, status, managed, service, sample,
                )
            }
        });

        let results = join_all(futures).await;
        let mut prev = self
            .prev_cpu
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut active_ids = HashSet::with_capacity(results.len());
        let mut out = Vec::with_capacity(results.len());

        for (id, full_id, name, image, state, status, managed, service, sample) in results {
            active_ids.insert(full_id.clone());
            let Some(s) = sample else {
                continue;
            };
            let (cpu_total_ns, system_cpu_ns, online_cpus) =
                parse::extract_cpu_counters(s.cpu_stats.as_ref());
            let (pre_total, pre_sys, _) = parse::extract_cpu_counters(s.precpu_stats.as_ref());
            let cpu_pct = if pre_sys > 0 {
                parse::compute_cpu_pct(cpu_total_ns, system_cpu_ns, pre_total, pre_sys, online_cpus)
            } else if let Some((old_cpu, old_sys)) = prev.get(&full_id).copied() {
                parse::compute_cpu_pct(cpu_total_ns, system_cpu_ns, old_cpu, old_sys, online_cpus)
            } else {
                0.0
            };
            prev.insert(full_id.clone(), (cpu_total_ns, system_cpu_ns));

            let (mem_used_bytes, mem_limit_bytes) =
                parse::compute_mem_used(s.memory_stats.as_ref());
            let (net_rx_bytes, net_tx_bytes) =
                s.networks
                    .unwrap_or_default()
                    .values()
                    .fold((0u64, 0u64), |(rx, tx), iface| {
                        (
                            rx.saturating_add(iface.rx_bytes.unwrap_or(0)),
                            tx.saturating_add(iface.tx_bytes.unwrap_or(0)),
                        )
                    });
            let pids = s.pids_stats.and_then(|p| p.current).unwrap_or(0);

            out.push(DockerContainerStat {
                id,
                full_id,
                name,
                image,
                state,
                status,
                cpu_pct,
                cpu_total_ns,
                system_cpu_ns,
                online_cpus,
                mem_used_bytes,
                mem_limit_bytes,
                net_rx_bytes,
                net_tx_bytes,
                pids,
                managed,
                service,
            });
        }
        prev.retain(|k, _| active_ids.contains(k));
        out.sort_by(|a, b| {
            b.cpu_pct
                .total_cmp(&a.cpu_pct)
                .then(b.mem_used_bytes.cmp(&a.mem_used_bytes))
        });
        Ok(out)
    }

    /// Execute a command inside a running container (used by templates and database dumps).
    pub async fn exec(&self, req: DockerExecRequest) -> Result<DockerExecOutput, DockerError> {
        if !valid_docker_ref(&req.id) {
            return Err(DockerError::Invalid(
                "That is not a valid container name or ID.".into(),
            ));
        }
        if req.cmd.is_empty() || req.cmd.len() > 64 || req.cmd.iter().any(|s| s.contains('\0')) {
            return Err(DockerError::Invalid(
                "Invalid command arguments for container exec.".into(),
            ));
        }
        if req.env.iter().any(|e| !e.contains('=') || e.contains('\0')) {
            return Err(DockerError::Invalid(
                "Invalid environment variables for container exec.".into(),
            ));
        }
        if req
            .user
            .as_deref()
            .is_some_and(|u| u.contains('\0') || u.len() > 64)
            || req
                .working_dir
                .as_deref()
                .is_some_and(|w| w.contains('\0') || w.len() > 512)
        {
            return Err(DockerError::Invalid(
                "Invalid user or working directory for container exec.".into(),
            ));
        }

        let docker = self.connect().await?;
        let has_stdin = req.stdin.is_some();
        let created = docker
            .create_exec(
                &req.id,
                CreateExecOptions {
                    attach_stdin: Some(has_stdin),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    tty: Some(false),
                    cmd: Some(req.cmd),
                    env: (!req.env.is_empty()).then_some(req.env),
                    user: req.user,
                    working_dir: req.working_dir,
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| DockerError::from_bollard(e, "create exec"))?;

        let start = docker
            .start_exec(
                &created.id,
                Some(StartExecOptions {
                    detach: false,
                    tty: false,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|e| DockerError::from_bollard(e, "start exec"))?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut truncated = false;

        if let StartExecResults::Attached {
            mut output,
            mut input,
        } = start
        {
            if let Some(bytes) = req.stdin {
                let _ = input.write_all(&bytes).await;
                let _ = input.shutdown().await;
            }
            let read_all = async {
                while let Some(item) = output.next().await {
                    match item {
                        Ok(LogOutput::StdOut { message } | LogOutput::Console { message }) => {
                            let keep = message
                                .len()
                                .min(MAX_EXEC_STDOUT.saturating_sub(stdout.len()));
                            stdout.extend_from_slice(&message[..keep]);
                            truncated |= keep < message.len();
                        }
                        Ok(LogOutput::StdErr { message }) => {
                            let keep = message
                                .len()
                                .min(MAX_EXEC_STDERR.saturating_sub(stderr.len()));
                            stderr.extend_from_slice(&message[..keep]);
                            truncated |= keep < message.len();
                        }
                        Ok(LogOutput::StdIn { .. }) => {}
                        Err(e) => return Err(DockerError::from_bollard(e, "exec output")),
                    }
                }
                Ok::<(), DockerError>(())
            };
            tokio::time::timeout(EXEC_TIMEOUT, read_all)
                .await
                .map_err(|_| DockerError::Timeout)??;
        }

        let inspected = docker
            .inspect_exec(&created.id)
            .await
            .map_err(|e| DockerError::from_bollard(e, "inspect exec"))?;

        Ok(DockerExecOutput {
            exit_code: inspected.exit_code.unwrap_or(0),
            stdout,
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
            truncated,
        })
    }

    /// List images with tag and usage status.
    pub async fn list_images(&self) -> Result<Vec<DockerImage>, DockerError> {
        let docker = self.connect().await?;
        let opts = ListImagesOptionsBuilder::default().all(false).build();
        let imgs = docker
            .list_images(Some(opts))
            .await
            .map_err(|e| DockerError::from_bollard(e, "list images"))?;
        let containers = self.list_containers().await.unwrap_or_default();
        let used_refs: HashSet<String> =
            containers.iter().flat_map(|c| [c.image.clone()]).collect();
        // Also inspect image IDs from raw container list if needed:
        let raw_containers = docker
            .list_containers(Some(
                ListContainersOptionsBuilder::default().all(true).build(),
            ))
            .await
            .unwrap_or_default();
        let used_ids: HashSet<String> = raw_containers
            .into_iter()
            .filter_map(|c| c.image_id)
            .collect();

        let mut out: Vec<DockerImage> = imgs
            .into_iter()
            .map(|img| {
                let (name, tag) = parse::split_repo_tag(&img.repo_tags);
                let full_ref = format!("{name}:{tag}");
                let containers = u64::try_from(img.containers).unwrap_or(0);
                let used = containers > 0
                    || used_ids.contains(&img.id)
                    || used_refs.contains(&full_ref)
                    || used_refs.contains(&name);
                DockerImage {
                    id: parse::short_id(&img.id),
                    full_id: img.id,
                    name,
                    tag,
                    size_bytes: u64::try_from(img.size).unwrap_or(0),
                    created: img.created,
                    used,
                    containers,
                }
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name).then(a.tag.cmp(&b.tag)));
        Ok(out)
    }

    /// List volumes with attached status.
    pub async fn list_volumes(&self) -> Result<Vec<DockerVolume>, DockerError> {
        let docker = self.connect().await?;
        let all = docker
            .list_volumes(None::<ListVolumesOptions>)
            .await
            .map_err(|e| DockerError::from_bollard(e, "list volumes"))?;
        let mut dangling_filters = HashMap::new();
        dangling_filters.insert("dangling", vec!["true"]);
        let dangling_resp = docker
            .list_volumes(Some(
                ListVolumesOptionsBuilder::default()
                    .filters(&dangling_filters)
                    .build(),
            ))
            .await
            .ok();
        let dangling_names: HashSet<String> = dangling_resp
            .and_then(|r| r.volumes)
            .unwrap_or_default()
            .into_iter()
            .map(|v| v.name)
            .collect();

        let mut out: Vec<DockerVolume> = all
            .volumes
            .unwrap_or_default()
            .into_iter()
            .map(|v| {
                let size_bytes = v
                    .usage_data
                    .as_ref()
                    .and_then(|u| u64::try_from(u.size).ok());
                let used = !dangling_names.contains(&v.name);
                DockerVolume {
                    name: v.name,
                    driver: v.driver,
                    mountpoint: v.mountpoint,
                    size_bytes,
                    used,
                    created_at: v.created_at.map(|t| t.to_string()),
                    labels: v.labels.into_iter().collect(),
                }
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// List networks with attached container counts.
    pub async fn list_networks(&self) -> Result<Vec<DockerNetwork>, DockerError> {
        let docker = self.connect().await?;
        let nets = docker
            .list_networks(Some(query_parameters_networks()))
            .await
            .map_err(|e| DockerError::from_bollard(e, "list networks"))?;
        let containers = docker
            .list_containers(Some(
                ListContainersOptionsBuilder::default().all(true).build(),
            ))
            .await
            .unwrap_or_default();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for c in containers {
            if let Some(map) = c.network_settings.and_then(|n| n.networks) {
                for net_name in map.keys() {
                    *counts.entry(net_name.clone()).or_insert(0) += 1;
                }
            }
        }

        let mut out: Vec<DockerNetwork> = nets
            .into_iter()
            .map(|n| {
                let full_id = n.id.unwrap_or_default();
                let name = n.name.unwrap_or_default();
                let containers = counts.get(&name).copied().unwrap_or(0);
                DockerNetwork {
                    id: parse::short_id(&full_id),
                    full_id,
                    name,
                    driver: n.driver.unwrap_or_default(),
                    scope: n.scope.unwrap_or_else(|| "local".into()),
                    internal: n.internal.unwrap_or(false),
                    containers,
                }
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Query `/system/df` over the Unix socket so both pre-1.52 and 1.52+ daemons decode accurately.
    async fn system_df_summary(&self) -> Result<parse::DfSummary, DockerError> {
        let ver = {
            let docker = self.connect().await?;
            docker.client_version()
        };
        let path = format!("/v{}.{}/system/df", ver.major_version, ver.minor_version);
        let raw = self.raw_get_json(&path).await?;
        Ok(parse::parse_df_json(&raw))
    }

    async fn raw_get_json(&self, path: &str) -> Result<serde_json::Value, DockerError> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|_| DockerError::DaemonUnavailable("Docker daemon is not running.".into()))?;
        let req = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        stream
            .write_all(req.as_bytes())
            .await
            .map_err(|e| DockerError::Internal(format!("Docker write failed: {e}")))?;
        let mut buf = Vec::new();
        tokio::time::timeout(
            Duration::from_secs(self.timeout_secs.min(120)),
            AsyncReadExt::take(&mut stream, MAX_DF_BYTES as u64).read_to_end(&mut buf),
        )
        .await
        .map_err(|_| DockerError::Timeout)?
        .map_err(|e| DockerError::Internal(format!("Docker read failed: {e}")))?;

        let split = buf
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or_else(|| DockerError::Internal("Malformed HTTP response from Docker.".into()))?;
        let head = String::from_utf8_lossy(&buf[..split]);
        let body = &buf[split + 4..];
        let is_chunked = head.lines().any(|l| {
            l.to_ascii_lowercase()
                .contains("transfer-encoding: chunked")
        });
        let decoded = if is_chunked {
            decode_chunked(body).ok_or_else(|| {
                DockerError::Internal("Malformed chunked HTTP body from Docker.".into())
            })?
        } else {
            body.to_vec()
        };
        serde_json::from_slice(&decoded)
            .map_err(|e| DockerError::Internal(format!("Invalid Docker JSON response: {e}")))
    }

    /// Fast cleanup scan (dangling images, stopped containers, build cache, dangling volume count).
    /// Volume sizes are deferred to [`Self::volume_sizes`] on demand.
    pub async fn scan_cleanup(&self) -> Result<DockerCleanupScan, DockerError> {
        let docker = self.connect().await?;

        let mut img_filters = HashMap::new();
        img_filters.insert("dangling", vec!["true"]);
        let dangling_imgs = docker
            .list_images(Some(
                ListImagesOptionsBuilder::default()
                    .filters(&img_filters)
                    .build(),
            ))
            .await
            .unwrap_or_default();
        let dangling_images_count = dangling_imgs.len();
        let dangling_images_bytes: u64 = dangling_imgs
            .iter()
            .filter_map(|i| u64::try_from(i.size).ok())
            .sum();

        let containers = docker
            .list_containers(Some(
                ListContainersOptionsBuilder::default()
                    .all(true)
                    .size(true)
                    .build(),
            ))
            .await
            .unwrap_or_default();
        let mut stopped_containers_count = 0usize;
        let mut stopped_containers_bytes = 0u64;
        for c in containers {
            let state = c.state.map_or_else(String::new, |s| s.to_string());
            if matches!(state.as_str(), "exited" | "created" | "dead") {
                stopped_containers_count += 1;
                if let Some(sz) = c.size_rw.and_then(|s| u64::try_from(s).ok()) {
                    stopped_containers_bytes = stopped_containers_bytes.saturating_add(sz);
                }
            }
        }

        let mut vol_filters = HashMap::new();
        vol_filters.insert("dangling", vec!["true"]);
        let dangling_volumes_count = docker
            .list_volumes(Some(
                ListVolumesOptionsBuilder::default()
                    .filters(&vol_filters)
                    .build(),
            ))
            .await
            .ok()
            .and_then(|v| v.volumes)
            .map_or(0, |v| v.len());

        let df = self.system_df_summary().await.unwrap_or_default();

        Ok(DockerCleanupScan {
            dangling_images_bytes,
            dangling_images_count,
            build_cache_bytes: df.build_cache_reclaimable,
            stopped_containers_bytes,
            stopped_containers_count,
            dangling_volumes_count,
        })
    }

    /// Compute reclaimable volume bytes and total volume count on demand.
    pub async fn volume_sizes(&self) -> Result<(u64, usize), DockerError> {
        let df = self.system_df_summary().await?;
        Ok((df.volumes_reclaimable, df.volumes_total_count))
    }

    /// Prune unused Docker resources and return bytes reclaimed.
    pub async fn prune(&self, kind: DockerPruneKind) -> Result<u64, DockerError> {
        let docker = self.connect().await?;
        let bytes = match kind {
            DockerPruneKind::Images => {
                let mut filters = HashMap::new();
                filters.insert("dangling", vec!["true"]);
                let res = docker
                    .prune_images(Some(
                        PruneImagesOptionsBuilder::default()
                            .filters(&filters)
                            .build(),
                    ))
                    .await
                    .map_err(|e| DockerError::from_bollard(e, "prune images"))?;
                res.space_reclaimed
                    .and_then(|n| u64::try_from(n).ok())
                    .unwrap_or(0)
            }
            DockerPruneKind::BuildCache => {
                let res = docker
                    .prune_build(Some(PruneBuildOptionsBuilder::default().build()))
                    .await
                    .map_err(|e| DockerError::from_bollard(e, "prune build cache"))?;
                res.space_reclaimed
                    .and_then(|n| u64::try_from(n).ok())
                    .unwrap_or(0)
            }
            DockerPruneKind::Containers => {
                let res = docker
                    .prune_containers(Some(PruneContainersOptionsBuilder::default().build()))
                    .await
                    .map_err(|e| DockerError::from_bollard(e, "prune containers"))?;
                res.space_reclaimed
                    .and_then(|n| u64::try_from(n).ok())
                    .unwrap_or(0)
            }
            DockerPruneKind::Volumes => {
                let res = docker
                    .prune_volumes(Some(PruneVolumesOptionsBuilder::default().build()))
                    .await
                    .map_err(|e| DockerError::from_bollard(e, "prune volumes"))?;
                res.space_reclaimed
                    .and_then(|n| u64::try_from(n).ok())
                    .unwrap_or(0)
            }
        };
        Ok(bytes)
    }

    /// Return names of containers currently in Docker's `restarting` state.
    pub async fn restarting_containers(&self) -> Vec<String> {
        if !self.socket_path.exists() {
            return Vec::new();
        }
        let Ok(docker) = self.connect().await else {
            return Vec::new();
        };
        let mut filters = HashMap::new();
        filters.insert("status", vec!["restarting"]);
        let Ok(list) = docker
            .list_containers(Some(
                ListContainersOptionsBuilder::default()
                    .all(true)
                    .filters(&filters)
                    .build(),
            ))
            .await
        else {
            return Vec::new();
        };
        list.into_iter()
            .filter(|c| {
                c.state
                    .as_ref()
                    .is_some_and(|s| s.to_string() == "restarting")
            })
            .map(|c| {
                let full_id = c.id.unwrap_or_default();
                parse::clean_name(c.names.as_deref(), &full_id)
            })
            .collect()
    }
}

fn query_parameters_networks() -> bollard::query_parameters::ListNetworksOptions {
    ListNetworksOptionsBuilder::default().build()
}

fn decode_chunked(mut input: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let crlf = input.windows(2).position(|w| w == b"\r\n")?;
        let size_line = std::str::from_utf8(&input[..crlf]).ok()?.trim();
        let hex_part = size_line.split(';').next()?.trim();
        let size = usize::from_str_radix(hex_part, 16).ok()?;
        input = &input[crlf + 2..];
        if size == 0 {
            return Some(out);
        }
        if input.len() < size + 2 {
            return None;
        }
        out.extend_from_slice(&input[..size]);
        input = &input[size + 2..];
    }
}

/// Format container log lines as `LogStream` helper for tests.
pub fn stream_name(s: DockerLogStream) -> &'static str {
    match s {
        DockerLogStream::Stdout => "stdout",
        DockerLogStream::Stderr => "stderr",
        DockerLogStream::Console => "console",
    }
}
