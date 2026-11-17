//! Privileged Docker operations and repository-backed installation
//! (docs/BACKEND_SPEC.md 7.4).
//!
//! Only `priv` talks to `/var/run/docker.sock` (via `opsbox-docker` / `bollard`).
//! Docker installation uses distro packages or Docker's official signed apt/dnf
//! repository with GPG key fingerprint verification; never the convenience script.

use std::future::Future;
use std::io::Read as _;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use futures_util::StreamExt;
use opsbox_docker::{DockerClient, DockerError, is_docker_installed};
use opsbox_proto::frame;
use opsbox_proto::{
    CommandOutcome, DockerContainer, DockerContainerAction, DockerContainerInspect,
    DockerContainerStat, DockerExecOutput, DockerExecRequest, DockerImage, DockerLogFrame,
    DockerLogLine, DockerNetwork, DockerStatus, DockerVolume, PrivError, PrivErrorCode, PrivReply,
    PrivRequest, PrivResponse, PrivResult, valid_docker_ref,
};
use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;
use tracing::{info, warn};

use super::exec::{find, run, run_with, tail};
use crate::io::write_frame;

const DOCKER_APT_GPG_FPR: &str = "9DC858229FC7DD38854AE2D88D81803C0EBFCD88";
const DOCKER_RPM_GPG_FPR: &str = "060A61C51B558A7F742B77AAC52FEB6B621E9F35";
const MAX_KEY_BYTES: usize = 64 * 1024;
const INSTALL_TIMEOUT: Duration = Duration::from_secs(20 * 60);

fn err(code: PrivErrorCode, message: impl Into<String>) -> PrivError {
    PrivError {
        code,
        message: message.into(),
    }
}

pub(crate) fn client() -> &'static DockerClient {
    static CLIENT: OnceLock<DockerClient> = OnceLock::new();
    CLIENT.get_or_init(DockerClient::default)
}

fn map_docker_err(e: DockerError, socket_path: &Path) -> PrivError {
    match e {
        DockerError::DaemonUnavailable(_) => {
            if is_docker_installed(socket_path) {
                err(
                    PrivErrorCode::Internal,
                    "Docker is installed, but the Docker daemon is not running. Start docker.service and try again.",
                )
            } else {
                err(
                    PrivErrorCode::Unsupported,
                    "Docker is not installed on this server.",
                )
            }
        }
        DockerError::NotFound(m) => err(PrivErrorCode::NotFound, m),
        DockerError::Conflict(m) => err(PrivErrorCode::Conflict, m),
        DockerError::Invalid(m) => err(PrivErrorCode::Invalid, m),
        DockerError::Timeout => err(PrivErrorCode::Internal, "Docker did not answer in time."),
        DockerError::Internal(m) => err(PrivErrorCode::Internal, m),
    }
}

pub(crate) fn block_on_fut<T>(work: impl Future<Output = T>) -> Result<T, PrivError> {
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        return Ok(rt.block_on(work));
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Helper runtime failed: {e}"),
            )
        })?;
    Ok(rt.block_on(work))
}

fn run_docker<T>(
    socket_path: &Path,
    work: impl Future<Output = Result<T, DockerError>>,
) -> Result<T, PrivError> {
    block_on_fut(work)?.map_err(|e| map_docker_err(e, socket_path))
}

// ------------------------------------------------------------------ operations

pub fn status_with(dc: &DockerClient) -> DockerStatus {
    let installed = is_docker_installed(dc.socket_path());
    let (can_install, install_method) = install_capability();
    if !dc.socket_path().exists() {
        return DockerStatus {
            installed,
            running: false,
            version: None,
            api_version: None,
            can_install,
            install_method,
            error: installed
                .then(|| "Docker is installed, but the Docker daemon is not running.".to_owned()),
        };
    }

    match block_on_fut(dc.ping_version()) {
        Ok(Ok((ver, api))) => DockerStatus {
            installed: true,
            running: true,
            version: Some(ver),
            api_version: Some(api),
            can_install,
            install_method,
            error: None,
        },
        Ok(Err(e)) => DockerStatus {
            installed,
            running: false,
            version: None,
            api_version: None,
            can_install,
            install_method,
            error: Some(e.to_string()),
        },
        Err(e) => DockerStatus {
            installed,
            running: false,
            version: None,
            api_version: None,
            can_install,
            install_method,
            error: Some(e.message),
        },
    }
}

