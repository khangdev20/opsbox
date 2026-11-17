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
    pub created_at: i64,
    pub updated_at: i64,
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

const SERVICE_COLS: &str = "\
    id, project_id, env_id, name, kind, owner_id, source_kind, repo, branch, \
    build_method, dockerfile_path, start_cmd, port, health_path, domain, image_ref, \
    deploy_key_id, auto_deploy, memory_limit_mb, cpu_limit_millis, state, \
    active_deploy_id, active_container, active_host_port, graph_x, graph_y, note, \
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

fn service_from_row(r: &Row<'_>) -> rusqlite::Result<ServiceRecord> {
    let kind_s: String = r.get(4)?;
    let source_s: String = r.get(6)?;
    let port_i: i64 = r.get(12)?;
    let auto_i: i64 = r.get(17)?;
    let state_s: String = r.get(20)?;
    let host_port_i: Option<i64> = r.get(23)?;

    Ok(ServiceRecord {
        id: r.get(0)?,
        project_id: r.get(1)?,
        env_id: r.get(2)?,
        name: r.get(3)?,
        kind: ServiceKind::parse(&kind_s).ok_or_else(|| invalid_col(4, "service kind"))?,
        owner_id: r.get(5)?,
        source_kind: SourceKind::parse(&source_s).ok_or_else(|| invalid_col(6, "source kind"))?,
        repo: r.get(7)?,
        branch: r.get(8)?,
        build_method: r.get(9)?,
        dockerfile_path: r.get(10)?,
        start_cmd: r.get(11)?,
        port: u16::try_from(port_i).map_err(|_| invalid_col(12, "port"))?,
        health_path: r.get(13)?,
        domain: r.get(14)?,
        image_ref: r.get(15)?,
        deploy_key_id: r.get(16)?,
        auto_deploy: auto_i != 0,
        memory_limit_mb: r.get(18)?,
        cpu_limit_millis: r.get(19)?,
        state: ServiceState::parse(&state_s).ok_or_else(|| invalid_col(20, "service state"))?,
        active_deploy_id: r.get(21)?,
        active_container: r.get(22)?,
        active_host_port: host_port_i
            .map(u16::try_from)
            .transpose()
            .map_err(|_| invalid_col(23, "host port"))?,
        graph_x: r.get(24)?,
        graph_y: r.get(25)?,
        note: r.get(26)?,
        created_at: r.get(27)?,
        updated_at: r.get(28)?,
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
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
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
        let id = ulid::Ulid::generate().to_string();
        let conn = self.lock();
        conn.execute(
            "INSERT INTO services (
                id, project_id, env_id, name, kind, owner_id, source_kind, repo, branch,
                build_method, dockerfile_path, start_cmd, port, health_path, domain, image_ref,
                deploy_key_id, auto_deploy, memory_limit_mb, cpu_limit_millis, state,
                graph_x, graph_y, note, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                ?17, ?18, ?19, ?20, 'idle',
                ?21, ?22, ?23, ?24, ?24
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
                new.domain,
                new.image_ref,
                new.deploy_key_id,
                i64::from(new.auto_deploy),
                new.memory_limit_mb,
                new.cpu_limit_millis,
                new.graph_x,
                new.graph_y,
                new.note,
                new.now,
            ],
        )?;
        Ok(conn.query_row(
            &format!("SELECT {SERVICE_COLS} FROM services WHERE id = ?1"),
            params![id],
            service_from_row,
        )?)
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

    pub fn upsert_service_var(
        &self,
        service_id: &str,
        key: &str,
        value_enc: &str,
        is_secret: bool,
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
            "INSERT INTO service_vars (id, service_id, key, value_enc, is_secret, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(service_id, key) DO UPDATE SET
                 value_enc = excluded.value_enc,
                 is_secret = excluded.is_secret,
                 updated_at = excluded.updated_at",
            params![id, service_id, key, value_enc, i64::from(is_secret), now],
        )?;
        let rec = tx.query_row(
            "SELECT id, service_id, key, value_enc, is_secret, created_at, updated_at
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
            "SELECT id, service_id, key, value_enc, is_secret, created_at, updated_at
             FROM service_vars WHERE service_id = ?1 ORDER BY key ASC",
        )?;
        let rows = stmt
            .query_map(params![service_id], var_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete_service_var(&self, service_id: &str, key: &str) -> Result<bool, StoreError> {
        let changed = self.lock().execute(
            "DELETE FROM service_vars WHERE service_id = ?1 AND key = ?2",
            params![service_id, key],
        )?;
        Ok(changed > 0)
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
                i64::from(host_port),
                detected_build_method,
                now,
            ],
        )?;
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
            "SELECT domain, active_host_port FROM services
             WHERE domain IS NOT NULL AND domain != ''
               AND active_host_port IS NOT NULL AND active_host_port > 0
             ORDER BY name ASC",
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
}
