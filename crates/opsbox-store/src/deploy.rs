//! Projects, environments, services, variables, deploys, and build logs (migration 0006).

use std::collections::HashSet;

use rusqlite::{OptionalExtension, Row, TransactionBehavior, params};

use crate::{Store, StoreError};

/// Project row (hosts one or more environments and their services).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRecord {
    pub id: String,
    pub name: String,
    pub created_at: i64,
}

/// Environment row (e.g. `production`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentRecord {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    App,
    Db,
    Tool,
    Compose,
}

impl ServiceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Db => "db",
            Self::Tool => "tool",
            Self::Compose => "compose",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "app" => Some(Self::App),
            "db" => Some(Self::Db),
            "tool" => Some(Self::Tool),
            "compose" => Some(Self::Compose),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Github,
    DeployKey,
    GitUrl,
    DockerImage,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::DeployKey => "deploy_key",
            Self::GitUrl => "git_url",
            Self::DockerImage => "docker_image",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "github" => Some(Self::Github),
            "deploy_key" => Some(Self::DeployKey),
            "git_url" => Some(Self::GitUrl),
            "docker_image" => Some(Self::DockerImage),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    Idle,
    Building,
    Active,
    Failed,
}

impl ServiceState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Building => "building",
            Self::Active => "active",
            Self::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "idle" => Some(Self::Idle),
            "building" => Some(Self::Building),
            "active" => Some(Self::Active),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeployTrigger {
    Manual,
    Webhook,
    Token,
    Rollback,
}

impl DeployTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Webhook => "webhook",
            Self::Token => "token",
            Self::Rollback => "rollback",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "manual" => Some(Self::Manual),
            "webhook" => Some(Self::Webhook),
            "token" => Some(Self::Token),
            "rollback" => Some(Self::Rollback),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeployStatus {
    Building,
    Live,
    Replaced,
    Failed,
}