pub fn list_containers_with(dc: &DockerClient) -> Result<Vec<DockerContainer>, PrivError> {
    run_docker(dc.socket_path(), dc.list_containers())
}

pub fn inspect_with(dc: &DockerClient, id: &str) -> Result<DockerContainerInspect, PrivError> {
    if !valid_docker_ref(id) {
        return Err(err(
            PrivErrorCode::Invalid,
            "That is not a valid container name or ID.",
        ));
    }
    run_docker(dc.socket_path(), dc.inspect_container(id))
}

pub fn action_with(
    dc: &DockerClient,
    id: &str,
    kind: DockerContainerAction,
) -> Result<(), PrivError> {
    if !valid_docker_ref(id) {
        return Err(err(
            PrivErrorCode::Invalid,
            "That is not a valid container name or ID.",
        ));
    }
    run_docker(dc.socket_path(), dc.container_action(id, kind))
}

pub fn logs_with(
    dc: &DockerClient,
    id: &str,
    tail_lines: u32,
) -> Result<Vec<DockerLogLine>, PrivError> {
    if !valid_docker_ref(id) {
        return Err(err(
            PrivErrorCode::Invalid,
            "That is not a valid container name or ID.",
        ));
    }
    run_docker(dc.socket_path(), dc.logs(id, tail_lines))
}

pub fn stats_with(dc: &DockerClient) -> Result<Vec<DockerContainerStat>, PrivError> {
    run_docker(dc.socket_path(), dc.stats())
}

pub fn exec_with(
    dc: &DockerClient,
    req: &DockerExecRequest,
) -> Result<DockerExecOutput, PrivError> {
    validate_exec_request(req)?;
    run_docker(dc.socket_path(), dc.exec(req.clone()))
}

