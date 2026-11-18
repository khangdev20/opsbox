//! Managed PostgreSQL administration inside `priv` (docs/BACKEND_SPEC.md 7.6, Databases part B).
//!
//! Everything here runs `psql`/`pg_dumpall` *inside* the database container with `docker exec`.
//! The administrator password is opened from sealed variables, passed through the exec
//! environment (never argv) and scrubbed from every error. SQL is built only from identifiers
//! that passed [`valid_pg_name`] / the extension allowlist, plus generated alphanumeric
//! passwords, so nothing user-supplied is interpolated unquoted.

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use opsbox_deploy::pipeline::scrub_secrets;
use opsbox_deploy::templates::{get_template, is_postgres_template, random_password};
use opsbox_docker::{DockerError, PortBindMode, TemplateContainerSpec};
use opsbox_proto::{
    ALLOWED_EXTENSIONS, APP_CONNECTION_VARS, BackupRunTarget, DatabaseAction,
    DatabaseInspectRequest, DatabaseInspectResult, DatabaseManageRequest, DatabaseManageResult,
    DatabasePoolerRequest, DatabasePoolerResult, DatabaseUpgradeRequest, DatabaseUpgradeResult,
    DbDatabaseInfo, DbExtensionInfo, DbGrantInfo, DbSlowQuery, DbTableSize, DbTarget, DbUserInfo,
    DeployEnvSpec, DockerContainerAction, DockerExecRequest, PrivError, PrivErrorCode, PrivReply,
    Secret, TemplateStartRequest, allowed_extension, pg_major, valid_db_identifier,
    valid_docker_image_ref, valid_docker_ref, valid_pg_name, valid_service_name,
};
use serde_json::Value;

use super::Env;
use super::deploy::{err, open_deploy_vars, short_deploy_suffix};
use super::docker::{block_on_fut, map_docker_err};
use super::sealed;
use super::templates_compose::start_template_with;

/// Pinned PgBouncer image (checked against the image's entrypoint contract:
/// `DB_USER` becomes `auth_user`, `AUTH_QUERY`/`AUTH_DBNAME` are supported).
pub const PGBOUNCER_IMAGE: &str = "edoburu/pgbouncer:v1.26.1-p0";
const POOLER_ROLE: &str = "opsbox_pooler";
const POOLER_SCHEMA: &str = "opsbox_pooler";
const POOLER_PASSWORD_VAR: &str = "OPSBOX_POOLER_PASSWORD";
const POOLER_PORT: u16 = 6432;
const DUMP_TIMEOUT: Duration = Duration::from_secs(3600);
const RESTORE_TIMEOUT: Duration = Duration::from_secs(3600);

// ---------------------------------------------------------------------------------- session

struct Session<'a> {
    env: &'a Env,
    container: &'a str,
    user: String,
    password: String,
    database: String,
    scrub: Vec<String>,
}

fn check_target(t: &DbTarget) -> Result<(), PrivError> {
    let id_ok = !t.service_id.is_empty()
        && t.service_id.len() <= 64
        && t.service_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !id_ok || !valid_service_name(&t.service_name) || !valid_docker_ref(&t.container_id) {
        return Err(err(PrivErrorCode::Invalid, "Invalid database service."));
    }
    if !is_postgres_template(&t.template_id) {
        return Err(err(
            PrivErrorCode::Invalid,
            "Database management is available for PostgreSQL databases.",
        ));
    }
    for v in &t.vars {
        if v.seal_context != format!("deploy:var:{}:{}", t.service_id, v.key) {
            return Err(err(
                PrivErrorCode::Refused,
                "Secret sealing context does not match database service.",
            ));
        }
    }
    Ok(())
}

impl<'a> Session<'a> {
    fn open(env: &'a Env, t: &'a DbTarget) -> Result<Self, PrivError> {
        check_target(t)?;
        let (opened, mut scrub) = open_deploy_vars(&env.secrets, &t.vars)?;
        let get = |k: &str| opened.iter().find(|v| v.key == k).map(|v| v.value.clone());
        let user = get("POSTGRES_USER").unwrap_or_else(|| "app".into());
        let database = get("POSTGRES_DB").unwrap_or_else(|| "app".into());
        let password = get("POSTGRES_PASSWORD")
            .filter(|p| !p.is_empty())
            .ok_or_else(|| {
                err(
                    PrivErrorCode::Conflict,
                    "This database has no administrator password stored.",
                )
            })?;
        if !valid_db_identifier(&user) || !valid_db_identifier(&database) {
            return Err(err(
                PrivErrorCode::Invalid,
                "Invalid PostgreSQL user or database identifier.",
            ));
        }
        scrub.push(password.clone());
        let running = match block_on_fut(env.docker.is_container_running(&t.container_id))? {
            Ok(r) => r,
            Err(DockerError::NotFound(_)) => false,
            Err(e) => return Err(map_docker_err(e, env.docker.socket_path())),
        };
        if !running {
            return Err(err(
                PrivErrorCode::Conflict,
                "Start this database before managing it.",
            ));
        }
        Ok(Self {
            env,
            container: &t.container_id,
            user,
            password,
            database,
            scrub,
        })
    }

    fn scrub_refs(&self) -> Vec<&str> {
        self.scrub.iter().map(String::as_str).collect()
    }

