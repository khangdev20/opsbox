//! Typed deploy pipeline messages, frames, and shared input validators.

use serde::{Deserialize, Serialize};

use crate::{PrivError, Secret};

/// Git source type for privileged source fetch and image build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeployGitSource {
    Github,
    DeployKey,
    GitUrl,
}

/// Sealed environment variable passed from `serve` to `priv` (never opened in `serve` if secret).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployEnvSpec {
    pub key: String,
    pub value_enc: String,
    pub seal_context: String,
    pub is_secret: bool,
}

/// Sealed environment variables belonging to a referenced service (`${{ service.KEY }}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployRefSource {
    pub service: String,
    pub vars: Vec<DeployEnvSpec>,
}

/// Privileged source fetch + image build request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployBuildRequest {
    pub service: String,
    pub deploy_id: String,
    pub source_kind: DeployGitSource,
    pub repo_url: String,
    pub branch: String,
    pub commit_sha: Option<String>,
    pub build_method: String,
    pub dockerfile_path: Option<String>,
    pub port: u16,
    pub github_token: Option<Secret>,
    pub deploy_key_enc: Option<String>,
    pub deploy_key_context: Option<String>,
    pub image_tag: String,
    pub cache_from: Option<String>,
    pub vars: Vec<DeployEnvSpec>,
    #[serde(default)]
    pub ref_sources: Vec<DeployRefSource>,
    pub timeout_secs: u64,
    pub memory_limit_mb: Option<i64>,
}

/// Completed build metadata returned at the end of a `DeployBuild` stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployBuildResult {
    pub commit_sha: String,
    pub commit_message: String,
    pub detected_build_method: String,
    pub image_tag: String,
    pub src_dir: String,
}

/// Frames emitted over the `priv` Unix socket for a streaming [`DeployBuildRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeployBuildFrame {
    Log { line: String },
    Done(DeployBuildResult),
    Error(PrivError),
}

/// Privileged Docker pull + retag request for `docker_image` sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployPullRequest {
    pub source_image: String,
    pub target_tag: String,
}

/// Privileged release start + health-check request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployStartRequest {
    pub project: String,
    pub service: String,
    pub deploy_id: String,
    pub image_ref: String,
    pub port: u16,
    pub health_path: String,
    pub health_retries: u8,
    pub vars: Vec<DeployEnvSpec>,
    #[serde(default)]
    pub ref_sources: Vec<DeployRefSource>,
    pub memory_limit_mb: Option<i64>,
    pub cpu_limit_millis: Option<i64>,
}

/// Running container identifier and loopback host port bound on `127.0.0.1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployStartResult {
    pub container_id: String,
    pub host_port: u16,
}

/// Privileged template container pull, named-volume creation, startup, and health check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateStartRequest {
    pub project: String,
    pub service: String,
    pub deploy_id: String,
    pub template_id: String,
    pub version: String,
    pub image_ref: String,
    pub internal_only: bool,
    #[serde(default)]
    pub public_port: bool,
    #[serde(default)]
    pub confirm_public_port: bool,
    pub vars: Vec<DeployEnvSpec>,
    #[serde(default)]
    pub ref_sources: Vec<DeployRefSource>,
    pub memory_limit_mb: Option<i64>,
    pub cpu_limit_millis: Option<i64>,
    /// Named volume to use for the template's first volume slot instead of the default
    /// `opsbox-<service>-<suffix>` (set after a major database upgrade).
    #[serde(default)]
    pub data_volume: Option<String>,
}

/// Privileged credential rotation request inside a running managed database container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseRotateRequest {
    pub service_id: String,
    pub service_name: String,
    pub template_id: String,
    pub container_id: String,
    pub vars: Vec<DeployEnvSpec>,
}

/// Newly sealed variables produced by [`DatabaseRotateRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseRotateResult {
    pub vars: Vec<DeployEnvSpec>,
}

/// Privileged `docker compose config` validation and safety check request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeValidateRequest {
    pub stack: String,
    pub yaml: String,
    pub confirm: bool,
}

/// Parsed Compose service summary returned by [`PrivOp::ComposeValidate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeServiceInfo {
    pub memory_limit_mb: u64,
    pub cpu_limit_millis: u64,
    pub name: String,
    pub image: Option<String>,
    pub port: u16,
    pub depends_on: Vec<String>,
    pub host_network: bool,
}

/// Result of [`PrivOp::ComposeValidate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeValidationResult {
    pub project_name: String,
    pub service_dir: String,
    pub services: Vec<ComposeServiceInfo>,
    pub warnings: Vec<String>,
}

/// Privileged Compose validation + `docker compose -p opsbox-<stack> up -d` execution request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeDeployRequest {
    pub project: String,
    pub stack: String,
    pub yaml: String,
    pub confirm: bool,
    pub vars: Vec<DeployEnvSpec>,
    #[serde(default)]
    pub ref_sources: Vec<DeployRefSource>,
}