fn validate_exec_request(req: &DockerExecRequest) -> Result<(), PrivError> {
    if !valid_docker_ref(&req.id) {
        return Err(err(
            PrivErrorCode::Invalid,
            "That is not a valid container name or ID.",
        ));
    }
    if req.cmd.is_empty() || req.cmd.len() > 64 || req.cmd.iter().any(|s| s.contains('\0')) {
        return Err(err(
            PrivErrorCode::Invalid,
            "Command arguments must be non-empty and contain no NUL bytes.",
        ));
    }
    if req.env.iter().any(|e| !e.contains('=') || e.contains('\0')) {
        return Err(err(
            PrivErrorCode::Invalid,
            "Environment entries must be KEY=VALUE without NUL bytes.",
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
        return Err(err(
            PrivErrorCode::Invalid,
            "Invalid user or working directory.",
        ));
    }
    Ok(())
}

pub fn list_images_with(dc: &DockerClient) -> Result<Vec<DockerImage>, PrivError> {
    run_docker(dc.socket_path(), dc.list_images())
}

pub fn list_volumes_with(dc: &DockerClient) -> Result<Vec<DockerVolume>, PrivError> {
    run_docker(dc.socket_path(), dc.list_volumes())
}

pub fn list_networks_with(dc: &DockerClient) -> Result<Vec<DockerNetwork>, PrivError> {
    run_docker(dc.socket_path(), dc.list_networks())
}

/// Fast non-blocking check used by `SystemFacts`.
pub fn restarting_containers_with(dc: &DockerClient) -> Vec<String> {
    if !dc.socket_path().exists() {
        return Vec::new();
    }
    block_on_fut(dc.restarting_containers()).unwrap_or_default()
}

/// Handle a `PrivOp::DockerLogsFollow` connection by upgrading the Unix socket to stream
/// length-prefixed [`DockerLogFrame`] JSON messages until the container ends or peer closes.
pub(crate) async fn follow_logs_with(
    dc: &DockerClient,
    stream: &mut UnixStream,
    request: &PrivRequest,
    id: &str,
    tail_lines: u32,
) {
    if !valid_docker_ref(id) {
        let resp = PrivResponse::failure(
            request.id.clone(),
            PrivErrorCode::Invalid,
            "That is not a valid container name or ID.",
        );
        let _ = write_frame(stream, &resp).await;
        return;
    }

    let mut log_stream = match dc.logs_follow(id, tail_lines).await {
        Ok(s) => s,
        Err(e) => {
            let pe = map_docker_err(e, dc.socket_path());
            let resp = PrivResponse {
                id: request.id.clone(),
                result: PrivResult::Failure { error: pe },
            };
            let _ = write_frame(stream, &resp).await;
            return;
        }
    };

    let started = PrivResponse::success(
        request.id.clone(),
        PrivReply::DockerLogStreamStarted { id: id.to_owned() },
    );
    if write_frame(stream, &started).await.is_err() {
        return;
    }

    let mut sink = [0u8; 256];
    loop {
        tokio::select! {
            item = log_stream.next() => match item {
                Some(Ok(line)) => {
                    if write_log_frame(stream, &DockerLogFrame::Line(line)).await.is_err() {
                        return;
                    }
                }
                Some(Err(e)) => {
                    let msg = map_docker_err(e, dc.socket_path()).message;
                    let _ = write_log_frame(stream, &DockerLogFrame::Error { message: msg }).await;
                    return;
                }
                None => {
                    let _ = write_log_frame(stream, &DockerLogFrame::End).await;
                    return;
                }
            },
            read = stream.read(&mut sink) => match read {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
        }
    }
}

async fn write_log_frame(stream: &mut UnixStream, msg: &DockerLogFrame) -> Result<(), ()> {
    use tokio::io::AsyncWriteExt;
    let bytes = frame::encode(msg).map_err(|_| ())?;
    stream.write_all(&bytes).await.map_err(|_| ())?;
    stream.flush().await.map_err(|_| ())
}

// ------------------------------------------------------------------ installation

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallPlan {
    DockerSignedApt {
        repo_distro: String,
        codename: String,
    },
    DistroApt,
    DockerSignedDnf {
        repo_distro: String,
    },
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct OsRelease {
    pub id: String,
    pub id_like: Vec<String>,
    pub version_codename: String,
    pub ubuntu_codename: String,
    pub debian_codename: String,
}

pub fn parse_os_release(text: &str) -> OsRelease {
    let mut out = OsRelease::default();
    for line in text.lines() {
        let Some((k, raw_v)) = line.trim().split_once('=') else {
            continue;
        };
        let v = raw_v
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_ascii_lowercase();
        match k.trim() {
            "ID" => out.id = v,
            "ID_LIKE" => out.id_like = v.split_whitespace().map(str::to_owned).collect(),
            "VERSION_CODENAME" => out.version_codename = v,
            "UBUNTU_CODENAME" => out.ubuntu_codename = v,
            "DEBIAN_CODENAME" => out.debian_codename = v,
            _ => {}
        }
    }
    out
}

fn valid_codename(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub fn plan_from_os(os: &OsRelease, has_apt: bool, has_dnf: bool) -> Option<InstallPlan> {
    if has_apt {
        let (distro, code) = if os.id == "ubuntu" {
            ("ubuntu", &os.version_codename)
        } else if os.id == "debian" {
            ("debian", &os.version_codename)
        } else if !os.ubuntu_codename.is_empty() && os.id_like.iter().any(|x| x == "ubuntu") {
            ("ubuntu", &os.ubuntu_codename)
        } else if !os.debian_codename.is_empty() && os.id_like.iter().any(|x| x == "debian") {
            ("debian", &os.debian_codename)
        } else {
            return Some(InstallPlan::DistroApt);
        };
        if valid_codename(code) {
            return Some(InstallPlan::DockerSignedApt {
                repo_distro: distro.to_owned(),
                codename: code.clone(),
            });
        }
        return Some(InstallPlan::DistroApt);
    }
    if has_dnf {
        let repo = match os.id.as_str() {
            "fedora" => "fedora",
            "rhel" => "rhel",
            "centos" | "rocky" | "almalinux" => "centos",
            _ if os.id_like.iter().any(|x| x == "fedora") => "fedora",
            _ if os.id_like.iter().any(|x| x == "rhel" || x == "centos") => "centos",
            _ => return None,
        };
        return Some(InstallPlan::DockerSignedDnf {
            repo_distro: repo.to_owned(),
        });
    }
    None
}

fn install_capability() -> (bool, Option<String>) {
    let has_apt = Path::new("/usr/bin/apt-get").exists();
    let has_dnf = Path::new("/usr/bin/dnf").exists();
    let os = std::fs::read_to_string("/etc/os-release")
        .map(|s| parse_os_release(&s))
        .unwrap_or_default();
    match plan_from_os(&os, has_apt, has_dnf) {
        Some(InstallPlan::DockerSignedApt { .. }) => {
            (true, Some("Docker's signed apt repository".into()))
        }
        Some(InstallPlan::DistroApt) => (true, Some("distribution apt packages".into())),
        Some(InstallPlan::DockerSignedDnf { .. }) => {
            (true, Some("Docker's signed dnf repository".into()))
        }
        None => (false, None),
    }
}

pub fn apt_sources_line(repo_distro: &str, codename: &str, arch: &str) -> String {
    format!(
        "deb [arch={arch} signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/{repo_distro} {codename} stable\n"
    )
}

/// Parse `gpg --show-keys --with-colons` output and return all `fpr` fingerprints.
pub fn parse_gpg_fingerprints(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(':');
            if fields.next()? != "fpr" {
                return None;
            }
            fields
                .nth(8)
                .map(|f| f.trim().to_ascii_uppercase())
                .filter(|f| !f.is_empty())
        })
        .collect()
}

pub fn validate_rpm_repo_text(text: &str, repo_distro: &str) -> bool {
    let expected_gpg = format!("https://download.docker.com/linux/{repo_distro}/gpg");
    text.contains("[docker-ce-stable]")
        && text.contains("gpgcheck=1")
        && text.contains(&expected_gpg)
        && text.contains(&format!("https://download.docker.com/linux/{repo_distro}/"))
}

fn verify_pgp_key(bytes: &[u8], expected_fpr: &str) -> Result<(), PrivError> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        err(
            PrivErrorCode::Internal,
            "Downloaded Docker GPG key is not valid ASCII-armored text.",
        )
    })?;
    if !text.contains("-----BEGIN PGP PUBLIC KEY BLOCK-----")
        || !text.contains("-----END PGP PUBLIC KEY BLOCK-----")
    {
        return Err(err(
            PrivErrorCode::Internal,
            "Downloaded Docker GPG key did not match expected PGP armor format.",
        ));
    }
    if let Some(gpg) = find(&["/usr/bin/gpg", "/bin/gpg"]) {
        let out = run_with(
            &gpg,
            &["--show-keys", "--with-colons"],
            Duration::from_secs(15),
            Some(text),
            &[],
        )
        .ok_or_else(|| err(PrivErrorCode::Internal, "Could not inspect Docker GPG key."))?;
        if !out.ok() {
            return Err(err(
                PrivErrorCode::Internal,
                "Docker GPG key inspection failed.",
            ));
        }
        let fprs = parse_gpg_fingerprints(&out.stdout);
        if !fprs.iter().any(|f| f == expected_fpr) {
            return Err(err(
                PrivErrorCode::Internal,
                format!(
                    "Docker GPG key fingerprint mismatch (expected {expected_fpr}, got {}). Refusing to install.",
                    fprs.join(", ")
                ),
            ));
        }
    }
    Ok(())
}