    fn exec_req(&self, db: &str, sql: &str) -> DockerExecRequest {
        DockerExecRequest {
            id: self.container.to_owned(),
            cmd: vec![
                "psql".into(),
                "-X".into(),
                "-q".into(),
                "-A".into(),
                "-t".into(),
                "-v".into(),
                "ON_ERROR_STOP=1".into(),
                "-U".into(),
                self.user.clone(),
                "-d".into(),
                db.to_owned(),
            ],
            user: None,
            env: vec![format!("PGPASSWORD={}", self.password)],
            working_dir: None,
            stdin: Some(sql.as_bytes().to_vec()),
        }
    }

    /// Run SQL through `psql` in database `db`; returns stdout.
    fn psql(&self, db: &str, sql: &str) -> Result<String, PrivError> {
        if !valid_db_identifier(db) {
            return Err(err(PrivErrorCode::Invalid, "Invalid database name."));
        }
        let out =
            block_on_fut(self.env.docker.exec(self.exec_req(db, sql)))?.map_err(|mut e| {
                if let DockerError::Internal(ref mut m) = e {
                    *m = scrub_secrets(m, &self.scrub_refs());
                }
                map_docker_err(e, self.env.docker.socket_path())
            })?;
        if out.exit_code != 0 {
            let raw = out.stderr.trim();
            let detail = if raw.is_empty() {
                "the database refused the command".to_owned()
            } else {
                scrub_secrets(raw, &self.scrub_refs())
            };
            return Err(err(PrivErrorCode::Conflict, detail));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

fn seal_var(
    env: &Env,
    service_id: &str,
    key: &str,
    value: String,
    is_secret: bool,
) -> Result<DeployEnvSpec, PrivError> {
    let ctx = format!("deploy:var:{service_id}:{key}");
    let PrivReply::Sealed { sealed } = sealed::seal(&env.secrets, &ctx, &Secret::new(value))?
    else {
        return Err(err(PrivErrorCode::Internal, "Could not seal a variable."));
    };
    Ok(DeployEnvSpec {
        key: key.to_owned(),
        value_enc: sealed,
        seal_context: ctx,
        is_secret,
    })
}

fn new_password() -> Result<String, PrivError> {
    random_password(24).map_err(|e| err(PrivErrorCode::Internal, e.to_string()))
}

// ------------------------------------------------------------------------------ SQL builders

fn lit(s: &str) -> String {
    debug_assert!(s.bytes().all(|b| b.is_ascii_alphanumeric()));
    format!("'{s}'")
}

fn extension_list_sql() -> String {
    ALLOWED_EXTENSIONS
        .iter()
        .map(|e| format!("'{e}'"))
        .collect::<Vec<_>>()
        .join(",")
}

/// One JSON document with everything except slow queries.
pub(crate) fn inspect_sql() -> String {
    format!(
        r#"SELECT json_build_object(
 'version', current_setting('server_version'),
 'database', current_database(),
 'databases', (SELECT coalesce(json_agg(json_build_object('name', d.datname, 'owner', pg_get_userbyid(d.datdba), 'size', pg_database_size(d.oid)) ORDER BY d.datname), '[]'::json) FROM pg_database d WHERE NOT d.datistemplate),
 'users', (SELECT coalesce(json_agg(json_build_object('name', r.rolname, 'login', r.rolcanlogin, 'super', r.rolsuper,
      'conns', (SELECT count(*) FROM pg_stat_activity a WHERE a.usename = r.rolname),
      'dbs', (SELECT coalesce(json_agg(d.datname ORDER BY d.datname), '[]'::json) FROM pg_database d WHERE NOT d.datistemplate AND has_database_privilege(r.oid, d.oid, 'CONNECT'))
   ) ORDER BY r.rolname), '[]'::json) FROM pg_roles r WHERE r.rolname !~ '^pg_'),
 'grants', (SELECT coalesce(json_agg(json_build_object('role', r.rolname, 'db', d.datname,
      'connect', has_database_privilege(r.oid, d.oid, 'CONNECT'),
      'create', has_database_privilege(r.oid, d.oid, 'CREATE'),
      'temp', has_database_privilege(r.oid, d.oid, 'TEMPORARY')) ORDER BY r.rolname, d.datname), '[]'::json)
      FROM pg_roles r CROSS JOIN pg_database d WHERE r.rolcanlogin AND NOT r.rolsuper AND NOT d.datistemplate),
 'extensions', (SELECT coalesce(json_agg(json_build_object('name', a.name, 'installed', e.extname IS NOT NULL, 'installed_version', e.extversion, 'default_version', a.default_version, 'comment', coalesce(a.comment, '')) ORDER BY a.name), '[]'::json)
      FROM pg_available_extensions a LEFT JOIN pg_extension e ON e.extname = a.name WHERE a.name IN ({exts})),
 'tables', (SELECT coalesce(json_agg(t), '[]'::json) FROM (SELECT schemaname AS schema, relname AS "table", pg_total_relation_size(relid) AS bytes, n_live_tup AS rows FROM pg_stat_user_tables ORDER BY pg_total_relation_size(relid) DESC LIMIT 25) t),
 'conns_now', (SELECT count(*) FROM pg_stat_activity WHERE backend_type = 'client backend'),
 'conns_max', current_setting('max_connections')::int,
 'stat_loaded', current_setting('shared_preload_libraries') LIKE '%pg_stat_statements%',
 'stat_enabled', EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'pg_stat_statements')
)::text;
"#,
        exts = extension_list_sql()
    )
}

const SLOW_SQL: &str = r#"SELECT coalesce(json_agg(q), '[]'::json)::text FROM (SELECT left(query, 400) AS query, calls, round(mean_exec_time::numeric, 2)::float8 AS mean_ms, round(total_exec_time::numeric, 2)::float8 AS total_ms, rows FROM pg_stat_statements WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) AND query NOT LIKE '%pg_stat_statements%' ORDER BY mean_exec_time DESC LIMIT 10) q;
"#;

pub(crate) fn role_sql(role: &str, password: &str) -> String {
    let (r, p) = (lit_name(role), lit(password));
    format!(
        "DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = {r}) THEN CREATE ROLE \"{role}\" LOGIN PASSWORD {p} NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION CONNECTION LIMIT 50; ELSE ALTER ROLE \"{role}\" LOGIN PASSWORD {p} NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION CONNECTION LIMIT 50; END IF; END $$;\n"
    )
}

fn lit_name(s: &str) -> String {
    debug_assert!(valid_pg_name(s));
    format!("'{s}'")
}

pub(crate) fn provision_sql(role: &str, db: &str, password: &str, admin_db: &str) -> String {
    let mut sql = role_sql(role, password);
    sql.push_str(&format!(
        "SELECT format('CREATE DATABASE %I OWNER %I', {d}, {r}) WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = {d}) \\gexec\n\
         ALTER DATABASE \"{db}\" OWNER TO \"{role}\";\n\
         REVOKE ALL ON DATABASE \"{db}\" FROM PUBLIC;\n\
         GRANT CONNECT, TEMPORARY ON DATABASE \"{db}\" TO \"{role}\";\n\
         REVOKE CONNECT ON DATABASE \"{admin_db}\" FROM PUBLIC;\n\
         REVOKE CONNECT ON DATABASE postgres FROM PUBLIC;\n\
         REVOKE CONNECT ON DATABASE template1 FROM PUBLIC;\n\
         \\connect \"{db}\"\n\
         REVOKE ALL ON SCHEMA public FROM PUBLIC;\n\
         ALTER SCHEMA public OWNER TO \"{role}\";\n",
        d = lit_name(db),
        r = lit_name(role),
    ));
    sql
}

pub(crate) fn drop_sql(role: &str, db: &str, drop_database: bool, admin_user: &str) -> String {
    if drop_database {
        format!("DROP DATABASE IF EXISTS \"{db}\" WITH (FORCE);\nDROP ROLE IF EXISTS \"{role}\";\n")
    } else {
        format!(
            "\\connect \"{db}\"\nREASSIGN OWNED BY \"{role}\" TO \"{admin_user}\";\nDROP OWNED BY \"{role}\";\nDROP ROLE IF EXISTS \"{role}\";\n"
        )
    }
}

pub(crate) fn pooler_setup_sql(password: &str) -> String {
    let mut sql = role_sql(POOLER_ROLE, password);
    sql.push_str(&format!(
        "ALTER ROLE \"{POOLER_ROLE}\" CONNECTION LIMIT 5;\n\
         GRANT CONNECT ON DATABASE postgres TO \"{POOLER_ROLE}\";\n\
         \\connect postgres\n\
         CREATE SCHEMA IF NOT EXISTS {POOLER_SCHEMA};\n\
         REVOKE ALL ON SCHEMA {POOLER_SCHEMA} FROM PUBLIC;\n\
         GRANT USAGE ON SCHEMA {POOLER_SCHEMA} TO \"{POOLER_ROLE}\";\n\
         CREATE OR REPLACE FUNCTION {POOLER_SCHEMA}.lookup(i_username text, OUT uname text, OUT phash text) RETURNS record AS $f$ SELECT usename::text, passwd::text FROM pg_catalog.pg_shadow WHERE usename = i_username $f$ LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog;\n\
         REVOKE ALL ON FUNCTION {POOLER_SCHEMA}.lookup(text) FROM PUBLIC;\n\
         GRANT EXECUTE ON FUNCTION {POOLER_SCHEMA}.lookup(text) TO \"{POOLER_ROLE}\";\n"
    ));
    sql
}

pub(crate) fn pooler_remove_sql() -> String {
    format!(
        "\\connect postgres\nDROP SCHEMA IF EXISTS {POOLER_SCHEMA} CASCADE;\nREVOKE CONNECT ON DATABASE postgres FROM \"{POOLER_ROLE}\";\nDROP ROLE IF EXISTS \"{POOLER_ROLE}\";\n"
    )
}

// ------------------------------------------------------------------------------------ inspect

fn jstr(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_owned()
}
fn ju64(v: &Value, k: &str) -> u64 {
    v.get(k)
        .and_then(|x| x.as_u64().or_else(|| x.as_f64().map(|f| f.max(0.0) as u64)))
        .unwrap_or(0)
}
fn jbool(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}
fn jarr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k)
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
}