/// Runtime state of one deployed Compose service container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeDeployedService {
    pub name: String,
    pub container_id: Option<String>,
    pub image: Option<String>,
    pub state: String,
    pub port: u16,
    pub host_port: Option<u16>,
    pub depends_on: Vec<String>,
}

/// Result of [`PrivOp::ComposeDeploy`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeDeployResult {
    pub project_name: String,
    pub log_lines: Vec<String>,
    pub services: Vec<ComposeDeployedService>,
}

/// Validate a service or project slug: lowercase ASCII alphanumeric + `-`, 1..=63 chars,
/// never starting or ending with `-`.
pub fn valid_service_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 63
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
        && b[0] != b'-'
        && b[b.len() - 1] != b'-'
}

/// Validate a Compose service name: lowercase ASCII alphanumeric + `-` or `_`, 1..=63 chars,
/// never starting or ending with `-` or `_`.
pub fn valid_compose_service_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 63
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-' || *c == b'_')
        && !matches!(b[0], b'-' | b'_')
        && !matches!(b[b.len() - 1], b'-' | b'_')
}

/// Validate a git branch name (strict safe ref characters only, no `..`, leading `-` or `/`).
pub fn valid_git_branch(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && !s.starts_with('-')
        && !s.starts_with('/')
        && !s.ends_with('/')
        && !s.ends_with('.')
        && !s.ends_with(".lock")
        && !s.contains("..")
        && !s.contains("//")
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
}

/// Validate a git commit SHA (7..=64 ASCII hex chars).
pub fn valid_commit_sha(s: &str) -> bool {
    (7..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Validate an environment variable / BuildKit secret key (`[A-Za-z_][A-Za-z0-9_]*`, max 64).
pub fn valid_env_key(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && (b[0].is_ascii_alphabetic() || b[0] == b'_')
        && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

/// Validate a SQL/database user or database identifier (`[A-Za-z0-9_-]{1,63}`).
pub fn valid_db_identifier(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 63
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-')
}

/// Validate a Docker image reference (tag or digest).
pub fn valid_docker_image_ref(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 256
        && !matches!(b[0], b'-' | b'.' | b'/' | b':' | b'@')
        && !s.contains("..")
        && s.bytes().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'/' | b':' | b'@' | b'-')
        })
}

/// Validate a relative repository subpath (such as a `Dockerfile` path).
pub fn valid_rel_repo_path(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && !s.starts_with('/')
        && !s.starts_with('\\')
        && !s.contains('\0')
        && !s.bytes().any(|b| b.is_ascii_control())
        && s.split(['/', '\\'])
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// Validate a health check target (`"tcp"` or HTTP path starting with `/`).
pub fn valid_health_path(s: &str) -> bool {
    s == "tcp" || (s.starts_with('/') && s.len() <= 256 && s.bytes().all(|b| b.is_ascii_graphic()))
}

/// Validate an optional domain name attached to a service.
pub fn valid_domain_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && !s.starts_with('.')
        && !s.ends_with('.')
        && s.contains('.')
        && s.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_deploy_identifiers_and_paths() {
        assert!(valid_service_name("web"));
        assert!(valid_service_name("shop-api-2"));
        assert!(!valid_service_name("-bad"));
        assert!(!valid_service_name("Bad"));
        assert!(!valid_service_name(""));

        assert!(valid_git_branch("main"));
        assert!(valid_git_branch("release/v1.2"));
        assert!(!valid_git_branch("-uext"));
        assert!(!valid_git_branch("a..b"));
        assert!(!valid_git_branch("main;rm"));

        assert!(valid_commit_sha("a3f9c21"));
        assert!(!valid_commit_sha("short"));
        assert!(!valid_commit_sha("zzzzzzz"));

        assert!(valid_env_key("DATABASE_URL"));
        assert!(!valid_env_key("1BAD"));
        assert!(!valid_env_key("A,B=C"));

        assert!(valid_docker_image_ref("nginx:1.27-alpine"));
        assert!(valid_docker_image_ref("ghcr.io/acme/app:v1"));
        assert!(!valid_docker_image_ref("-bad"));
        assert!(!valid_docker_image_ref("nginx;id"));

        assert!(valid_rel_repo_path("Dockerfile"));
        assert!(valid_rel_repo_path("apps/web/Dockerfile.prod"));
        assert!(!valid_rel_repo_path("/etc/passwd"));
        assert!(!valid_rel_repo_path("../Dockerfile"));

        assert!(valid_health_path("/healthz"));
        assert!(!valid_health_path("healthz"));
        assert!(valid_domain_name("app.example.com"));
        assert!(!valid_domain_name("localhost"));

        assert!(valid_db_identifier("app"));
        assert!(valid_db_identifier("my_db_1-test"));
        assert!(!valid_db_identifier(""));
        assert!(!valid_db_identifier("app;DROP"));
        assert!(!valid_db_identifier("-bad"));
    }
}
