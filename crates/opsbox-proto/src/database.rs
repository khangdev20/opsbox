//! Typed requests and answers for managing a PostgreSQL database service (Databases part B).
//!
//! `serve` only describes what it wants (sealed credentials in, sealed credentials out);
//! `priv` opens the sealed administrator login, runs `psql` inside the container and
//! returns structured results. No SQL text and no plaintext password ever crosses this
//! boundary.

use serde::{Deserialize, Serialize};

use crate::backup::BackupRunSpec;
use crate::deploy::DeployEnvSpec;

/// Reserved variable-name prefix for per-app credentials stored on a database service
/// (`OPSBOX_APP_<APP>_DATABASE_URL`, …). These never reach the database container's own
/// environment.
pub const APP_VAR_PREFIX: &str = "OPSBOX_APP_";
/// Reserved prefix for opsbox-internal database service variables (for example the pooler
/// login). Also kept out of the container environment.
pub const INTERNAL_VAR_PREFIX: &str = "OPSBOX_";

/// Extensions that may be enabled from the panel. Anything else is refused in `priv`.
pub const ALLOWED_EXTENSIONS: &[&str] = &[
    "vector",
    "postgis",
    "pg_stat_statements",
    "pgcrypto",
    "uuid-ossp",
    "pg_trgm",
    "citext",
    "hstore",
];

/// Lowercase letters, digits and `_` only, starting with a letter. Used for every database
/// and role name opsbox creates, so identifiers can be quoted without escaping.
pub fn valid_pg_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 63
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

/// Variable-name prefix holding one app's credentials on the database service:
/// `web-api` becomes `OPSBOX_APP_WEB_API_`.
pub fn app_var_prefix(app_service: &str) -> String {
    format!(
        "{APP_VAR_PREFIX}{}_",
        app_service.to_ascii_uppercase().replace('-', "_")
    )
}

/// Database and login name for an app: `web-api` becomes `app_web_api`. Longer than 63
/// bytes (so unusable) for service names over 59 characters; callers check
/// [`valid_pg_name`].
pub fn app_pg_name(app_service: &str) -> String {
    format!("app_{}", app_service.replace('-', "_"))
}

/// The connection variables stored per app: `(stored suffix, name the app sees, secret,
/// pooled)`. Direct and pooled entries share the same `(name the app sees)` so an app is
/// pointed at one or the other.
pub const APP_CONNECTION_VARS: &[(&str, &str, bool, bool)] = &[
    ("DATABASE_URL", "DATABASE_URL", true, false),
    ("PGHOST", "PGHOST", false, false),
    ("PGPORT", "PGPORT", false, false),
    ("PGUSER", "PGUSER", false, false),
    ("PGPASSWORD", "PGPASSWORD", true, false),
    ("PGDATABASE", "PGDATABASE", false, false),
    ("POOLED_DATABASE_URL", "DATABASE_URL", true, true),
    ("POOLED_PGHOST", "PGHOST", false, true),
    ("POOLED_PGPORT", "PGPORT", false, true),
];

/// `(name the app sees, stored suffix)` pairs for linking an app directly or through the
/// pooler. Through the pooler, the pooled entry replaces the direct one of the same name.
pub fn app_connect_pairs(pooled: bool) -> Vec<(&'static str, &'static str)> {
    let mut out: Vec<(&'static str, &'static str)> = Vec::new();
    for (suffix, env, _, is_pooled) in APP_CONNECTION_VARS {
        if *is_pooled == pooled {
            out.push((env, suffix));
        }
    }
    if pooled {
        for (suffix, env, _, is_pooled) in APP_CONNECTION_VARS {
            if !is_pooled && !out.iter().any(|(e, _)| e == env) {
                out.push((env, suffix));
            }
        }
    }
    out
}

/// Whether `name` is an extension opsbox is willing to enable.
pub fn allowed_extension(name: &str) -> bool {
    ALLOWED_EXTENSIONS.contains(&name)
}

/// The managed database a request is about: its container and sealed variables. `priv` opens
/// `vars` itself and refuses any seal context that does not belong to `service_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbTarget {
    pub service_id: String,
    pub service_name: String,
    pub template_id: String,
    pub container_id: String,
    pub vars: Vec<DeployEnvSpec>,
}

/// One database on the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbDatabaseInfo {
    pub name: String,
    pub owner: String,
    pub size_bytes: u64,
}

/// One login or group role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbUserInfo {
    pub name: String,
    pub can_login: bool,
    pub superuser: bool,
    pub connections: u32,
    /// Databases this role may connect to.
    pub databases: Vec<String>,
}

/// What one role may do in one database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbGrantInfo {
    pub role: String,
    pub database: String,
    pub connect: bool,
    pub create: bool,
    pub temp: bool,
}

/// An extension the server offers, and whether it is on in the inspected database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbExtensionInfo {
    pub name: String,
    pub installed: bool,
    pub installed_version: Option<String>,
    pub default_version: Option<String>,
    pub comment: String,
}

/// Disk use of one table (data, indexes and TOAST).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbTableSize {
    pub schema: String,
    pub table: String,
    pub total_bytes: u64,
    pub rows_estimate: i64,
}