pub(crate) fn parse_inspect(doc: &Value) -> DatabaseInspectResult {
    DatabaseInspectResult {
        server_version: jstr(doc, "version"),
        database: jstr(doc, "database"),
        databases: jarr(doc, "databases")
            .iter()
            .map(|d| DbDatabaseInfo {
                name: jstr(d, "name"),
                owner: jstr(d, "owner"),
                size_bytes: ju64(d, "size"),
            })
            .collect(),
        users: jarr(doc, "users")
            .iter()
            .map(|u| DbUserInfo {
                name: jstr(u, "name"),
                can_login: jbool(u, "login"),
                superuser: jbool(u, "super"),
                connections: u32::try_from(ju64(u, "conns")).unwrap_or(u32::MAX),
                databases: jarr(u, "dbs")
                    .iter()
                    .filter_map(|d| d.as_str().map(str::to_owned))
                    .collect(),
            })
            .collect(),
        grants: jarr(doc, "grants")
            .iter()
            .map(|g| DbGrantInfo {
                role: jstr(g, "role"),
                database: jstr(g, "db"),
                connect: jbool(g, "connect"),
                create: jbool(g, "create"),
                temp: jbool(g, "temp"),
            })
            .collect(),
        extensions: jarr(doc, "extensions")
            .iter()
            .map(|e| DbExtensionInfo {
                name: jstr(e, "name"),
                installed: jbool(e, "installed"),
                installed_version: e
                    .get("installed_version")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                default_version: e
                    .get("default_version")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                comment: jstr(e, "comment"),
            })
            .collect(),
        tables: jarr(doc, "tables")
            .iter()
            .map(|t| DbTableSize {
                schema: jstr(t, "schema"),
                table: jstr(t, "table"),
                total_bytes: ju64(t, "bytes"),
                rows_estimate: i64::try_from(ju64(t, "rows")).unwrap_or(i64::MAX),
            })
            .collect(),
        connections_current: u32::try_from(ju64(doc, "conns_now")).unwrap_or(u32::MAX),
        connections_max: u32::try_from(ju64(doc, "conns_max")).unwrap_or(u32::MAX),
        stat_statements_loaded: jbool(doc, "stat_loaded"),
        stat_statements_enabled: jbool(doc, "stat_enabled"),
        slow_queries: Vec::new(),
    }
}