impl DeployStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Building => "building",
            Self::Live => "live",
            Self::Replaced => "replaced",
            Self::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "building" => Some(Self::Building),
            "live" => Some(Self::Live),
            "replaced" => Some(Self::Replaced),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceRecord {
    pub id: String,
    pub project_id: String,
    pub env_id: String,
    pub name: String,
    pub kind: ServiceKind,
    pub owner_id: Option<String>,
    pub source_kind: SourceKind,
    pub repo: String,
    pub branch: String,
    pub build_method: String,
    pub dockerfile_path: Option<String>,
    pub start_cmd: Option<String>,
    pub port: u16,
    pub health_path: String,
    pub domain: Option<String>,
    pub image_ref: Option<String>,
    pub deploy_key_id: Option<String>,
    pub auto_deploy: bool,
    pub memory_limit_mb: Option<i64>,
    pub cpu_limit_millis: Option<i64>,
    pub state: ServiceState,
    pub active_deploy_id: Option<String>,
    pub active_container: Option<String>,
    pub active_host_port: Option<u16>,
    pub graph_x: i64,
    pub graph_y: i64,
    pub note: Option<String>,
    pub compose_stack_id: Option<String>,
    pub template_id: Option<String>,
    pub internal_only: bool,
    pub public_port: bool,
    pub credentials_rotated_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewService<'a> {
    pub project_id: &'a str,
    pub env_id: &'a str,
    pub name: &'a str,
    pub kind: ServiceKind,
    pub owner_id: Option<&'a str>,
    pub source_kind: SourceKind,
    pub repo: &'a str,
    pub branch: &'a str,
    pub build_method: &'a str,
    pub dockerfile_path: Option<&'a str>,
    pub start_cmd: Option<&'a str>,
    pub port: u16,
    pub health_path: &'a str,
    pub domain: Option<&'a str>,
    pub image_ref: Option<&'a str>,
    pub deploy_key_id: Option<&'a str>,
    pub auto_deploy: bool,
    pub memory_limit_mb: Option<i64>,
    pub cpu_limit_millis: Option<i64>,
    pub graph_x: i64,
    pub graph_y: i64,
    pub note: Option<&'a str>,
    pub now: i64,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateService<'a> {
    pub branch: Option<&'a str>,
    pub build_method: Option<&'a str>,
    pub dockerfile_path: Option<Option<&'a str>>,
    pub start_cmd: Option<Option<&'a str>>,
    pub port: Option<u16>,
    pub health_path: Option<&'a str>,
    pub domain: Option<Option<&'a str>>,
    pub image_ref: Option<Option<&'a str>>,
    pub deploy_key_id: Option<Option<&'a str>>,
    pub auto_deploy: Option<bool>,
    pub memory_limit_mb: Option<Option<i64>>,
    pub cpu_limit_millis: Option<Option<i64>>,
    pub owner_id: Option<Option<&'a str>>,
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceVarRecord {
    pub id: String,
    pub service_id: String,
    pub key: String,
    pub value_enc: String,
    pub is_secret: bool,
    pub ref_expr: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceVarRefRecord {
    pub id: String,
    pub service_id: String,
    pub var_key: String,
    pub target_service_id: String,
    pub target_key: String,
    pub stale: bool,
    pub ref_seq: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencingServiceInfo {
    pub service: ServiceRecord,
    pub var_keys: Vec<String>,
    pub needs_redeploy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployRecord {
    pub id: String,
    pub service_id: String,
    pub commit_sha: Option<String>,
    pub message: String,
    pub trigger_kind: DeployTrigger,
    pub triggered_by: String,
    pub status: DeployStatus,
    pub image_ref: Option<String>,
    pub container_id: Option<String>,
    pub error_message: Option<String>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployLogRow {
    pub id: i64,
    pub deploy_id: String,
    pub ts: i64,
    pub line: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDomainRecord {
    pub id: String,
    pub service_id: String,
    pub domain: String,
    pub is_primary: bool,
    pub redirect_to: Option<String>,
    pub force_https: bool,
    pub hsts: bool,
    pub dns_status: String,
    pub dns_checked_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewServiceDomain<'a> {
    pub domain: &'a str,
    pub is_primary: bool,
    pub redirect_to: Option<&'a str>,
    pub force_https: bool,
    pub hsts: bool,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateServiceDomain<'a> {
    pub redirect_to: Option<Option<&'a str>>,
    pub force_https: Option<bool>,
    pub hsts: Option<bool>,
    pub dns_status: Option<&'a str>,
    pub dns_checked_at: Option<Option<i64>>,
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveServiceRoute {
    pub service_id: String,
    pub service_name: String,
    pub domain: String,
    pub host_port: Option<u16>,
    pub redirect_to: Option<String>,
    pub force_https: bool,
    pub hsts: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeStackRecord {
    pub id: String,
    pub project_id: String,
    pub env_id: String,
    pub name: String,
    pub owner_id: Option<String>,
    pub compose_yaml: String,
    pub project_name: String,
    pub service_names: Vec<String>,
    pub depends_edges: Vec<(String, String)>,
    pub confirmed: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct UpsertComposeStack<'a> {
    pub project_id: &'a str,
    pub env_id: &'a str,
    pub name: &'a str,
    pub owner_id: Option<&'a str>,
    pub compose_yaml: &'a str,
    pub project_name: &'a str,
    pub service_names: &'a [String],
    pub depends_edges: &'a [(String, String)],
    pub confirmed: bool,
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupJobRecord {
    pub id: String,
    pub service_id: String,
    pub template_id: Option<String>,
    pub method: String,
    pub cron: String,
    pub retain_daily: i64,
    pub retain_weekly: i64,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

const DOMAIN_COLS: &str = "\
    id, service_id, domain, is_primary, redirect_to, force_https, hsts, \
    dns_status, dns_checked_at, created_at, updated_at";

const SERVICE_COLS: &str = "\
    id, project_id, env_id, name, kind, owner_id, source_kind, repo, branch, \
    build_method, dockerfile_path, start_cmd, port, health_path, domain, image_ref, \
    deploy_key_id, auto_deploy, memory_limit_mb, cpu_limit_millis, state, \
    active_deploy_id, active_container, active_host_port, graph_x, graph_y, note, \
    compose_stack_id, template_id, internal_only, public_port, credentials_rotated_at, \
    created_at, updated_at";

const DEPLOY_COLS: &str = "\
    id, service_id, commit_sha, message, trigger_kind, triggered_by, status, \
    image_ref, container_id, error_message, started_at, finished_at";

fn invalid_col(col: usize, what: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        col,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid {what}"),
        )),
    )
}

fn service_row_at(r: &Row<'_>, off: usize) -> rusqlite::Result<ServiceRecord> {
    let kind_s: String = r.get(off + 4)?;
    let source_s: String = r.get(off + 6)?;
    let port_i: i64 = r.get(off + 12)?;
    let auto_i: i64 = r.get(off + 17)?;
    let state_s: String = r.get(off + 20)?;
    let host_port_i: Option<i64> = r.get(off + 23)?;
    let internal_i: i64 = r.get(off + 29)?;
    let public_port_i: i64 = r.get(off + 30)?;

    Ok(ServiceRecord {
        id: r.get(off)?,
        project_id: r.get(off + 1)?,
        env_id: r.get(off + 2)?,
        name: r.get(off + 3)?,
        kind: ServiceKind::parse(&kind_s).ok_or_else(|| invalid_col(off + 4, "service kind"))?,
        owner_id: r.get(off + 5)?,
        source_kind: SourceKind::parse(&source_s)
            .ok_or_else(|| invalid_col(off + 6, "source kind"))?,
        repo: r.get(off + 7)?,
        branch: r.get(off + 8)?,
        build_method: r.get(off + 9)?,
        dockerfile_path: r.get(off + 10)?,
        start_cmd: r.get(off + 11)?,
        port: u16::try_from(port_i).map_err(|_| invalid_col(off + 12, "port"))?,
        health_path: r.get(off + 13)?,
        domain: r.get(off + 14)?,
        image_ref: r.get(off + 15)?,
        deploy_key_id: r.get(off + 16)?,
        auto_deploy: auto_i != 0,
        memory_limit_mb: r.get(off + 18)?,
        cpu_limit_millis: r.get(off + 19)?,
        state: ServiceState::parse(&state_s)
            .ok_or_else(|| invalid_col(off + 20, "service state"))?,
        active_deploy_id: r.get(off + 21)?,
        active_container: r.get(off + 22)?,
        active_host_port: host_port_i
            .map(u16::try_from)
            .transpose()
            .map_err(|_| invalid_col(off + 23, "host port"))?,
        graph_x: r.get(off + 24)?,
        graph_y: r.get(off + 25)?,
        note: r.get(off + 26)?,
        compose_stack_id: r.get(off + 27)?,
        template_id: r.get(off + 28)?,
        internal_only: internal_i != 0,
        public_port: public_port_i != 0,
        credentials_rotated_at: r.get(off + 31)?,
        created_at: r.get(off + 32)?,
        updated_at: r.get(off + 33)?,
    })
}

fn service_from_row(r: &Row<'_>) -> rusqlite::Result<ServiceRecord> {
    service_row_at(r, 0)
}

fn compose_stack_from_row(r: &Row<'_>) -> rusqlite::Result<ComposeStackRecord> {
    let names_raw: String = r.get(7)?;
    let edges_raw: String = r.get(8)?;
    let confirmed_i: i64 = r.get(9)?;
    let service_names: Vec<String> =
        serde_json::from_str(&names_raw).map_err(|_| invalid_col(7, "service_names"))?;
    let depends_edges: Vec<(String, String)> =
        serde_json::from_str(&edges_raw).map_err(|_| invalid_col(8, "depends_edges"))?;
    Ok(ComposeStackRecord {
        id: r.get(0)?,
        project_id: r.get(1)?,
        env_id: r.get(2)?,
        name: r.get(3)?,
        owner_id: r.get(4)?,
        compose_yaml: r.get(5)?,
        project_name: r.get(6)?,
        service_names,
        depends_edges,
        confirmed: confirmed_i != 0,
        created_at: r.get(10)?,
        updated_at: r.get(11)?,
    })
}

fn backup_job_from_row(r: &Row<'_>) -> rusqlite::Result<BackupJobRecord> {
    let enabled_i: i64 = r.get(7)?;
    Ok(BackupJobRecord {
        id: r.get(0)?,
        service_id: r.get(1)?,
        template_id: r.get(2)?,
        method: r.get(3)?,
        cron: r.get(4)?,
        retain_daily: r.get(5)?,
        retain_weekly: r.get(6)?,
        enabled: enabled_i != 0,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

fn var_from_row(r: &Row<'_>) -> rusqlite::Result<ServiceVarRecord> {
    let is_sec: i64 = r.get(4)?;
    Ok(ServiceVarRecord {
        id: r.get(0)?,
        service_id: r.get(1)?,
        key: r.get(2)?,
        value_enc: r.get(3)?,
        is_secret: is_sec != 0,
        ref_expr: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

fn var_ref_from_row(r: &Row<'_>) -> rusqlite::Result<ServiceVarRefRecord> {
    let stale_i: i64 = r.get(5)?;
    Ok(ServiceVarRefRecord {
        id: r.get(0)?,
        service_id: r.get(1)?,
        var_key: r.get(2)?,
        target_service_id: r.get(3)?,
        target_key: r.get(4)?,
        stale: stale_i != 0,
        ref_seq: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
    })
}

fn deploy_from_row(r: &Row<'_>) -> rusqlite::Result<DeployRecord> {
    let trig_s: String = r.get(4)?;
    let status_s: String = r.get(6)?;
    Ok(DeployRecord {
        id: r.get(0)?,
        service_id: r.get(1)?,
        commit_sha: r.get(2)?,
        message: r.get(3)?,
        trigger_kind: DeployTrigger::parse(&trig_s).ok_or_else(|| invalid_col(4, "trigger"))?,
        triggered_by: r.get(5)?,
        status: DeployStatus::parse(&status_s).ok_or_else(|| invalid_col(6, "deploy status"))?,
        image_ref: r.get(7)?,
        container_id: r.get(8)?,
        error_message: r.get(9)?,
        started_at: r.get(10)?,
        finished_at: r.get(11)?,
    })
}

fn domain_from_row(r: &Row<'_>) -> rusqlite::Result<ServiceDomainRecord> {
    let prim_i: i64 = r.get(3)?;
    let force_i: i64 = r.get(5)?;
    let hsts_i: i64 = r.get(6)?;
    Ok(ServiceDomainRecord {
        id: r.get(0)?,
        service_id: r.get(1)?,
        domain: r.get(2)?,
        is_primary: prim_i != 0,
        redirect_to: r.get(4)?,
        force_https: force_i != 0,
        hsts: hsts_i != 0,
        dns_status: r.get(7)?,
        dns_checked_at: r.get(8)?,
        created_at: r.get(9)?,
        updated_at: r.get(10)?,
    })
}

fn sync_primary_domain_in_tx(
    tx: &rusqlite::Transaction<'_>,
    service_id: &str,
    now: i64,
) -> Result<Option<String>, StoreError> {
    let cur_primary: Option<(String, String, Option<String>)> = tx
        .query_row(
            "SELECT id, domain, redirect_to FROM service_domains
             WHERE service_id = ?1 AND is_primary = 1
             ORDER BY created_at ASC LIMIT 1",
            params![service_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;

    let chosen: Option<String> = if let Some((id, dom, redir)) = cur_primary
        && redir.is_none()
    {
        tx.execute(
            "UPDATE service_domains SET is_primary = CASE WHEN id = ?2 THEN 1 ELSE 0 END WHERE service_id = ?1",
            params![service_id, id],
        )?;
        Some(dom)
    } else {
        tx.execute(
            "UPDATE service_domains SET is_primary = 0 WHERE service_id = ?1",
            params![service_id],
        )?;
        let candidate: Option<(String, String)> = tx
            .query_row(
                "SELECT id, domain FROM service_domains
                 WHERE service_id = ?1 AND redirect_to IS NULL
                 ORDER BY created_at ASC, id ASC LIMIT 1",
                params![service_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((cid, cdom)) = candidate {
            tx.execute(
                "UPDATE service_domains SET is_primary = 1, updated_at = ?2 WHERE id = ?1",
                params![cid, now],
            )?;
            Some(cdom)
        } else {
            None
        }
    };

    tx.execute(
        "UPDATE services SET domain = ?2, updated_at = ?3 WHERE id = ?1",
        params![service_id, chosen, now],
    )?;
    Ok(chosen)
}

fn ensure_env_in_tx(
    tx: &rusqlite::Transaction<'_>,
    project_id: &str,
    env_name: &str,
    now: i64,
) -> Result<EnvironmentRecord, StoreError> {
    if let Some(env) = tx
        .query_row(
            "SELECT id, project_id, name, created_at FROM environments WHERE project_id = ?1 AND name = ?2",
            params![project_id, env_name],
            |r| {
                Ok(EnvironmentRecord {
                    id: r.get(0)?,
                    project_id: r.get(1)?,
                    name: r.get(2)?,
                    created_at: r.get(3)?,
                })
            },
        )
        .optional()?
    {
        return Ok(env);
    }
    let id = ulid::Ulid::generate().to_string();
    tx.execute(
        "INSERT INTO environments (id, project_id, name, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![id, project_id, env_name, now],
    )?;
    Ok(EnvironmentRecord {
        id,
        project_id: project_id.to_owned(),
        name: env_name.to_owned(),
        created_at: now,
    })
}

impl Store {
    /// Ensure the `default` project and its `production` environment exist.
    pub fn ensure_default_project(
        &self,
        now: i64,
    ) -> Result<(ProjectRecord, EnvironmentRecord), StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let proj = if let Some(existing) = tx
            .query_row(
                "SELECT id, name, created_at FROM projects WHERE name = 'default' OR id = 'default' ORDER BY created_at LIMIT 1",
                [],
                |r| {
                    Ok(ProjectRecord {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        created_at: r.get(2)?,
                    })
                },
            )
            .optional()?
        {
            existing
        } else {
            let id = ulid::Ulid::generate().to_string();
            tx.execute(
                "INSERT INTO projects (id, name, created_at) VALUES (?1, 'default', ?2)",
                params![id, now],
            )?;
            ProjectRecord {
                id,
                name: "default".to_owned(),
                created_at: now,
            }
        };
        let env = ensure_env_in_tx(&tx, &proj.id, "production", now)?;
        tx.commit()?;
        Ok((proj, env))
    }

    /// Resolve a project by id or name, auto-creating `default` on `"default"`.
    pub fn resolve_project(
        &self,
        id_or_name: &str,
        now: i64,
    ) -> Result<Option<(ProjectRecord, EnvironmentRecord)>, StoreError> {
        if id_or_name == "default" {
            return self.ensure_default_project(now).map(Some);
        }
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(proj) = tx
            .query_row(
                "SELECT id, name, created_at FROM projects WHERE id = ?1 OR name = ?1 LIMIT 1",
                params![id_or_name],
                |r| {
                    Ok(ProjectRecord {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        created_at: r.get(2)?,
                    })
                },
            )
            .optional()?
        else {
            return Ok(None);
        };
        let env = ensure_env_in_tx(&tx, &proj.id, "production", now)?;
        tx.commit()?;
        Ok(Some((proj, env)))
    }

    pub fn create_service(&self, new: NewService<'_>) -> Result<ServiceRecord, StoreError> {
        self.create_service_with_exposure(new, None, None, false, false)
    }

    pub fn create_service_with_meta(
        &self,
        new: NewService<'_>,
        compose_stack_id: Option<&str>,
        template_id: Option<&str>,
        internal_only: bool,
    ) -> Result<ServiceRecord, StoreError> {
        self.create_service_with_exposure(new, compose_stack_id, template_id, internal_only, false)
    }

    pub fn create_service_with_exposure(
        &self,
        new: NewService<'_>,
        compose_stack_id: Option<&str>,
        template_id: Option<&str>,
        internal_only: bool,
        public_port: bool,
    ) -> Result<ServiceRecord, StoreError> {
        let id = ulid::Ulid::generate().to_string();
        let norm_domain = new
            .domain
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase);
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO services (
                id, project_id, env_id, name, kind, owner_id, source_kind, repo, branch,
                build_method, dockerfile_path, start_cmd, port, health_path, domain, image_ref,
                deploy_key_id, auto_deploy, memory_limit_mb, cpu_limit_millis, state,
                graph_x, graph_y, note, compose_stack_id, template_id, internal_only, public_port, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                ?17, ?18, ?19, ?20, 'idle',
                ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?28
            )",
            params![
                id,
                new.project_id,
                new.env_id,
                new.name,
                new.kind.as_str(),
                new.owner_id,
                new.source_kind.as_str(),
                new.repo,
                new.branch,
                new.build_method,
                new.dockerfile_path,
                new.start_cmd,
                i64::from(new.port),
                new.health_path,
                norm_domain,
                new.image_ref,
                new.deploy_key_id,
                i64::from(new.auto_deploy),
                new.memory_limit_mb,
                new.cpu_limit_millis,
                new.graph_x,
                new.graph_y,
                new.note,
                compose_stack_id,
                template_id,
                i64::from(internal_only),
                i64::from(public_port),
                new.now,
            ],
        )?;
        if let Some(ref dom) = norm_domain {
            let dom_id = ulid::Ulid::generate().to_string();
            tx.execute(
                "INSERT INTO service_domains (
                    id, service_id, domain, is_primary, redirect_to, force_https, hsts,
                    dns_status, dns_checked_at, created_at, updated_at
                ) VALUES (?1, ?2, ?3, 1, NULL, 1, 1, 'pending', NULL, ?4, ?4)",
                params![dom_id, id, dom, new.now],
            )?;
        }
        let rec = tx.query_row(
            &format!("SELECT {SERVICE_COLS} FROM services WHERE id = ?1"),
            params![id],
            service_from_row,
        )?;
        tx.commit()?;
        Ok(rec)
    }

    pub fn get_service(&self, id_or_name: &str) -> Result<Option<ServiceRecord>, StoreError> {
        Ok(self
            .lock()
            .query_row(
                &format!("SELECT {SERVICE_COLS} FROM services WHERE id = ?1 OR name = ?1 LIMIT 1"),
                params![id_or_name],
                service_from_row,
            )
            .optional()?)
    }

    pub fn list_services(
        &self,
        project_id: &str,
        env_id: &str,
    ) -> Result<Vec<ServiceRecord>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {SERVICE_COLS} FROM services WHERE project_id = ?1 AND env_id = ?2 ORDER BY created_at ASC, name ASC"
        ))?;
        let rows = stmt
            .query_map(params![project_id, env_id], service_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_all_services(&self) -> Result<Vec<ServiceRecord>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {SERVICE_COLS} FROM services ORDER BY created_at ASC, name ASC"
        ))?;
        let rows = stmt
            .query_map([], service_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_services_by_owner(&self, owner_id: &str) -> Result<Vec<ServiceRecord>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {SERVICE_COLS} FROM services WHERE owner_id = ?1 ORDER BY created_at ASC, name ASC"
        ))?;
        let rows = stmt
            .query_map(params![owner_id], service_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_auto_deploy_services(
        &self,
        branch: &str,
    ) -> Result<Vec<ServiceRecord>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {SERVICE_COLS} FROM services WHERE auto_deploy = 1 AND branch = ?1 AND source_kind != 'docker_image'"
        ))?;
        let rows = stmt
            .query_map(params![branch], service_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn update_service(
        &self,
        service_id: &str,
        upd: UpdateService<'_>,
    ) -> Result<Option<ServiceRecord>, StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(cur) = tx
            .query_row(
                &format!("SELECT {SERVICE_COLS} FROM services WHERE id = ?1"),
                params![service_id],
                service_from_row,
            )
            .optional()?
        else {
            return Ok(None);
        };
        let branch = upd.branch.unwrap_or(&cur.branch);
        let build_method = upd.build_method.unwrap_or(&cur.build_method);
        let dockerfile_path = match upd.dockerfile_path {
            Some(v) => v.map(str::to_owned),
            None => cur.dockerfile_path,
        };
        let start_cmd = match upd.start_cmd {
            Some(v) => v.map(str::to_owned),
            None => cur.start_cmd,
        };
        let port = upd.port.unwrap_or(cur.port);
        let health_path = upd.health_path.unwrap_or(&cur.health_path);
        let domain = match upd.domain {
            Some(v) => v.map(str::to_owned),
            None => cur.domain,
        };
        let image_ref = match upd.image_ref {
            Some(v) => v.map(str::to_owned),
            None => cur.image_ref,
        };
        let deploy_key_id = match upd.deploy_key_id {
            Some(v) => v.map(str::to_owned),
            None => cur.deploy_key_id,
        };
        let auto_deploy = upd.auto_deploy.unwrap_or(cur.auto_deploy);
        let memory_limit_mb = upd.memory_limit_mb.unwrap_or(cur.memory_limit_mb);
        let cpu_limit_millis = upd.cpu_limit_millis.unwrap_or(cur.cpu_limit_millis);
        let owner_id = match upd.owner_id {
            Some(v) => v.map(str::to_owned),
            None => cur.owner_id,
        };
        tx.execute(
            "UPDATE services SET
                branch = ?2, build_method = ?3, dockerfile_path = ?4, start_cmd = ?5,
                port = ?6, health_path = ?7, domain = ?8, image_ref = ?9,
                deploy_key_id = ?10, auto_deploy = ?11, memory_limit_mb = ?12,
                cpu_limit_millis = ?13, owner_id = ?14, updated_at = ?15
             WHERE id = ?1",
            params![
                service_id,
                branch,
                build_method,
                dockerfile_path,
                start_cmd,
                i64::from(port),
                health_path,
                domain,
                image_ref,
                deploy_key_id,
                i64::from(auto_deploy),
                memory_limit_mb,
                cpu_limit_millis,
                owner_id,
                upd.now,
            ],
        )?;
        if let Some(dom_opt) = upd.domain {
            match dom_opt.map(str::trim).filter(|s| !s.is_empty()) {
                None => {
                    tx.execute(
                        "DELETE FROM service_domains WHERE service_id = ?1 AND is_primary = 1",
                        params![service_id],
                    )?;
                    sync_primary_domain_in_tx(&tx, service_id, upd.now)?;
                }
                Some(raw) => {
                    let clean = raw.to_ascii_lowercase();
                    let existing_for_svc: Option<(String, Option<String>)> = tx
                        .query_row(
                            "SELECT id, redirect_to FROM service_domains WHERE service_id = ?1 AND domain = ?2",
                            params![service_id, clean],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional()?;
                    if let Some((dom_id, _)) = existing_for_svc {
                        tx.execute(
                            "UPDATE service_domains SET is_primary = CASE WHEN id = ?2 THEN 1 ELSE 0 END, redirect_to = CASE WHEN id = ?2 THEN NULL ELSE redirect_to END, updated_at = ?3 WHERE service_id = ?1",
                            params![service_id, dom_id, upd.now],
                        )?;
                    } else {
                        let cur_prim_id: Option<String> = tx
                            .query_row(
                                "SELECT id FROM service_domains WHERE service_id = ?1 AND is_primary = 1 LIMIT 1",
                                params![service_id],
                                |r| r.get(0),
                            )
                            .optional()?;
                        if let Some(pid) = cur_prim_id {
                            tx.execute(
                                "UPDATE service_domains SET domain = ?2, dns_status = 'pending', dns_checked_at = NULL, updated_at = ?3 WHERE id = ?1",
                                params![pid, clean, upd.now],
                            )?;
                        } else {
                            let new_id = ulid::Ulid::generate().to_string();
                            tx.execute(
                                "INSERT INTO service_domains (
                                    id, service_id, domain, is_primary, redirect_to, force_https, hsts,
                                    dns_status, dns_checked_at, created_at, updated_at
                                ) VALUES (?1, ?2, ?3, 1, NULL, 1, 1, 'pending', NULL, ?4, ?4)",
                                params![new_id, service_id, clean, upd.now],
                            )?;
                        }
                    }
                    sync_primary_domain_in_tx(&tx, service_id, upd.now)?;
                }
            }
        }
        let updated = tx.query_row(
            &format!("SELECT {SERVICE_COLS} FROM services WHERE id = ?1"),
            params![service_id],
            service_from_row,
        )?;
        tx.commit()?;
        Ok(Some(updated))
    }

    pub fn delete_service(&self, service_id: &str) -> Result<Option<ServiceRecord>, StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let found = tx
            .query_row(
                &format!("SELECT {SERVICE_COLS} FROM services WHERE id = ?1"),
                params![service_id],
                service_from_row,
            )
            .optional()?;
        if found.is_some() {
            tx.execute("DELETE FROM services WHERE id = ?1", params![service_id])?;
        }
        tx.commit()?;
        Ok(found)
    }

    pub fn set_service_building(&self, service_id: &str, now: i64) -> Result<(), StoreError> {
        self.lock().execute(
            "UPDATE services SET state = 'building', updated_at = ?2 WHERE id = ?1",
            params![service_id, now],
        )?;
        Ok(())
    }

    pub fn set_service_exposure(
        &self,
        service_id: &str,
        internal_only: bool,
        public_port: bool,
        now: i64,
    ) -> Result<Option<ServiceRecord>, StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE services SET internal_only = ?2, public_port = ?3, updated_at = ?4 WHERE id = ?1",
            params![service_id, i64::from(internal_only), i64::from(public_port), now],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        let rec = tx.query_row(
            &format!("SELECT {SERVICE_COLS} FROM services WHERE id = ?1"),
            params![service_id],
            service_from_row,
        )?;
        tx.commit()?;
        Ok(Some(rec))
    }

    pub fn upsert_service_var(
        &self,
        service_id: &str,
        key: &str,
        value_enc: &str,
        is_secret: bool,
        now: i64,
    ) -> Result<ServiceVarRecord, StoreError> {
        self.upsert_service_var_with_refs(service_id, key, value_enc, is_secret, None, &[], now)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_service_var_with_refs(
        &self,
        service_id: &str,
        key: &str,
        value_enc: &str,
        is_secret: bool,
        ref_expr: Option<&str>,
        targets: &[(String, String)],
        now: i64,
    ) -> Result<ServiceVarRecord, StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing_id: Option<String> = tx
            .query_row(
                "SELECT id FROM service_vars WHERE service_id = ?1 AND key = ?2",
                params![service_id, key],
                |r| r.get(0),
            )
            .optional()?;
        let id = existing_id.unwrap_or_else(|| ulid::Ulid::generate().to_string());
        tx.execute(
            "INSERT INTO service_vars (id, service_id, key, value_enc, is_secret, ref_expr, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT(service_id, key) DO UPDATE SET
                 value_enc = excluded.value_enc,
                 is_secret = excluded.is_secret,
                 ref_expr = excluded.ref_expr,
                 updated_at = excluded.updated_at",
            params![
                id,
                service_id,
                key,
                value_enc,
                i64::from(is_secret),
                ref_expr,
                now,
            ],
        )?;
        tx.execute(
            "DELETE FROM service_var_refs WHERE service_id = ?1 AND var_key = ?2",
            params![service_id, key],
        )?;
        if !targets.is_empty() {
            let has_deployed: bool = tx
                .query_row(
                    "SELECT (active_deploy_id IS NOT NULL OR active_container IS NOT NULL)
                     FROM services WHERE id = ?1",
                    params![service_id],
                    |r| r.get::<_, i64>(0),
                )
                .optional()?
                .is_some_and(|v| v != 0);
            let next_seq: i64 = tx.query_row(
                "SELECT COALESCE(MAX(ref_seq), 0) + 1 FROM service_var_refs WHERE service_id = ?1",
                params![service_id],
                |r| r.get(0),
            )?;
            let stale_i = i64::from(has_deployed);
            for (target_service_id, target_key) in targets {
                let ref_id = ulid::Ulid::generate().to_string();
                tx.execute(
                    "INSERT INTO service_var_refs (
                        id, service_id, var_key, target_service_id, target_key,
                        stale, ref_seq, created_at, updated_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                    ON CONFLICT(service_id, var_key, target_service_id, target_key) DO UPDATE SET
                        stale = excluded.stale,
                        ref_seq = excluded.ref_seq,
                        updated_at = excluded.updated_at",
                    params![
                        ref_id,
                        service_id,
                        key,
                        target_service_id,
                        target_key,
                        stale_i,
                        next_seq,
                        now,
                    ],
                )?;
            }
        }
        let rec = tx.query_row(
            "SELECT id, service_id, key, value_enc, is_secret, ref_expr, created_at, updated_at
             FROM service_vars WHERE service_id = ?1 AND key = ?2",
            params![service_id, key],
            var_from_row,
        )?;
        tx.commit()?;
        Ok(rec)
    }

    pub fn list_service_vars(&self, service_id: &str) -> Result<Vec<ServiceVarRecord>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, service_id, key, value_enc, is_secret, ref_expr, created_at, updated_at
             FROM service_vars WHERE service_id = ?1 ORDER BY key ASC",
        )?;
        let rows = stmt
            .query_map(params![service_id], var_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete_service_var(&self, service_id: &str, key: &str) -> Result<bool, StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM service_var_refs WHERE service_id = ?1 AND var_key = ?2",
            params![service_id, key],
        )?;
        let changed = tx.execute(
            "DELETE FROM service_vars WHERE service_id = ?1 AND key = ?2",
            params![service_id, key],
        )?;
        tx.commit()?;
        Ok(changed > 0)
    }

    pub fn delete_service_vars_referencing(
        &self,
        service_id: &str,
        target_service_id: &str,
    ) -> Result<Vec<String>, StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let keys: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT DISTINCT var_key FROM service_var_refs
                 WHERE service_id = ?1 AND target_service_id = ?2
                 ORDER BY var_key ASC",
            )?;
            let rows = stmt.query_map(params![service_id, target_service_id], |r| r.get(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for key in &keys {
            tx.execute(
                "DELETE FROM service_vars WHERE service_id = ?1 AND key = ?2",
                params![service_id, key],
            )?;
            tx.execute(
                "DELETE FROM service_var_refs WHERE service_id = ?1 AND var_key = ?2",
                params![service_id, key],
            )?;
        }
        tx.execute(
            "DELETE FROM service_var_refs WHERE service_id = ?1 AND target_service_id = ?2",
            params![service_id, target_service_id],
        )?;
        tx.commit()?;
        Ok(keys)
    }

    pub fn list_service_var_refs(
        &self,
        service_id: &str,
    ) -> Result<Vec<ServiceVarRefRecord>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, service_id, var_key, target_service_id, target_key, stale, ref_seq, created_at, updated_at
             FROM service_var_refs WHERE service_id = ?1 ORDER BY var_key ASC, target_key ASC",
        )?;
        let rows = stmt
            .query_map(params![service_id], var_ref_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_referencing_services(
        &self,
        target_service_id: &str,
    ) -> Result<Vec<ReferencingServiceInfo>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT r.var_key, r.stale, {}
             FROM service_var_refs r
             JOIN services s ON s.id = r.service_id
             WHERE r.target_service_id = ?1
             ORDER BY s.name ASC, r.var_key ASC",
            SERVICE_COLS
                .split(',')
                .map(|c| format!("s.{}", c.trim()))
                .collect::<Vec<_>>()
                .join(", ")
        ))?;
        let mut grouped: Vec<ReferencingServiceInfo> = Vec::new();
        let mut rows = stmt.query(params![target_service_id])?;
        while let Some(r) = rows.next()? {
            let var_key: String = r.get(0)?;
            let stale: i64 = r.get(1)?;
            let svc = service_row_at(r, 2)?;
            if let Some(last) = grouped.last_mut().filter(|g| g.service.id == svc.id) {
                if !last.var_keys.contains(&var_key) {
                    last.var_keys.push(var_key);
                }
                if stale != 0 {
                    last.needs_redeploy = true;
                }
            } else {
                grouped.push(ReferencingServiceInfo {
                    service: svc,
                    var_keys: vec![var_key],
                    needs_redeploy: stale != 0,
                });
            }
        }
        Ok(grouped)
    }

    pub fn list_project_var_ref_edges(
        &self,
        project_id: &str,
        env_id: &str,
    ) -> Result<Vec<(String, String)>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT s.name, t.name
             FROM service_var_refs r
             JOIN services s ON s.id = r.service_id
             JOIN services t ON t.id = r.target_service_id
             WHERE s.project_id = ?1 AND s.env_id = ?2
             ORDER BY s.name ASC, t.name ASC",
        )?;
        let rows = stmt
            .query_map(params![project_id, env_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn capture_service_ref_seq(&self, service_id: &str) -> Result<i64, StoreError> {
        let seq: i64 = self.lock().query_row(
            "SELECT COALESCE(MAX(ref_seq), 0) FROM service_var_refs WHERE service_id = ?1",
            params![service_id],
            |r| r.get(0),
        )?;
        Ok(seq)
    }

    pub fn mark_credentials_rotated(
        &self,
        target_service_id: &str,
        rotated_keys: &[&str],
        now: i64,
    ) -> Result<Vec<ReferencingServiceInfo>, StoreError> {
        {
            let mut conn = self.lock();
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "UPDATE services SET credentials_rotated_at = ?2, updated_at = ?2 WHERE id = ?1",
                params![target_service_id, now],
            )?;
            let mut candidates: Vec<(String, String, String)> = Vec::new();
            {
                let mut stmt = tx.prepare(
                    "SELECT r.id, r.service_id, r.target_key
                     FROM service_var_refs r
                     JOIN services s ON s.id = r.service_id
                     WHERE r.target_service_id = ?1
                       AND (s.active_deploy_id IS NOT NULL OR s.active_container IS NOT NULL)",
                )?;
                let mut rows = stmt.query(params![target_service_id])?;
                while let Some(row) = rows.next()? {
                    let id: String = row.get(0)?;
                    let svc_id: String = row.get(1)?;
                    let tkey: String = row.get(2)?;
                    if rotated_keys.iter().any(|k| *k == tkey) {
                        candidates.push((id, svc_id, tkey));
                    }
                }
            }
            for (ref_id, svc_id, _) in candidates {
                let next_seq: i64 = tx.query_row(
                    "SELECT COALESCE(MAX(ref_seq), 0) + 1 FROM service_var_refs WHERE service_id = ?1",
                    params![svc_id],
                    |r| r.get(0),
                )?;
                tx.execute(
                    "UPDATE service_var_refs SET stale = 1, ref_seq = ?2, updated_at = ?3 WHERE id = ?1",
                    params![ref_id, next_seq, now],
                )?;
            }
            tx.commit()?;
        }
        self.list_referencing_services(target_service_id)
    }

    pub fn create_deploy(
        &self,
        service_id: &str,
        commit_sha: Option<&str>,
        message: &str,
        trigger_kind: DeployTrigger,
        triggered_by: &str,
        now: i64,
    ) -> Result<DeployRecord, StoreError> {
        let id = ulid::Ulid::generate().to_string();
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO deploys (id, service_id, commit_sha, message, trigger_kind, triggered_by, status, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'building', ?7)",
            params![
                id,
                service_id,
                commit_sha,
                message,
                trigger_kind.as_str(),
                triggered_by,
                now,
            ],
        )?;
        tx.execute(
            "UPDATE services SET state = 'building', updated_at = ?2 WHERE id = ?1",
            params![service_id, now],
        )?;
        let rec = tx.query_row(
            &format!("SELECT {DEPLOY_COLS} FROM deploys WHERE id = ?1"),
            params![id],
            deploy_from_row,
        )?;
        tx.commit()?;
        Ok(rec)
    }

    pub fn get_deploy(&self, deploy_id: &str) -> Result<Option<DeployRecord>, StoreError> {
        Ok(self
            .lock()
            .query_row(
                &format!("SELECT {DEPLOY_COLS} FROM deploys WHERE id = ?1"),
                params![deploy_id],
                deploy_from_row,
            )
            .optional()?)
    }

    pub fn list_deploys(
        &self,
        service_id: &str,
        limit: usize,
    ) -> Result<Vec<DeployRecord>, StoreError> {
        let conn = self.lock();
        let lim = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut stmt = conn.prepare(&format!(
            "SELECT {DEPLOY_COLS} FROM deploys WHERE service_id = ?1 ORDER BY started_at DESC, id DESC LIMIT ?2"
        ))?;
        let rows = stmt
            .query_map(params![service_id, lim], deploy_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn update_deploy_metadata(
        &self,
        deploy_id: &str,
        commit_sha: Option<&str>,
        message: Option<&str>,
        image_ref: Option<&str>,
        container_id: Option<&str>,
    ) -> Result<(), StoreError> {
        self.lock().execute(
            "UPDATE deploys SET
                commit_sha = COALESCE(?2, commit_sha),
                message = COALESCE(?3, message),
                image_ref = COALESCE(?4, image_ref),
                container_id = COALESCE(?5, container_id)
             WHERE id = ?1",
            params![deploy_id, commit_sha, message, image_ref, container_id],
        )?;
        Ok(())
    }

    /// Atomically mark `deploy_id` as `live`, previous `live` deploy(s) as `replaced`, and
    /// update the service's active pointers. Returns the previous active container ID if any.
    #[allow(clippy::too_many_arguments)]
    pub fn complete_deploy_switch(
        &self,
        service_id: &str,
        deploy_id: &str,
        image_ref: &str,
        container_id: &str,
        host_port: u16,
        detected_build_method: Option<&str>,
        now: i64,
    ) -> Result<Option<String>, StoreError> {
        self.complete_deploy_switch_with_refs(
            service_id,
            deploy_id,
            image_ref,
            container_id,
            host_port,
            detected_build_method,
            None,
            now,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete_deploy_switch_with_refs(
        &self,
        service_id: &str,
        deploy_id: &str,
        image_ref: &str,
        container_id: &str,
        host_port: u16,
        detected_build_method: Option<&str>,
        max_ref_seq: Option<i64>,
        now: i64,
    ) -> Result<Option<String>, StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prev_container: Option<String> = tx
            .query_row(
                "SELECT active_container FROM services WHERE id = ?1",
                params![service_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();

        tx.execute(
            "UPDATE deploys SET status = 'replaced' WHERE service_id = ?1 AND status = 'live' AND id != ?2",
            params![service_id, deploy_id],
        )?;
        tx.execute(
            "UPDATE deploys SET status = 'live', image_ref = ?2, container_id = ?3, error_message = NULL, finished_at = ?4
             WHERE id = ?1",
            params![deploy_id, image_ref, container_id, now],
        )?;
        tx.execute(
            "UPDATE services SET
                state = 'active',
                active_deploy_id = ?2,
                active_container = ?3,
                active_host_port = ?4,
                build_method = COALESCE(?5, build_method),
                updated_at = ?6
             WHERE id = ?1",
            params![
                service_id,
                deploy_id,
                container_id,
                (host_port > 0).then_some(i64::from(host_port)),
                detected_build_method,
                now,
            ],
        )?;
        if let Some(seq) = max_ref_seq {
            tx.execute(
                "UPDATE service_var_refs SET stale = 0, updated_at = ?3 WHERE service_id = ?1 AND ref_seq <= ?2",
                params![service_id, seq, now],
            )?;
        }
        tx.commit()?;
        Ok(prev_container)
    }

    /// Mark a deploy failed. If an older container is still live, keep the service `active`.
    pub fn fail_deploy(
        &self,
        service_id: &str,
        deploy_id: &str,
        error_message: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE deploys SET status = 'failed', error_message = ?2, finished_at = ?3 WHERE id = ?1",
            params![deploy_id, error_message, now],
        )?;
        let has_live: bool = tx
            .query_row(
                "SELECT active_container IS NOT NULL FROM services WHERE id = ?1",
                params![service_id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(false);
        let state = if has_live {
            ServiceState::Active.as_str()
        } else {
            ServiceState::Failed.as_str()
        };
        tx.execute(
            "UPDATE services SET state = ?2, updated_at = ?3 WHERE id = ?1",
            params![service_id, state, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn append_deploy_log(
        &self,
        deploy_id: &str,
        line: &str,
        ts: i64,
    ) -> Result<(), StoreError> {
        self.lock().execute(
            "INSERT INTO deploy_logs (deploy_id, ts, line) VALUES (?1, ?2, ?3)",
            params![deploy_id, ts, line],
        )?;
        Ok(())
    }

    pub fn append_deploy_logs(
        &self,
        deploy_id: &str,
        lines: &[(i64, String)],
    ) -> Result<(), StoreError> {
        if lines.is_empty() {
            return Ok(());
        }
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let mut stmt =
                tx.prepare("INSERT INTO deploy_logs (deploy_id, ts, line) VALUES (?1, ?2, ?3)")?;
            for (ts, line) in lines {
                stmt.execute(params![deploy_id, ts, line])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_deploy_logs(
        &self,
        deploy_id: &str,
        limit: usize,
    ) -> Result<Vec<DeployLogRow>, StoreError> {
        let conn = self.lock();
        let lim = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut stmt = conn.prepare(
            "SELECT id, deploy_id, ts, line FROM deploy_logs WHERE deploy_id = ?1 ORDER BY id ASC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![deploy_id, lim], |r| {
                Ok(DeployLogRow {
                    id: r.get(0)?,
                    deploy_id: r.get(1)?,
                    ts: r.get(2)?,
                    line: r.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Return `(domain, active_host_port)` for every active service with a configured domain and port.
    pub fn list_active_service_domains(&self) -> Result<Vec<(String, u16)>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT sd.domain, s.active_host_port
             FROM service_domains sd
             JOIN services s ON s.id = sd.service_id
             WHERE sd.redirect_to IS NULL
               AND s.active_host_port IS NOT NULL AND s.active_host_port > 0
             ORDER BY s.name ASC, sd.is_primary DESC, sd.created_at ASC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let dom: String = r.get(0)?;
                let port_i: i64 = r.get(1)?;
                let port = u16::try_from(port_i).map_err(|_| invalid_col(1, "active_host_port"))?;
                Ok((dom, port))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Return all active service domain proxy routes (serving + redirect domains).
    pub fn list_active_service_routes(&self) -> Result<Vec<ActiveServiceRoute>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT sd.service_id, s.name, sd.domain, s.active_host_port,
                    sd.redirect_to, sd.force_https, sd.hsts
             FROM service_domains sd
             JOIN services s ON s.id = sd.service_id
             WHERE (sd.redirect_to IS NOT NULL AND s.kind IN ('app', 'tool', 'compose'))
                OR (s.active_host_port IS NOT NULL AND s.active_host_port > 0)
             ORDER BY s.name ASC, sd.is_primary DESC, sd.created_at ASC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let port_i: Option<i64> = r.get(3)?;
                let host_port = port_i
                    .filter(|p| *p > 0)
                    .map(u16::try_from)
                    .transpose()
                    .map_err(|_| invalid_col(3, "active_host_port"))?;
                let force_i: i64 = r.get(5)?;
                let hsts_i: i64 = r.get(6)?;
                Ok(ActiveServiceRoute {
                    service_id: r.get(0)?,
                    service_name: r.get(1)?,
                    domain: r.get(2)?,
                    host_port,
                    redirect_to: r.get(4)?,
                    force_https: force_i != 0,
                    hsts: hsts_i != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_service_domains(
        &self,
        service_id: &str,
    ) -> Result<Vec<ServiceDomainRecord>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {DOMAIN_COLS} FROM service_domains
             WHERE service_id = ?1
             ORDER BY is_primary DESC, created_at ASC, id ASC"
        ))?;
        let rows = stmt
            .query_map(params![service_id], domain_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_service_domain(
        &self,
        domain: &str,
    ) -> Result<Option<ServiceDomainRecord>, StoreError> {
        Ok(self
            .lock()
            .query_row(
                &format!("SELECT {DOMAIN_COLS} FROM service_domains WHERE domain = ?1 LIMIT 1"),
                params![domain.trim().to_ascii_lowercase()],
                domain_from_row,
            )
            .optional()?)
    }

    pub fn list_all_service_domains_with_services(
        &self,
    ) -> Result<Vec<(ServiceDomainRecord, ServiceRecord)>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT sd.id, sd.service_id, sd.domain, sd.is_primary, sd.redirect_to,
                    sd.force_https, sd.hsts, sd.dns_status, sd.dns_checked_at,
                    sd.created_at, sd.updated_at,
                    s.id, s.project_id, s.env_id, s.name, s.kind, s.owner_id, s.source_kind, s.repo, s.branch,
                    s.build_method, s.dockerfile_path, s.start_cmd, s.port, s.health_path, s.domain, s.image_ref,
                    s.deploy_key_id, s.auto_deploy, s.memory_limit_mb, s.cpu_limit_millis, s.state,
                    s.active_deploy_id, s.active_container, s.active_host_port, s.graph_x, s.graph_y, s.note,
                    s.compose_stack_id, s.template_id, s.internal_only, s.public_port, s.credentials_rotated_at,
                    s.created_at, s.updated_at
             FROM service_domains sd
             JOIN services s ON s.id = sd.service_id
             ORDER BY s.name ASC, sd.is_primary DESC, sd.created_at ASC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let dom = ServiceDomainRecord {
                    id: r.get(0)?,
                    service_id: r.get(1)?,
                    domain: r.get(2)?,
                    is_primary: r.get::<_, i64>(3)? != 0,
                    redirect_to: r.get(4)?,
                    force_https: r.get::<_, i64>(5)? != 0,
                    hsts: r.get::<_, i64>(6)? != 0,
                    dns_status: r.get(7)?,
                    dns_checked_at: r.get(8)?,
                    created_at: r.get(9)?,
                    updated_at: r.get(10)?,
                };
                let svc = service_row_at(r, 11)?;
                Ok((dom, svc))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn add_service_domains(
        &self,
        service_id: &str,
        entries: &[NewServiceDomain<'_>],
        now: i64,
    ) -> Result<Vec<ServiceDomainRecord>, StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut has_primary: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM service_domains WHERE service_id = ?1 AND is_primary = 1 AND redirect_to IS NULL)",
                params![service_id],
                |r| r.get(0),
            )?;
        let mut inserted_ids = Vec::with_capacity(entries.len());
        for entry in entries {
            let clean = entry.domain.trim().to_ascii_lowercase();
            let redir = entry
                .redirect_to
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty());
            let want_primary = redir.is_none() && (entry.is_primary || !has_primary);
            if want_primary {
                tx.execute(
                    "UPDATE service_domains SET is_primary = 0 WHERE service_id = ?1",
                    params![service_id],
                )?;
                has_primary = true;
            }
            let id = ulid::Ulid::generate().to_string();
            tx.execute(
                "INSERT INTO service_domains (
                    id, service_id, domain, is_primary, redirect_to, force_https, hsts,
                    dns_status, dns_checked_at, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', NULL, ?8, ?8)",
                params![
                    id,
                    service_id,
                    clean,
                    i64::from(want_primary),
                    redir,
                    i64::from(entry.force_https),
                    i64::from(entry.hsts),
                    now,
                ],
            )?;
            inserted_ids.push(id);
        }
        sync_primary_domain_in_tx(&tx, service_id, now)?;
        let mut out = Vec::with_capacity(inserted_ids.len());
        for id in inserted_ids {
            let rec = tx.query_row(
                &format!("SELECT {DOMAIN_COLS} FROM service_domains WHERE id = ?1"),
                params![id],
                domain_from_row,
            )?;
            out.push(rec);
        }
        tx.commit()?;
        Ok(out)
    }

    pub fn update_service_domain(
        &self,
        domain: &str,
        upd: UpdateServiceDomain<'_>,
    ) -> Result<Option<ServiceDomainRecord>, StoreError> {
        let clean = domain.trim().to_ascii_lowercase();
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(cur) = tx
            .query_row(
                &format!("SELECT {DOMAIN_COLS} FROM service_domains WHERE domain = ?1 LIMIT 1"),
                params![clean],
                domain_from_row,
            )
            .optional()?
        else {
            return Ok(None);
        };
        let redirect_to = match upd.redirect_to {
            Some(v) => v.map(|s| s.trim().to_ascii_lowercase()),
            None => cur.redirect_to,
        };
        let force_https = upd.force_https.unwrap_or(cur.force_https);
        let hsts = upd.hsts.unwrap_or(cur.hsts);
        let dns_status = upd.dns_status.unwrap_or(&cur.dns_status);
        let dns_checked_at = upd.dns_checked_at.unwrap_or(cur.dns_checked_at);
        let is_primary = if redirect_to.is_some() {
            false
        } else {
            cur.is_primary
        };
        tx.execute(
            "UPDATE service_domains SET
                is_primary = ?2,
                redirect_to = ?3,
                force_https = ?4,
                hsts = ?5,
                dns_status = ?6,
                dns_checked_at = ?7,
                updated_at = ?8
             WHERE id = ?1",
            params![
                cur.id,
                i64::from(is_primary),
                redirect_to,
                i64::from(force_https),
                i64::from(hsts),
                dns_status,
                dns_checked_at,
                upd.now,
            ],
        )?;
        sync_primary_domain_in_tx(&tx, &cur.service_id, upd.now)?;
        let updated = tx.query_row(
            &format!("SELECT {DOMAIN_COLS} FROM service_domains WHERE id = ?1"),
            params![cur.id],
            domain_from_row,
        )?;
        tx.commit()?;
        Ok(Some(updated))
    }

    pub fn make_primary_service_domain(
        &self,
        service_id: &str,
        domain: &str,
        now: i64,
    ) -> Result<Option<ServiceDomainRecord>, StoreError> {
        let clean = domain.trim().to_ascii_lowercase();
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(cur) = tx
            .query_row(
                &format!(
                    "SELECT {DOMAIN_COLS} FROM service_domains
                     WHERE service_id = ?1 AND domain = ?2 LIMIT 1"
                ),
                params![service_id, clean],
                domain_from_row,
            )
            .optional()?
        else {
            return Ok(None);
        };
        if cur.redirect_to.is_some() {
            return Ok(None);
        }
        tx.execute(
            "UPDATE service_domains SET is_primary = CASE WHEN id = ?2 THEN 1 ELSE 0 END, updated_at = ?3 WHERE service_id = ?1",
            params![service_id, cur.id, now],
        )?;
        sync_primary_domain_in_tx(&tx, service_id, now)?;
        let updated = tx.query_row(
            &format!("SELECT {DOMAIN_COLS} FROM service_domains WHERE id = ?1"),
            params![cur.id],
            domain_from_row,
        )?;
        tx.commit()?;
        Ok(Some(updated))
    }

    pub fn remove_service_domain(
        &self,
        domain: &str,
        now: i64,
    ) -> Result<Option<(ServiceDomainRecord, Option<String>)>, StoreError> {
        let clean = domain.trim().to_ascii_lowercase();
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(cur) = tx
            .query_row(
                &format!("SELECT {DOMAIN_COLS} FROM service_domains WHERE domain = ?1 LIMIT 1"),
                params![clean],
                domain_from_row,
            )
            .optional()?
        else {
            return Ok(None);
        };
        tx.execute("DELETE FROM service_domains WHERE id = ?1", params![cur.id])?;
        let next_primary = sync_primary_domain_in_tx(&tx, &cur.service_id, now)?;
        tx.commit()?;
        Ok(Some((cur, next_primary)))
    }

    /// Return services whose latest deploy has `status = 'failed'`.
    pub fn list_failed_latest_deploys(
        &self,
    ) -> Result<Vec<(ServiceRecord, DeployRecord)>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "WITH latest AS (
                SELECT d.*, ROW_NUMBER() OVER (PARTITION BY service_id ORDER BY started_at DESC, id DESC) AS rn
                FROM deploys d
            )
            SELECT s.id FROM latest l
            JOIN services s ON s.id = l.service_id
            WHERE l.rn = 1 AND l.status = 'failed'
            ORDER BY l.started_at DESC",
        )?;
        let ids: Vec<String> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut out = Vec::with_capacity(ids.len());
        for svc_id in ids {
            let Some(svc) = conn
                .query_row(
                    &format!("SELECT {SERVICE_COLS} FROM services WHERE id = ?1"),
                    params![svc_id],
                    service_from_row,
                )
                .optional()?
            else {
                continue;
            };
            let Some(dep) = conn
                .query_row(
                    &format!(
                        "SELECT {DEPLOY_COLS} FROM deploys WHERE service_id = ?1 ORDER BY started_at DESC, id DESC LIMIT 1"
                    ),
                    params![svc.id],
                    deploy_from_row,
                )
                .optional()?
            else {
                continue;
            };
            out.push((svc, dep));
        }
        Ok(out)
    }

    /// Return `opsbox/<service>:*` image refs that can be pruned, keeping the active deploy's
    /// image plus the newest `keep` distinct successful (`live`/`replaced`) image refs.
    pub fn prune_old_deploy_image_refs(
        &self,
        service_id: &str,
        keep: usize,
    ) -> Result<Vec<String>, StoreError> {
        let conn = self.lock();
        let active_img: Option<String> = conn
            .query_row(
                "SELECT d.image_ref FROM services s
                 LEFT JOIN deploys d ON d.id = s.active_deploy_id
                 WHERE s.id = ?1",
                params![service_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();

        let mut stmt = conn.prepare(
            "SELECT image_ref, status FROM deploys
             WHERE service_id = ?1 AND image_ref IS NOT NULL
             ORDER BY started_at DESC, id DESC",
        )?;
        let rows: Vec<(String, String)> = stmt
            .query_map(params![service_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;

        let mut kept: HashSet<String> = HashSet::new();
        if let Some(img) = active_img {
            kept.insert(img);
        }
        for (img, status) in &rows {
            if (status == "live" || status == "replaced") && kept.len() < keep {
                kept.insert(img.clone());
            }
        }

        let mut candidates = Vec::new();
        let mut seen = HashSet::new();
        for (img, _) in rows {
            if !kept.contains(&img) && seen.insert(img.clone()) {
                candidates.push(img);
            }
        }
        Ok(candidates)
    }

    pub fn upsert_compose_stack(
        &self,
        stack: UpsertComposeStack<'_>,
    ) -> Result<ComposeStackRecord, StoreError> {
        let names_json = serde_json::to_string(stack.service_names)?;
        let edges_json = serde_json::to_string(stack.depends_edges)?;
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing_id: Option<String> = tx
            .query_row(
                "SELECT id FROM compose_stacks WHERE name = ?1 LIMIT 1",
                params![stack.name],
                |r| r.get(0),
            )
            .optional()?;
        let id = existing_id.unwrap_or_else(|| ulid::Ulid::generate().to_string());
        tx.execute(
            "INSERT INTO compose_stacks (
                id, project_id, env_id, name, owner_id, compose_yaml, project_name,
                service_names, depends_edges, confirmed, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
            ON CONFLICT(name) DO UPDATE SET
                compose_yaml = excluded.compose_yaml,
                project_name = excluded.project_name,
                service_names = excluded.service_names,
                depends_edges = excluded.depends_edges,
                confirmed = CASE WHEN excluded.confirmed = 1 THEN 1 ELSE compose_stacks.confirmed END,
                updated_at = excluded.updated_at",
            params![
                id,
                stack.project_id,
                stack.env_id,
                stack.name,
                stack.owner_id,
                stack.compose_yaml,
                stack.project_name,
                names_json,
                edges_json,
                i64::from(stack.confirmed),
                stack.now,
            ],
        )?;
        let rec = tx.query_row(
            "SELECT id, project_id, env_id, name, owner_id, compose_yaml, project_name,
                    service_names, depends_edges, confirmed, created_at, updated_at
             FROM compose_stacks WHERE id = ?1",
            params![id],
            compose_stack_from_row,
        )?;
        tx.commit()?;
        Ok(rec)
    }

    pub fn get_compose_stack(
        &self,
        id_or_name: &str,
    ) -> Result<Option<ComposeStackRecord>, StoreError> {
        Ok(self
            .lock()
            .query_row(
                "SELECT id, project_id, env_id, name, owner_id, compose_yaml, project_name,
                        service_names, depends_edges, confirmed, created_at, updated_at
                 FROM compose_stacks WHERE id = ?1 OR name = ?1 LIMIT 1",
                params![id_or_name],
                compose_stack_from_row,
            )
            .optional()?)
    }

    pub fn list_compose_stacks(
        &self,
        project_id: &str,
        env_id: &str,
    ) -> Result<Vec<ComposeStackRecord>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, project_id, env_id, name, owner_id, compose_yaml, project_name,
                    service_names, depends_edges, confirmed, created_at, updated_at
             FROM compose_stacks
             WHERE project_id = ?1 AND env_id = ?2
             ORDER BY created_at ASC, name ASC",
        )?;
        let rows = stmt
            .query_map(params![project_id, env_id], compose_stack_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_compose_stacks_by_owner(
        &self,
        owner_id: &str,
    ) -> Result<Vec<ComposeStackRecord>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, project_id, env_id, name, owner_id, compose_yaml, project_name,
                    service_names, depends_edges, confirmed, created_at, updated_at
             FROM compose_stacks
             WHERE owner_id = ?1
             ORDER BY created_at ASC, name ASC",
        )?;
        let rows = stmt
            .query_map(params![owner_id], compose_stack_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn list_services_by_compose_stack(
        &self,
        stack_id: &str,
    ) -> Result<Vec<ServiceRecord>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {SERVICE_COLS} FROM services WHERE compose_stack_id = ?1 ORDER BY created_at ASC, name ASC"
        ))?;
        let rows = stmt
            .query_map(params![stack_id], service_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete_compose_stack(
        &self,
        id_or_name: &str,
    ) -> Result<Option<ComposeStackRecord>, StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let found = tx
            .query_row(
                "SELECT id, project_id, env_id, name, owner_id, compose_yaml, project_name,
                        service_names, depends_edges, confirmed, created_at, updated_at
                 FROM compose_stacks WHERE id = ?1 OR name = ?1 LIMIT 1",
                params![id_or_name],
                compose_stack_from_row,
            )
            .optional()?;
        if let Some(ref stack) = found {
            tx.execute(
                "DELETE FROM services WHERE compose_stack_id = ?1",
                params![stack.id],
            )?;
            tx.execute(
                "DELETE FROM compose_stacks WHERE id = ?1",
                params![stack.id],
            )?;
        }
        tx.commit()?;
        Ok(found)
    }

    pub fn upsert_backup_job(
        &self,
        service_id: &str,
        template_id: Option<&str>,
        method: &str,
        cron: &str,
        enabled: bool,
        now: i64,
    ) -> Result<BackupJobRecord, StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing_id: Option<String> = tx
            .query_row(
                "SELECT id FROM backup_jobs WHERE service_id = ?1 LIMIT 1",
                params![service_id],
                |r| r.get(0),
            )
            .optional()?;
        let id = existing_id.unwrap_or_else(|| ulid::Ulid::generate().to_string());
        tx.execute(
            "INSERT INTO backup_jobs (
                id, service_id, template_id, method, cron, retain_daily, retain_weekly, enabled, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, 14, 8, ?6, ?7, ?7)
            ON CONFLICT(service_id) DO UPDATE SET
                template_id = excluded.template_id,
                method = excluded.method,
                cron = excluded.cron,
                enabled = excluded.enabled,
                updated_at = excluded.updated_at",
            params![id, service_id, template_id, method, cron, i64::from(enabled), now],
        )?;
        let rec = tx.query_row(
            "SELECT id, service_id, template_id, method, cron, retain_daily, retain_weekly, enabled, created_at, updated_at
             FROM backup_jobs WHERE service_id = ?1",
            params![service_id],
            backup_job_from_row,
        )?;
        tx.commit()?;
        Ok(rec)
    }

    pub fn get_backup_job_for_service(
        &self,
        service_id: &str,
    ) -> Result<Option<BackupJobRecord>, StoreError> {
        Ok(self
            .lock()
            .query_row(
                "SELECT id, service_id, template_id, method, cron, retain_daily, retain_weekly, enabled, created_at, updated_at
                 FROM backup_jobs WHERE service_id = ?1 LIMIT 1",
                params![service_id],
                backup_job_from_row,
            )
            .optional()?)
    }
}