/// One normalised statement from `pg_stat_statements`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbSlowQuery {
    pub query: String,
    pub calls: u64,
    pub mean_ms: f64,
    pub total_ms: f64,
    pub rows: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseInspectRequest {
    pub target: DbTarget,
    /// Database to read tables, extensions and slow queries from (default: the admin database).
    #[serde(default)]
    pub database: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatabaseInspectResult {
    pub server_version: String,
    /// The database the table list, extension state and slow queries belong to.
    pub database: String,
    pub databases: Vec<DbDatabaseInfo>,
    pub users: Vec<DbUserInfo>,
    pub grants: Vec<DbGrantInfo>,
    pub extensions: Vec<DbExtensionInfo>,
    pub tables: Vec<DbTableSize>,
    pub connections_current: u32,
    pub connections_max: u32,
    /// `pg_stat_statements` is loaded into the server (needs a one-time restart on old databases).
    pub stat_statements_loaded: bool,
    /// `pg_stat_statements` is enabled in the inspected database.
    pub stat_statements_enabled: bool,
    pub slow_queries: Vec<DbSlowQuery>,
}

/// Something to change on a database server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DatabaseAction {
    /// Create (or re-use) one database and one least-privilege login for an app and return the
    /// sealed connection variables to store on the database service.
    ProvisionApp {
        app_service: String,
        db_name: String,
        role: String,
        /// `OPSBOX_APP_<APP>_`.
        var_prefix: String,
        /// Internal name apps use for a connection pooler, when the pooler is offered.
        pooler_host: String,
        pooler_port: u16,
    },
    /// Give one app login a new password. Returns the same variables as `ProvisionApp`.
    RotateApp {
        app_service: String,
        db_name: String,
        role: String,
        var_prefix: String,
        pooler_host: String,
        pooler_port: u16,
    },
    /// Remove an app login, and with `drop_database` its database as well.
    DropApp {
        role: String,
        db_name: String,
        drop_database: bool,
    },
    /// Turn an allowed extension on or off in one database.
    SetExtension {
        database: String,
        extension: String,
        enable: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseManageRequest {
    pub target: DbTarget,
    pub action: DatabaseAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseManageResult {
    /// Sealed variables to store on the database service (empty for actions without any).
    pub vars: Vec<DeployEnvSpec>,
    /// One sentence for the audit log and the UI.
    pub detail: String,
}

/// Start or remove the PgBouncer container in front of a database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabasePoolerRequest {
    pub target: DbTarget,
    pub project: String,
    pub enable: bool,
    /// `transaction` (default) or `session`.
    pub pool_mode: String,
    pub max_client_conn: u32,
    pub default_pool_size: u32,
    /// Pooler container to replace or remove, if one is recorded.
    #[serde(default)]
    pub existing_container: Option<String>,
    pub deploy_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabasePoolerResult {
    pub container_id: Option<String>,
    /// The pooler login, sealed for the database service (empty when disabled).
    pub vars: Vec<DeployEnvSpec>,
}

/// Move a database to a newer major version: snapshot, dump, new volume, restore, verify.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseUpgradeRequest {
    pub target: DbTarget,
    pub project: String,
    pub deploy_id: String,
    pub from_version: String,
    pub to_version: String,
    pub image_ref: String,
    pub internal_only: bool,
    #[serde(default)]
    pub public_port: bool,
    #[serde(default)]
    pub confirm_public_port: bool,
    pub memory_limit_mb: Option<i64>,
    pub cpu_limit_millis: Option<i64>,
    /// Volume holding the current data (left untouched).
    pub old_volume: String,
    /// Fresh volume for the new major version.
    pub new_volume: String,
    /// Backup that must succeed before anything else happens.
    pub snapshot: BackupRunSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseUpgradeResult {
    pub container_id: String,
    pub host_port: u16,
    pub snapshot_id: String,
    pub old_volume: String,
    pub new_volume: String,
    pub databases: u32,
    pub log: Vec<String>,
}

/// Major version of a PostgreSQL image tag such as `16.4`, `pg17` or `17-3.5`.
pub fn pg_major(version: &str) -> Option<u32> {
    let v = version.strip_prefix("pg").unwrap_or(version);
    let digits: String = v.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok().filter(|m| (9..=99).contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_strictly_lowercase_identifiers() {
        assert!(valid_pg_name("app_web_api"));
        assert!(!valid_pg_name("App"));
        assert!(!valid_pg_name("1app"));
        assert!(!valid_pg_name("a-b"));
        assert!(!valid_pg_name("a\"; drop"));
        assert!(!valid_pg_name(""));
        assert!(!valid_pg_name(&"a".repeat(64)));
    }

    #[test]
    fn majors_parse_from_image_tags() {
        assert_eq!(pg_major("16.4"), Some(16));
        assert_eq!(pg_major("pg17"), Some(17));
        assert_eq!(pg_major("17-3.5"), Some(17));
        assert_eq!(pg_major("latest"), None);
    }

    #[test]
    fn app_names_and_pairs_are_derived_from_the_service_name() {
        assert_eq!(app_var_prefix("web-api"), "OPSBOX_APP_WEB_API_");
        assert_eq!(app_pg_name("web-api"), "app_web_api");
        let direct = app_connect_pairs(false);
        assert!(direct.contains(&("DATABASE_URL", "DATABASE_URL")));
        assert!(direct.contains(&("PGHOST", "PGHOST")));
        let pooled = app_connect_pairs(true);
        assert!(pooled.contains(&("DATABASE_URL", "POOLED_DATABASE_URL")));
        assert!(pooled.contains(&("PGPORT", "POOLED_PGPORT")));
        assert!(!pooled.contains(&("DATABASE_URL", "DATABASE_URL")));
        assert!(pooled.contains(&("PGUSER", "PGUSER")));
    }

    #[test]
    fn only_listed_extensions_are_allowed() {
        assert!(allowed_extension("vector"));
        assert!(!allowed_extension("plpython3u"));
        assert!(!allowed_extension("file_fdw"));
    }
}