pub(crate) fn parse_slow(doc: &Value) -> Vec<DbSlowQuery> {
    doc.as_array()
        .map(|rows| {
            rows.iter()
                .map(|q| DbSlowQuery {
                    query: jstr(q, "query"),
                    calls: ju64(q, "calls"),
                    mean_ms: q.get("mean_ms").and_then(Value::as_f64).unwrap_or(0.0),
                    total_ms: q.get("total_ms").and_then(Value::as_f64).unwrap_or(0.0),
                    rows: ju64(q, "rows"),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_json_out(out: &str) -> Result<Value, PrivError> {
    serde_json::from_str(out.trim()).map_err(|_| {
        err(
            PrivErrorCode::Internal,
            "The database returned unexpected output.",
        )
    })
}

pub fn inspect_database_with(
    env: &Env,
    req: &DatabaseInspectRequest,
) -> Result<DatabaseInspectResult, PrivError> {
    let s = Session::open(env, &req.target)?;
    let db = req.database.clone().unwrap_or_else(|| s.database.clone());
    if !valid_db_identifier(&db) {
        return Err(err(PrivErrorCode::Invalid, "Invalid database name."));
    }
    let doc = parse_json_out(&s.psql(&db, &inspect_sql())?)?;
    let mut result = parse_inspect(&doc);
    if result.stat_statements_loaded && result.stat_statements_enabled {
        // Best effort: a failing statistics view must not hide the rest of the page.
        if let Ok(out) = s.psql(&db, SLOW_SQL)
            && let Ok(doc) = parse_json_out(&out)
        {
            result.slow_queries = parse_slow(&doc);
        }
    }
    Ok(result)
}

// ------------------------------------------------------------------------------------ manage

fn app_vars(
    env: &Env,
    t: &DbTarget,
    prefix: &str,
    role: &str,
    db: &str,
    password: &str,
    pooler: (&str, u16),
) -> Result<Vec<DeployEnvSpec>, PrivError> {
    let host = &t.service_name;
    let url = |h: &str, port: u16| format!("postgresql://{role}:{password}@{h}:{port}/{db}");
    let mut out = Vec::new();
    for (suffix, _, secret, pooled) in APP_CONNECTION_VARS {
        let (h, port) = if *pooled {
            pooler
        } else {
            (host.as_str(), 5432)
        };
        let value = match *suffix {
            "DATABASE_URL" | "POOLED_DATABASE_URL" => url(h, port),
            "PGHOST" | "POOLED_PGHOST" => h.to_owned(),
            "PGPORT" | "POOLED_PGPORT" => port.to_string(),
            "PGUSER" => role.to_owned(),
            "PGPASSWORD" => password.to_owned(),
            "PGDATABASE" => db.to_owned(),
            _ => continue,
        };
        out.push(seal_var(
            env,
            &t.service_id,
            &format!("{prefix}{suffix}"),
            value,
            *secret,
        )?);
    }
    Ok(out)
}

fn check_app_names(
    s: &Session<'_>,
    prefix: &str,
    role: &str,
    db: &str,
    pooler_host: &str,
) -> Result<(), PrivError> {
    let prefix_ok = prefix.starts_with(opsbox_proto::APP_VAR_PREFIX)
        && prefix.ends_with('_')
        && prefix
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
    if !valid_pg_name(role) || !valid_pg_name(db) || !prefix_ok || !valid_service_name(pooler_host)
    {
        return Err(err(
            PrivErrorCode::Invalid,
            "Invalid app database name. Use a shorter app name.",
        ));
    }
    if role == s.user || db == s.database || role == POOLER_ROLE || db == "postgres" {
        return Err(err(
            PrivErrorCode::Conflict,
            "That name is already used by the main database login.",
        ));
    }
    Ok(())
}

fn missing_extension_hint(ext: &str) -> String {
    let flavour = match ext {
        "vector" => " Create the database from the \"PostgreSQL + pgvector\" template.",
        "postgis" => " Create the database from the \"PostgreSQL + PostGIS\" template.",
        _ => "",
    };
    format!("This PostgreSQL image does not include {ext}.{flavour}")
}

pub fn manage_database_with(
    env: &Env,
    req: &DatabaseManageRequest,
) -> Result<DatabaseManageResult, PrivError> {
    let s = Session::open(env, &req.target)?;
    match &req.action {
        DatabaseAction::ProvisionApp {
            db_name,
            role,
            var_prefix,
            pooler_host,
            pooler_port,
            ..
        }
        | DatabaseAction::RotateApp {
            db_name,
            role,
            var_prefix,
            pooler_host,
            pooler_port,
            ..
        } => {
            check_app_names(&s, var_prefix, role, db_name, pooler_host)?;
            let rotate = matches!(req.action, DatabaseAction::RotateApp { .. });
            let pw = new_password()?;
            let mut scrub = s.scrub.clone();
            scrub.push(pw.clone());
            let sql = if rotate {
                let exists = s.psql(
                    &s.database,
                    &format!(
                        "SELECT 1 FROM pg_roles WHERE rolname = {};\n",
                        lit_name(role)
                    ),
                )?;
                if exists.trim() != "1" {
                    return Err(err(
                        PrivErrorCode::NotFound,
                        "That app login no longer exists on the database.",
                    ));
                }
                format!("ALTER ROLE \"{role}\" PASSWORD {};\n", lit(&pw))
            } else {
                provision_sql(role, db_name, &pw, &s.database)
            };
            s.psql(&s.database, &sql).map_err(|mut e| {
                e.message = scrub_secrets(
                    &e.message,
                    &scrub.iter().map(String::as_str).collect::<Vec<_>>(),
                );
                e
            })?;
            let vars = app_vars(
                env,
                &req.target,
                var_prefix,
                role,
                db_name,
                &pw,
                (pooler_host, *pooler_port),
            )?;
            Ok(DatabaseManageResult {
                vars,
                detail: if rotate {
                    format!("Rotated the password of {role}.")
                } else {
                    format!("Created database {db_name} and login {role}.")
                },
            })
        }
        DatabaseAction::DropApp {
            role,
            db_name,
            drop_database,
        } => {
            if !valid_pg_name(role) || !valid_pg_name(db_name) {
                return Err(err(PrivErrorCode::Invalid, "Invalid app database name."));
            }
            if role == &s.user || db_name == &s.database || role == POOLER_ROLE {
                return Err(err(
                    PrivErrorCode::Refused,
                    "The main database login cannot be removed here.",
                ));
            }
            s.psql(
                &s.database,
                &drop_sql(role, db_name, *drop_database, &s.user),
            )?;
            Ok(DatabaseManageResult {
                vars: Vec::new(),
                detail: if *drop_database {
                    format!("Deleted database {db_name} and login {role}.")
                } else {
                    format!("Removed login {role}; database {db_name} kept.")
                },
            })
        }
        DatabaseAction::SetExtension {
            database,
            extension,
            enable,
        } => {
            if !allowed_extension(extension) {
                return Err(err(
                    PrivErrorCode::Refused,
                    "That extension cannot be changed from opsbox.",
                ));
            }
            if !valid_db_identifier(database) {
                return Err(err(PrivErrorCode::Invalid, "Invalid database name."));
            }
            if *enable && extension == "pg_stat_statements" {
                let loaded = s.psql(
                    database,
                    "SELECT current_setting('shared_preload_libraries') LIKE '%pg_stat_statements%';\n",
                )?;
                if loaded.trim() != "t" {
                    return Err(err(
                        PrivErrorCode::Conflict,
                        "Redeploy this database once so slow-query tracking can load, then turn it on.",
                    ));
                }
            }
            let sql = if *enable {
                format!("CREATE EXTENSION IF NOT EXISTS \"{extension}\";\n")
            } else {
                format!("DROP EXTENSION IF EXISTS \"{extension}\";\n")
            };
            s.psql(database, &sql).map_err(|mut e| {
                if e.message.contains("is not available")
                    || e.message.contains("could not open extension control file")
                {
                    e.message = missing_extension_hint(extension);
                }
                e
            })?;
            Ok(DatabaseManageResult {
                vars: Vec::new(),
                detail: format!(
                    "{} {extension} in {database}.",
                    if *enable { "Enabled" } else { "Disabled" }
                ),
            })
        }
    }
}

// ------------------------------------------------------------------------------------ pooler

fn pooler_service(db_service: &str) -> String {
    format!("{db_service}-pooler")
}

pub fn pooler_database_with(
    env: &Env,
    req: &DatabasePoolerRequest,
) -> Result<DatabasePoolerResult, PrivError> {
    let t = &req.target;
    if !valid_service_name(&req.project) || !valid_docker_ref(&req.deploy_id) {
        return Err(err(PrivErrorCode::Invalid, "Invalid pooler parameters."));
    }
    if let Some(c) = &req.existing_container
        && !valid_docker_ref(c)
    {
        return Err(err(PrivErrorCode::Invalid, "Invalid pooler container."));
    }
    let socket = env.docker.socket_path();

    if !req.enable {
        check_target(t)?;
        if let Some(c) = &req.existing_container {
            block_on_fut(env.docker.force_stop_and_remove_container(c))?
                .map_err(|e| map_docker_err(e, socket))?;
        }
        // Remove the lookup login again when the database is up; a stopped database keeps it
        // harmlessly (connect-only, no data access) until the next enable.
        if let Ok(s) = Session::open(env, t) {
            let _ = s.psql("postgres", &pooler_remove_sql());
        }
        return Ok(DatabasePoolerResult {
            container_id: None,
            vars: Vec::new(),
        });
    }

    if !matches!(req.pool_mode.as_str(), "transaction" | "session")
        || !(10..=5000).contains(&req.max_client_conn)
        || !(1..=500).contains(&req.default_pool_size)
    {
        return Err(err(
            PrivErrorCode::Invalid,
            "Pool mode must be transaction or session, with sensible limits.",
        ));
    }
    if !valid_docker_image_ref(PGBOUNCER_IMAGE) {
        return Err(err(PrivErrorCode::Internal, "Invalid pooler image."));
    }
    let s = Session::open(env, t)?;
    let (opened, _) = open_deploy_vars(&env.secrets, &t.vars)?;
    let existing_pw = opened
        .iter()
        .find(|v| v.key == POOLER_PASSWORD_VAR)
        .map(|v| v.value.clone())
        .filter(|v| !v.is_empty());
    let pw = match &existing_pw {
        Some(p) => p.clone(),
        None => new_password()?,
    };
    s.psql("postgres", &pooler_setup_sql(&pw))?;

    if let Some(c) = &req.existing_container {
        block_on_fut(env.docker.force_stop_and_remove_container(c))?
            .map_err(|e| map_docker_err(e, socket))?;
    }
    super::capacity::check_image(env, PGBOUNCER_IMAGE)?;
    block_on_fut(env.docker.pull_image(PGBOUNCER_IMAGE))?.map_err(|e| map_docker_err(e, socket))?;

    let svc = pooler_service(&t.service_name);
    let env_strings = vec![
        format!("DB_HOST={}", t.service_name),
        "DB_PORT=5432".to_owned(),
        format!("DB_USER={POOLER_ROLE}"),
        format!("DB_PASSWORD={pw}"),
        "AUTH_TYPE=scram-sha-256".to_owned(),
        format!("AUTH_USER={POOLER_ROLE}"),
        format!("AUTH_QUERY=SELECT uname, phash FROM {POOLER_SCHEMA}.lookup($1)"),
        "AUTH_DBNAME=postgres".to_owned(),
        format!("POOL_MODE={}", req.pool_mode),
        format!("MAX_CLIENT_CONN={}", req.max_client_conn),
        format!("DEFAULT_POOL_SIZE={}", req.default_pool_size),
        format!("LISTEN_PORT={POOLER_PORT}"),
    ];
    let container_name = format!("opsbox-{svc}-{}", short_deploy_suffix(&req.deploy_id));
    let network = format!("opsbox-{}", req.project);
    let spec = TemplateContainerSpec {
        project: &req.project,
        service: &svc,
        template_id: "pgbouncer",
        deploy_id: &req.deploy_id,
        container_name: &container_name,
        network_name: &network,
        image_ref: PGBOUNCER_IMAGE,
        env: &env_strings,
        cmd: &[],
        volume_binds: &[],
        container_port: POOLER_PORT,
        port_bind: PortBindMode::InternalOnly,
        memory_limit_mb: Some(128),
        cpu_limit_millis: None,
    };
    let scrub = [pw.as_str()];
    let (cid, _, _) = block_on_fut(env.docker.create_and_start_template_container(&spec))?
        .map_err(|mut e| {
            if let DockerError::Internal(ref mut m) = e {
                *m = scrub_secrets(m, &scrub);
            }
            map_docker_err(e, socket)
        })?;

    // PgBouncer starts in well under a second; confirm it stays up.
    std::thread::sleep(Duration::from_millis(1500));
    let up = matches!(
        block_on_fut(env.docker.is_container_running(&cid))?,
        Ok(true)
    );
    if !up {
        let tail = block_on_fut(env.docker.logs(&cid, 10))
            .ok()
            .and_then(Result::ok)
            .map(|l| {
                l.into_iter()
                    .map(|x| scrub_secrets(&x.text, &scrub))
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .unwrap_or_default();
        let _ = block_on_fut(env.docker.force_stop_and_remove_container(&cid));
        return Err(err(
            PrivErrorCode::Internal,
            format!("The connection pooler did not stay running. {tail}"),
        ));
    }

    let vars = if existing_pw.is_some() {
        Vec::new()
    } else {
        vec![seal_var(
            env,
            &t.service_id,
            POOLER_PASSWORD_VAR,
            pw.clone(),
            true,
        )?]
    };
    Ok(DatabasePoolerResult {
        container_id: Some(cid),
        vars,
    })
}

// ------------------------------------------------------------------------------------ upgrade

/// Names and table counts used to check a restored cluster against the original.
fn fingerprint(s: &Session<'_>) -> Result<Vec<String>, PrivError> {
    let dbs = s.psql(
        &s.database,
        "SELECT datname FROM pg_database WHERE NOT datistemplate ORDER BY 1;\n",
    )?;
    let mut out = Vec::new();
    for db in dbs.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if !valid_db_identifier(db) {
            continue;
        }
        let n = s.psql(
            db,
            "SELECT count(*) FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog','information_schema');\n",
        )?;
        out.push(format!("{db}={}", n.trim()));
    }
    let roles = s.psql(
        &s.database,
        "SELECT rolname FROM pg_roles WHERE rolname !~ '^pg_' ORDER BY 1;\n",
    )?;
    out.push(format!(
        "roles={}",
        roles.lines().map(str::trim).collect::<Vec<_>>().join(",")
    ));
    Ok(out)
}

/// Restore errors that are normal when loading `pg_dumpall` into a freshly initialised cluster.
fn restore_error_is_expected(line: &str) -> bool {
    line.contains("already exists")
}

pub fn upgrade_database_with(
    env: &Env,
    req: &DatabaseUpgradeRequest,
) -> Result<DatabaseUpgradeResult, PrivError> {
    let t = &req.target;
    check_target(t)?;
    let tmpl = get_template(&t.template_id)
        .ok_or_else(|| err(PrivErrorCode::Invalid, "Unknown database template."))?;
    let (Some(from), Some(to)) = (pg_major(&req.from_version), pg_major(&req.to_version)) else {
        return Err(err(PrivErrorCode::Invalid, "Unknown PostgreSQL version."));
    };
    if to <= from {
        return Err(err(
            PrivErrorCode::Invalid,
            "Choose a newer major version. Moving back is not supported.",
        ));
    }
    if !tmpl.versions.iter().any(|v| v == &req.to_version)
        || req.image_ref != format!("{}:{}", tmpl.image, req.to_version)
    {
        return Err(err(
            PrivErrorCode::Invalid,
            "That version is not offered for this database.",
        ));
    }
    if !valid_service_name(&req.project)
        || !valid_docker_ref(&req.deploy_id)
        || !valid_docker_ref(&req.old_volume)
        || !valid_docker_ref(&req.new_volume)
        || req.old_volume == req.new_volume
    {
        return Err(err(PrivErrorCode::Invalid, "Invalid upgrade parameters."));
    }
    match &req.snapshot.target {
        BackupRunTarget::Database { service_id, .. } if service_id == &t.service_id => {}
        _ => {
            return Err(err(
                PrivErrorCode::Invalid,
                "The upgrade needs a backup of this database first.",
            ));
        }
    }
    let s = Session::open(env, t)?;
    let socket = env.docker.socket_path();
    let mut log = Vec::new();

    // 1. Snapshot. Nothing else happens unless this succeeds.
    let snap = super::backup::run_backup_with(env, req.snapshot.clone()).map_err(|e| {
        err(
            PrivErrorCode::Conflict,
            format!(
                "Upgrade cancelled: the safety backup failed ({}).",
                e.message
            ),
        )
    })?;
    log.push(format!("Safety backup {} saved.", snap.short_id));

    // 2. Dump every database and role to a private file.
    std::fs::create_dir_all(&env.backup_cache_dir).map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Could not prepare staging directory: {e}"),
        )
    })?;
    let _ = std::fs::set_permissions(
        &env.backup_cache_dir,
        std::fs::Permissions::from_mode(0o700),
    );
    let dump_file = tempfile::Builder::new()
        .prefix("opsbox-pgdump-")
        .tempfile_in(&env.backup_cache_dir)
        .map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Could not create dump file: {e}"),
            )
        })?;
    let before = fingerprint(&s)?;
    let dump_req = DockerExecRequest {
        id: t.container_id.clone(),
        cmd: vec!["pg_dumpall".into(), "-U".into(), s.user.clone()],
        user: None,
        env: vec![format!("PGPASSWORD={}", s.password)],
        working_dir: None,
        stdin: None,
    };
    let dump_path = dump_file.path().to_owned();
    let status = block_on_fut(async {
        let f = tokio::fs::File::create(&dump_path).await?;
        env.docker
            .exec_to_writer(dump_req, f, DUMP_TIMEOUT)
            .await
            .map_err(std::io::Error::other)
    })?
    .map_err(|e| err(PrivErrorCode::Internal, format!("Dump failed: {e}")))?;
    if status.exit_code != 0 || status.bytes_written == 0 {
        return Err(err(
            PrivErrorCode::Internal,
            format!(
                "Dump failed, nothing was changed. {}",
                scrub_secrets(status.stderr.trim(), &s.scrub_refs())
            ),
        ));
    }
    log.push(format!("Exported {} bytes.", status.bytes_written));

    // 3. Stop the old container (kept, so it can be started again), start the new version.
    block_on_fut(
        env.docker
            .container_action(&t.container_id, DockerContainerAction::Stop),
    )?
    .map_err(|e| map_docker_err(e, socket))?;
    let rollback = |new_cid: Option<&str>, why: String| -> PrivError {
        if let Some(c) = new_cid {
            let _ = block_on_fut(env.docker.force_stop_and_remove_container(c));
        }
        let _ = block_on_fut(env.docker.remove_volume(&req.new_volume));
        let restarted = block_on_fut(
            env.docker
                .container_action(&t.container_id, DockerContainerAction::Start),
        )
        .is_ok_and(|r| r.is_ok());
        err(
            PrivErrorCode::Conflict,
            format!(
                "The upgrade did not work and was rolled back{}. {why}",
                if restarted {
                    "; your database is running as before"
                } else {
                    "; start your database from the Containers page"
                }
            ),
        )
    };

    let started = start_template_with(
        env,
        &TemplateStartRequest {
            project: req.project.clone(),
            service: t.service_name.clone(),
            deploy_id: req.deploy_id.clone(),
            template_id: t.template_id.clone(),
            version: req.to_version.clone(),
            image_ref: req.image_ref.clone(),
            internal_only: req.internal_only,
            public_port: req.public_port,
            confirm_public_port: req.confirm_public_port,
            vars: t.vars.clone(),
            ref_sources: Vec::new(),
            memory_limit_mb: req.memory_limit_mb,
            cpu_limit_millis: req.cpu_limit_millis,
            data_volume: Some(req.new_volume.clone()),
        },
    )
    .map_err(|e| rollback(None, e.message))?;
    let new_cid = started.container_id.clone();

    // 4. Restore the dump into the new server and check what arrived.
    let restore_req = DockerExecRequest {
        id: new_cid.clone(),
        cmd: vec![
            "psql".into(),
            "-X".into(),
            "-q".into(),
            "-U".into(),
            s.user.clone(),
            "-d".into(),
            "postgres".into(),
        ],
        user: None,
        env: vec![format!("PGPASSWORD={}", s.password)],
        working_dir: None,
        stdin: None,
    };
    let restored = block_on_fut(async {
        let f = tokio::fs::File::open(&dump_path).await?;
        env.docker
            .exec_from_reader(restore_req, f, RESTORE_TIMEOUT)
            .await
            .map_err(std::io::Error::other)
    })?
    .map_err(|e| rollback(Some(&new_cid), format!("Restore failed: {e}")))?;
    let bad: Vec<&str> = restored
        .stderr
        .lines()
        .filter(|l| l.contains("ERROR:") && !restore_error_is_expected(l))
        .collect();
    if !bad.is_empty() {
        let first = scrub_secrets(bad[0], &s.scrub_refs());
        return Err(rollback(
            Some(&new_cid),
            format!("Restore reported: {first}"),
        ));
    }
    let new_session = Session {
        env,
        container: &new_cid,
        user: s.user.clone(),
        password: s.password.clone(),
        database: s.database.clone(),
        scrub: s.scrub.clone(),
    };
    let after = fingerprint(&new_session).map_err(|e| rollback(Some(&new_cid), e.message))?;
    if before != after {
        return Err(rollback(
            Some(&new_cid),
            "The restored data did not match the original, so nothing was switched.".into(),
        ));
    }
    log.push(format!(
        "Restored and checked {} databases.",
        after.len().saturating_sub(1)
    ));

    // 5. Success: retire the old container. Its volume stays until the user deletes it.
    let _ = block_on_fut(env.docker.force_stop_and_remove_container(&t.container_id));
    let databases = u32::try_from(after.len().saturating_sub(1)).unwrap_or(u32::MAX);
    Ok(DatabaseUpgradeResult {
        container_id: new_cid,
        host_port: started.host_port,
        snapshot_id: snap.snapshot_id,
        old_volume: req.old_volume.clone(),
        new_volume: req.new_volume.clone(),
        databases,
        log,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provision_sql_isolates_each_app_database() {
        let sql = provision_sql("app_web", "app_web", "Pw12345678", "app");
        assert!(sql.contains("CREATE ROLE \"app_web\" LOGIN PASSWORD 'Pw12345678' NOSUPERUSER"));
        assert!(sql.contains("REVOKE ALL ON DATABASE \"app_web\" FROM PUBLIC"));
        assert!(sql.contains("REVOKE CONNECT ON DATABASE \"app\" FROM PUBLIC"));
        assert!(sql.contains("\\connect \"app_web\""));
        assert!(sql.contains("ALTER SCHEMA public OWNER TO \"app_web\""));
    }

    #[test]
    fn drop_sql_keeps_or_deletes_data() {
        let keep = drop_sql("app_x", "app_x", false, "app");
        assert!(keep.contains("REASSIGN OWNED BY \"app_x\" TO \"app\""));
        assert!(!keep.contains("DROP DATABASE"));
        let gone = drop_sql("app_x", "app_x", true, "app");
        assert!(gone.contains("DROP DATABASE IF EXISTS \"app_x\" WITH (FORCE)"));
    }

    #[test]
    fn inspect_json_parses_into_typed_results() {
        let doc = serde_json::json!({
            "version": "16.4", "database": "app",
            "databases": [{"name": "app", "owner": "app", "size": 8_000_000}],
            "users": [{"name": "app", "login": true, "super": true, "conns": 2, "dbs": ["app"]}],
            "grants": [{"role": "app_x", "db": "app_x", "connect": true, "create": true, "temp": true}],
            "extensions": [{"name": "vector", "installed": false, "installed_version": null, "default_version": "0.7.0", "comment": "vectors"}],
            "tables": [{"schema": "public", "table": "t", "bytes": 16384, "rows": 3}],
            "conns_now": 4, "conns_max": 100, "stat_loaded": true, "stat_enabled": false
        });
        let r = parse_inspect(&doc);
        assert_eq!(r.server_version, "16.4");
        assert_eq!(r.databases[0].size_bytes, 8_000_000);
        assert!(r.users[0].superuser);
        assert_eq!(r.tables[0].rows_estimate, 3);
        assert_eq!((r.connections_current, r.connections_max), (4, 100));
        assert!(r.stat_statements_loaded && !r.stat_statements_enabled);
        let slow = parse_slow(
            &serde_json::json!([{"query": "select $1", "calls": 5, "mean_ms": 1.5, "total_ms": 7.5, "rows": 5}]),
        );
        assert_eq!(slow[0].calls, 5);
    }

    #[test]
    fn expected_restore_errors_are_only_already_exists() {
        assert!(restore_error_is_expected(
            "ERROR:  role \"app\" already exists"
        ));
        assert!(!restore_error_is_expected(
            "ERROR:  could not open extension control file"
        ));
    }
}