fn fetch_https_text(url: &str) -> Result<Vec<u8>, PrivError> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .new_agent();
    let resp = agent.get(url).call().map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Could not download {url}: {e}"),
        )
    })?;
    let mut reader = resp
        .into_body()
        .into_reader()
        .take((MAX_KEY_BYTES + 1) as u64);
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Could not read response from {url}: {e}"),
        )
    })?;
    if buf.len() > MAX_KEY_BYTES {
        return Err(err(
            PrivErrorCode::Internal,
            format!("Response from {url} exceeded size limit."),
        ));
    }
    Ok(buf)
}

fn write_atomic_bytes(path: &Path, bytes: &[u8], mode: u32) -> Result<(), PrivError> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Cannot create {}: {e}", parent.display()),
            )
        })?;
    }
    let tmp = path.with_extension("opsbox-tmp");
    std::fs::write(&tmp, bytes).map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Cannot write {}: {e}", path.display()),
        )
    })?;
    let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode));
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        err(
            PrivErrorCode::Internal,
            format!("Cannot install {}: {e}", path.display()),
        )
    })
}

fn dpkg_arch() -> String {
    if let Some(dpkg) = find(&["/usr/bin/dpkg", "/bin/dpkg"])
        && let Some(out) = run(&dpkg, &["--print-architecture"], Duration::from_secs(5))
        && out.ok()
    {
        let a = out.stdout.trim();
        if matches!(a, "amd64" | "arm64" | "armhf" | "s390x" | "ppc64el") {
            return a.to_owned();
        }
    }
    match std::env::consts::ARCH {
        "x86_64" => "amd64".into(),
        "aarch64" => "arm64".into(),
        other => other.to_owned(),
    }
}

pub(super) fn install() -> Result<CommandOutcome, PrivError> {
    let dc = client();
    if let Ok(rt) = tokio::runtime::Handle::try_current()
        && rt.block_on(dc.ping_version()).is_ok()
    {
        return Ok(CommandOutcome {
            ok: true,
            tail: vec!["Docker is already installed and running.".into()],
            count: 0,
        });
    }

    let has_apt = Path::new("/usr/bin/apt-get").exists();
    let has_dnf = Path::new("/usr/bin/dnf").exists();
    let os = std::fs::read_to_string("/etc/os-release")
        .map(|s| parse_os_release(&s))
        .unwrap_or_default();
    let plan = plan_from_os(&os, has_apt, has_dnf).ok_or_else(|| {
        err(
            PrivErrorCode::Unsupported,
            "Automatic Docker installation requires apt (Ubuntu/Debian) or dnf (Fedora/RHEL/Rocky/AlmaLinux).",
        )
    })?;

    info!(?plan, "starting Docker installation");
    let mut log_tail: Vec<String> = Vec::new();
    let apt_env = [
        ("DEBIAN_FRONTEND", "noninteractive"),
        ("NEEDRESTART_MODE", "a"),
    ];

    match plan {
        InstallPlan::DockerSignedApt {
            repo_distro,
            codename,
        } => {
            let upd0 = run_with(
                "/usr/bin/apt-get",
                &["-q", "update"],
                Duration::from_secs(300),
                None,
                &apt_env,
            )
            .ok_or_else(|| err(PrivErrorCode::Internal, "apt-get did not start."))?;
            if !upd0.ok() {
                return Err(err(
                    PrivErrorCode::Internal,
                    format!(
                        "apt-get update failed: {}",
                        tail(&upd0.combined(), 3).join(" ")
                    ),
                ));
            }
            if !Path::new("/etc/ssl/certs/ca-certificates.crt").exists() {
                let ca = run_with(
                    "/usr/bin/apt-get",
                    &["-q", "-y", "install", "ca-certificates"],
                    Duration::from_secs(300),
                    None,
                    &apt_env,
                )
                .ok_or_else(|| err(PrivErrorCode::Internal, "apt-get did not start."))?;
                if !ca.ok() {
                    return Err(err(
                        PrivErrorCode::Internal,
                        "Could not install ca-certificates before configuring Docker repository.",
                    ));
                }
            }

            let gpg_url = format!("https://download.docker.com/linux/{repo_distro}/gpg");
            let gpg_bytes = fetch_https_text(&gpg_url)?;
            verify_pgp_key(&gpg_bytes, DOCKER_APT_GPG_FPR)?;
            write_atomic_bytes(Path::new("/etc/apt/keyrings/docker.asc"), &gpg_bytes, 0o644)?;

            let arch = dpkg_arch();
            let list = apt_sources_line(&repo_distro, &codename, &arch);
            write_atomic_bytes(
                Path::new("/etc/apt/sources.list.d/docker.list"),
                list.as_bytes(),
                0o644,
            )?;

            let upd1 = run_with(
                "/usr/bin/apt-get",
                &["-q", "update"],
                Duration::from_secs(300),
                None,
                &apt_env,
            )
            .ok_or_else(|| err(PrivErrorCode::Internal, "apt-get update did not start."))?;
            if !upd1.ok() {
                return Err(err(
                    PrivErrorCode::Internal,
                    format!(
                        "Could not refresh Docker apt repository: {}",
                        tail(&upd1.combined(), 3).join(" ")
                    ),
                ));
            }

            let inst = run_with(
                "/usr/bin/apt-get",
                &[
                    "-q",
                    "-y",
                    "-o",
                    "Dpkg::Options::=--force-confold",
                    "-o",
                    "Dpkg::Options::=--force-confdef",
                    "install",
                    "docker-ce",
                    "docker-ce-cli",
                    "containerd.io",
                    "docker-buildx-plugin",
                    "docker-compose-plugin",
                ],
                INSTALL_TIMEOUT,
                None,
                &apt_env,
            )
            .ok_or_else(|| {
                err(
                    PrivErrorCode::Internal,
                    "Docker package installation timed out.",
                )
            })?;
            log_tail.extend(tail(&inst.combined(), 15));
            if !inst.ok() {
                return Err(err(
                    PrivErrorCode::Internal,
                    format!(
                        "Docker installation failed: {}",
                        log_tail.last().cloned().unwrap_or_default()
                    ),
                ));
            }
        }
        InstallPlan::DistroApt => {
            let upd = run_with(
                "/usr/bin/apt-get",
                &["-q", "update"],
                Duration::from_secs(300),
                None,
                &apt_env,
            )
            .ok_or_else(|| err(PrivErrorCode::Internal, "apt-get update did not start."))?;
            if !upd.ok() {
                return Err(err(
                    PrivErrorCode::Internal,
                    "Could not refresh package lists before installing Docker.",
                ));
            }
            let inst = run_with(
                "/usr/bin/apt-get",
                &[
                    "-q",
                    "-y",
                    "-o",
                    "Dpkg::Options::=--force-confold",
                    "-o",
                    "Dpkg::Options::=--force-confdef",
                    "install",
                    "docker.io",
                ],
                INSTALL_TIMEOUT,
                None,
                &apt_env,
            )
            .ok_or_else(|| {
                err(
                    PrivErrorCode::Internal,
                    "Docker package installation timed out.",
                )
            })?;
            log_tail.extend(tail(&inst.combined(), 15));
            if !inst.ok() {
                return Err(err(
                    PrivErrorCode::Internal,
                    format!(
                        "Docker installation failed: {}",
                        log_tail.last().cloned().unwrap_or_default()
                    ),
                ));
            }
        }
        InstallPlan::DockerSignedDnf { repo_distro } => {
            let repo_url =
                format!("https://download.docker.com/linux/{repo_distro}/docker-ce.repo");
            let repo_bytes = fetch_https_text(&repo_url)?;
            let repo_text = std::str::from_utf8(&repo_bytes).map_err(|_| {
                err(
                    PrivErrorCode::Internal,
                    "Downloaded Docker repository file is not valid UTF-8.",
                )
            })?;
            if !validate_rpm_repo_text(repo_text, &repo_distro) {
                return Err(err(
                    PrivErrorCode::Internal,
                    "Downloaded Docker repository definition did not pass validation.",
                ));
            }
            let gpg_url = format!("https://download.docker.com/linux/{repo_distro}/gpg");
            let gpg_bytes = fetch_https_text(&gpg_url)?;
            verify_pgp_key(&gpg_bytes, DOCKER_RPM_GPG_FPR)?;
            if let Some(rpm) = find(&["/usr/bin/rpm", "/bin/rpm"]) {
                let tmp_key = Path::new("/tmp/opsbox-docker-rpm.gpg");
                write_atomic_bytes(tmp_key, &gpg_bytes, 0o644)?;
                let res = run(
                    &rpm,
                    &["--import", "/tmp/opsbox-docker-rpm.gpg"],
                    Duration::from_secs(30),
                );
                let _ = std::fs::remove_file(tmp_key);
                if !res.is_some_and(|o| o.ok()) {
                    return Err(err(
                        PrivErrorCode::Internal,
                        "Could not import Docker's RPM GPG key.",
                    ));
                }
            }
            write_atomic_bytes(
                Path::new("/etc/yum.repos.d/docker-ce.repo"),
                repo_bytes.as_slice(),
                0o644,
            )?;

            let inst = run_with(
                "/usr/bin/dnf",
                &[
                    "-q",
                    "-y",
                    "install",
                    "docker-ce",
                    "docker-ce-cli",
                    "containerd.io",
                    "docker-buildx-plugin",
                    "docker-compose-plugin",
                ],
                INSTALL_TIMEOUT,
                None,
                &[],
            )
            .ok_or_else(|| {
                err(
                    PrivErrorCode::Internal,
                    "Docker package installation timed out.",
                )
            })?;
            log_tail.extend(tail(&inst.combined(), 15));
            if !inst.ok() {
                return Err(err(
                    PrivErrorCode::Internal,
                    format!(
                        "Docker installation failed: {}",
                        log_tail.last().cloned().unwrap_or_default()
                    ),
                ));
            }
        }
    }

    if let Some(systemctl) = find(&["/usr/bin/systemctl", "/bin/systemctl"]) {
        let en = run(
            &systemctl,
            &["enable", "--now", "docker.service"],
            Duration::from_secs(60),
        );
        if !en.as_ref().is_some_and(|o| o.ok()) {
            warn!(
                "systemctl enable --now docker.service returned: {:?}",
                en.map(|o| o.combined())
            );
        }
    }

    // Wait briefly for dockerd to bring up /var/run/docker.sock.
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        for _ in 0..30 {
            if rt.block_on(dc.ping_version()).is_ok() {
                log_tail.push("Docker daemon is running.".into());
                return Ok(CommandOutcome {
                    ok: true,
                    tail: log_tail,
                    count: 1,
                });
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    Err(err(
        PrivErrorCode::Internal,
        "Docker packages were installed, but the Docker daemon did not answer yet. Check `systemctl status docker`.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_release_and_install_plan_pick_signed_repos() {
        let ubuntu = parse_os_release(
            "ID=ubuntu\nVERSION_CODENAME=noble\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\n",
        );
        assert_eq!(
            plan_from_os(&ubuntu, true, false),
            Some(InstallPlan::DockerSignedApt {
                repo_distro: "ubuntu".into(),
                codename: "noble".into(),
            })
        );

        let mint =
            parse_os_release("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\nUBUNTU_CODENAME=noble\n");
        assert_eq!(
            plan_from_os(&mint, true, false),
            Some(InstallPlan::DockerSignedApt {
                repo_distro: "ubuntu".into(),
                codename: "noble".into(),
            })
        );

        let rocky = parse_os_release("ID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n");
        assert_eq!(
            plan_from_os(&rocky, false, true),
            Some(InstallPlan::DockerSignedDnf {
                repo_distro: "centos".into(),
            })
        );

        let unknown = parse_os_release("ID=custom\n");
        assert_eq!(
            plan_from_os(&unknown, true, false),
            Some(InstallPlan::DistroApt)
        );
        assert_eq!(plan_from_os(&unknown, false, true), None);
        assert_eq!(plan_from_os(&ubuntu, false, false), None);
    }

    #[test]
    fn apt_source_and_rpm_repo_validation_are_strict() {
        assert_eq!(
            apt_sources_line("ubuntu", "noble", "arm64"),
            "deb [arch=arm64 signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu noble stable\n"
        );
        let good_rpm = "[docker-ce-stable]\nbaseurl=https://download.docker.com/linux/centos/$releasever/$basearch/stable\ngpgcheck=1\ngpgkey=https://download.docker.com/linux/centos/gpg\n";
        assert!(validate_rpm_repo_text(good_rpm, "centos"));
        assert!(!validate_rpm_repo_text(
            &good_rpm.replace("gpgcheck=1", "gpgcheck=0"),
            "centos"
        ));
        assert!(!validate_rpm_repo_text(
            &good_rpm.replace("https://download.docker.com", "https://evil.example"),
            "centos"
        ));
    }

    #[test]
    fn gpg_fingerprint_parser_extracts_fpr_records() {
        let sample = "pub:-:4096:1:8D81803C0EBFCD88:1487797996:::-:::scSC:\nfpr:::::::::9DC858229FC7DD38854AE2D88D81803C0EBFCD88:\nuid:-::::1487797996::123::Docker Release (CE deb) <docker@docker.com>:::\n";
        assert_eq!(
            parse_gpg_fingerprints(sample),
            vec!["9DC858229FC7DD38854AE2D88D81803C0EBFCD88"]
        );
    }

    #[test]
    fn exec_validation_rejects_bad_inputs_before_socket_call() {
        assert!(
            validate_exec_request(&DockerExecRequest {
                id: "../etc".into(),
                cmd: vec!["id".into()],
                user: None,
                env: vec![],
                working_dir: None,
                stdin: None,
            })
            .is_err()
        );
        assert!(
            validate_exec_request(&DockerExecRequest {
                id: "web".into(),
                cmd: vec![],
                user: None,
                env: vec![],
                working_dir: None,
                stdin: None,
            })
            .is_err()
        );
        assert!(
            validate_exec_request(&DockerExecRequest {
                id: "web".into(),
                cmd: vec!["echo".into()],
                user: None,
                env: vec!["BAD_NO_EQUALS".into()],
                working_dir: None,
                stdin: None,
            })
            .is_err()
        );
    }
}
