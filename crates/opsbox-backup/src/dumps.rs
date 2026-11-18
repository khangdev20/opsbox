//! Logical database dump command builders for template databases
//! (`pg_dump -Fc`, `mysqldump --single-transaction`, `redis-cli --rdb`, `mongodump --archive`).
//!
//! Credentials travel solely inside container exec environment variables (never argv).

use opsbox_proto::{BackupDbKind, valid_db_identifier, valid_service_name};

use crate::BackupError;

/// Command, environment, and stream metadata for dumping a running template database
/// container directly to stdout so `priv` can pipe it to `restic backup --stdin`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseDumpSpec {
    pub cmd: Vec<String>,
    pub env: Vec<String>,
    pub stdin_filename: String,
    pub scrub_secrets: Vec<String>,
}

fn find_var<'a>(vars: &'a [(String, String)], key: &str) -> Option<&'a str> {
    vars.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn collect_scrub_secrets(vars: &[(String, String)]) -> Vec<String> {
    vars.iter()
        .filter(|(k, v)| {
            !v.is_empty()
                && (k.contains("PASSWORD")
                    || k.contains("SECRET")
                    || k.contains("URL")
                    || k.contains("URI"))
        })
        .map(|(_, v)| v.clone())
        .collect()
}

fn pg_creds(vars: &[(String, String)]) -> Result<(&str, &str, &str), BackupError> {
    let user = find_var(vars, "POSTGRES_USER").unwrap_or("app");
    let db = find_var(vars, "POSTGRES_DB").unwrap_or("app");
    let pw = find_var(vars, "POSTGRES_PASSWORD").unwrap_or("");
    if !valid_db_identifier(user) || !valid_db_identifier(db) {
        return Err(BackupError::Validation(
            "Invalid PostgreSQL user or database identifier.".into(),
        ));
    }
    Ok((user, db, pw))
}

fn mysql_creds(vars: &[(String, String)]) -> Result<(&str, &str, &str), BackupError> {
    let db = find_var(vars, "MYSQL_DATABASE").unwrap_or("app");
    if !valid_db_identifier(db) {
        return Err(BackupError::Validation(
            "Invalid MySQL database identifier.".into(),
        ));
    }
    let (user, pw) = if let Some(root_pw) = find_var(vars, "MYSQL_ROOT_PASSWORD") {
        ("root", root_pw)
    } else {
        (
            find_var(vars, "MYSQL_USER").unwrap_or("app"),
            find_var(vars, "MYSQL_PASSWORD").unwrap_or(""),
        )
    };
    if !valid_db_identifier(user) {
        return Err(BackupError::Validation(
            "Invalid MySQL user identifier.".into(),
        ));
    }
    Ok((user, db, pw))
}

fn mariadb_creds(vars: &[(String, String)]) -> Result<(&str, &str, &str), BackupError> {
    let db = find_var(vars, "MARIADB_DATABASE").unwrap_or("app");
    if !valid_db_identifier(db) {
        return Err(BackupError::Validation(
            "Invalid MariaDB database identifier.".into(),
        ));
    }
    let (user, pw) = if let Some(root_pw) = find_var(vars, "MARIADB_ROOT_PASSWORD") {
        ("root", root_pw)
    } else {
        (
            find_var(vars, "MARIADB_USER").unwrap_or("app"),
            find_var(vars, "MARIADB_PASSWORD").unwrap_or(""),
        )
    };
    if !valid_db_identifier(user) {
        return Err(BackupError::Validation(
            "Invalid MariaDB user identifier.".into(),
        ));
    }
    Ok((user, db, pw))
}

fn mongo_creds(vars: &[(String, String)]) -> Result<(&str, &str), BackupError> {
    let user = find_var(vars, "MONGO_INITDB_ROOT_USERNAME").unwrap_or("app");
    let pw = find_var(vars, "MONGO_INITDB_ROOT_PASSWORD").unwrap_or("");
    if !valid_db_identifier(user) {
        return Err(BackupError::Validation(
            "Invalid MongoDB user identifier.".into(),
        ));
    }
    Ok((user, pw))
}

/// Default stdin filename used inside restic snapshots for a logical database dump.
pub fn default_stdin_filename(kind: BackupDbKind, service_name: &str) -> String {
    match kind {
        BackupDbKind::Postgres => format!("{service_name}.pg_dump.fc"),
        BackupDbKind::Mysql | BackupDbKind::Mariadb => format!("{service_name}.sql"),
        BackupDbKind::Redis => format!("{service_name}.rdb"),
        BackupDbKind::Mongo => format!("{service_name}.mongodump.archive"),
    }
}

/// Build the `docker exec` command and environment for streaming a logical database dump.
pub fn build_dump_spec(
    kind: BackupDbKind,
    service_name: &str,
    vars: &[(String, String)],
) -> Result<DatabaseDumpSpec, BackupError> {
    if !valid_service_name(service_name) {
        return Err(BackupError::Validation(
            "Invalid database service name.".into(),
        ));
    }

    let mut scrub_secrets = collect_scrub_secrets(vars);
    let stdin_filename = default_stdin_filename(kind, service_name);

    match kind {
        BackupDbKind::Postgres => {
            let (user, db, pw) = pg_creds(vars)?;
            if !pw.is_empty() {
                scrub_secrets.push(pw.to_owned());
            }
            Ok(DatabaseDumpSpec {
                cmd: vec![
                    "pg_dump".into(),
                    "-Fc".into(),
                    "-U".into(),
                    user.to_owned(),
                    "-d".into(),
                    db.to_owned(),
                ],
                env: vec![format!("PGPASSWORD={pw}")],
                stdin_filename,
                scrub_secrets,
            })
        }
        BackupDbKind::Mysql => {
            let (user, db, pw) = mysql_creds(vars)?;
            if !pw.is_empty() {
                scrub_secrets.push(pw.to_owned());
            }
            Ok(DatabaseDumpSpec {
                cmd: vec![
                    "mysqldump".into(),
                    "--single-transaction".into(),
                    "--routines".into(),
                    "--triggers".into(),
                    "--quick".into(),
                    "-h".into(),
                    "127.0.0.1".into(),
                    format!("-u{user}"),
                    db.to_owned(),
                ],
                env: vec![format!("MYSQL_PWD={pw}")],
                stdin_filename,
                scrub_secrets,
            })
        }
        BackupDbKind::Mariadb => {
            let (user, db, pw) = mariadb_creds(vars)?;
            if !pw.is_empty() {
                scrub_secrets.push(pw.to_owned());
            }
            Ok(DatabaseDumpSpec {
                cmd: vec![
                    "sh".into(),
                    "-c".into(),
                    "exec \"$(command -v mariadb-dump || command -v mysqldump)\" --single-transaction --routines --triggers --quick -h 127.0.0.1 -u\"$1\" \"$2\"".into(),
                    "_".into(),
                    user.to_owned(),
                    db.to_owned(),
                ],
                env: vec![format!("MYSQL_PWD={pw}")],
                stdin_filename,
                scrub_secrets,
            })
        }
        BackupDbKind::Redis => {
            let pw = find_var(vars, "REDIS_PASSWORD").unwrap_or("");
            if !pw.is_empty() {
                scrub_secrets.push(pw.to_owned());
            }
            Ok(DatabaseDumpSpec {
                cmd: vec![
                    "redis-cli".into(),
                    "--no-auth-warning".into(),
                    "--rdb".into(),
                    "-".into(),
                ],
                env: if pw.is_empty() {
                    Vec::new()
                } else {
                    vec![format!("REDISCLI_AUTH={pw}")]
                },
                stdin_filename,
                scrub_secrets,
            })
        }
        BackupDbKind::Mongo => {
            let (user, pw) = mongo_creds(vars)?;
            if !pw.is_empty() {
                scrub_secrets.push(pw.to_owned());
            }
            Ok(DatabaseDumpSpec {
                cmd: vec![
                    "sh".into(),
                    "-c".into(),
                    "exec mongodump --archive --authenticationDatabase=admin --host=127.0.0.1 --port=27017 --username=\"$OPSBOX_MONGO_USER\" --password=\"$OPSBOX_MONGO_PASS\"".into(),
                ],
                env: vec![
                    format!("OPSBOX_MONGO_USER={user}"),
                    format!("OPSBOX_MONGO_PASS={pw}"),
                ],
                stdin_filename,
                scrub_secrets,
            })
        }
    }
}

/// Strategy and credentials for restoring a logical database dump into a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseRestoreMethod {
    StdinExec { cmd: Vec<String>, env: Vec<String> },
    RedisRdbVolume,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseRestoreSpec {
    pub method: DatabaseRestoreMethod,
    pub scrub_secrets: Vec<String>,
}

/// Command and environment for verifying a restored database accepts queries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseSanitySpec {
    pub cmd: Vec<String>,
    pub env: Vec<String>,
    pub scrub_secrets: Vec<String>,
}

/// Build the restore specification for a target database service container.
pub fn build_restore_spec(
    kind: BackupDbKind,
    service_name: &str,
    vars: &[(String, String)],
) -> Result<DatabaseRestoreSpec, BackupError> {
    if !valid_service_name(service_name) {
        return Err(BackupError::Validation(
            "Invalid database service name.".into(),
        ));
    }
    let scrub_secrets = collect_scrub_secrets(vars);
    let method = match kind {
        BackupDbKind::Postgres => {
            let (user, db, pw) = pg_creds(vars)?;
            DatabaseRestoreMethod::StdinExec {
                cmd: vec![
                    "pg_restore".into(),
                    "--clean".into(),
                    "--if-exists".into(),
                    "--no-owner".into(),
                    "--no-privileges".into(),
                    "-U".into(),
                    user.to_owned(),
                    "-d".into(),
                    db.to_owned(),
                ],
                env: vec![format!("PGPASSWORD={pw}")],
            }
        }
        BackupDbKind::Mysql => {
            let (user, db, pw) = mysql_creds(vars)?;
            DatabaseRestoreMethod::StdinExec {
                cmd: vec![
                    "mysql".into(),
                    "-h".into(),
                    "127.0.0.1".into(),
                    format!("-u{user}"),
                    db.to_owned(),
                ],
                env: vec![format!("MYSQL_PWD={pw}")],
            }
        }
        BackupDbKind::Mariadb => {
            let (user, db, pw) = mariadb_creds(vars)?;
            DatabaseRestoreMethod::StdinExec {
                cmd: vec![
                    "sh".into(),
                    "-c".into(),
                    "exec \"$(command -v mariadb || command -v mysql)\" -h 127.0.0.1 -u\"$1\" \"$2\"".into(),
                    "_".into(),
                    user.to_owned(),
                    db.to_owned(),
                ],
                env: vec![format!("MYSQL_PWD={pw}")],
            }
        }
        BackupDbKind::Redis => DatabaseRestoreMethod::RedisRdbVolume,
        BackupDbKind::Mongo => {
            let (user, pw) = mongo_creds(vars)?;
            DatabaseRestoreMethod::StdinExec {
                cmd: vec![
                    "sh".into(),
                    "-c".into(),
                    "exec mongorestore --archive --drop --authenticationDatabase=admin --host=127.0.0.1 --port=27017 --username=\"$OPSBOX_MONGO_USER\" --password=\"$OPSBOX_MONGO_PASS\"".into(),
                ],
                env: vec![
                    format!("OPSBOX_MONGO_USER={user}"),
                    format!("OPSBOX_MONGO_PASS={pw}"),
                ],
            }
        }
    };
    Ok(DatabaseRestoreSpec {
        method,
        scrub_secrets,
    })
}

/// Build the `docker exec` command and environment for a live database sanity query.
pub fn build_sanity_check_spec(
    kind: BackupDbKind,
    vars: &[(String, String)],
) -> Result<DatabaseSanitySpec, BackupError> {
    let scrub_secrets = collect_scrub_secrets(vars);
    match kind {
        BackupDbKind::Postgres => {
            let (user, db, pw) = pg_creds(vars)?;
            Ok(DatabaseSanitySpec {
                cmd: vec![
                    "psql".into(),
                    "-U".into(),
                    user.to_owned(),
                    "-d".into(),
                    db.to_owned(),
                    "-tAc".into(),
                    "SELECT count(*) FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog','information_schema');".into(),
                ],
                env: vec![format!("PGPASSWORD={pw}")],
                scrub_secrets,
            })
        }
        BackupDbKind::Mysql => {
            let (user, db, pw) = mysql_creds(vars)?;
            Ok(DatabaseSanitySpec {
                cmd: vec![
                    "mysql".into(),
                    "-h".into(),
                    "127.0.0.1".into(),
                    format!("-u{user}"),
                    "-NBe".into(),
                    "SHOW TABLES".into(),
                    db.to_owned(),
                ],
                env: vec![format!("MYSQL_PWD={pw}")],
                scrub_secrets,
            })
        }
        BackupDbKind::Mariadb => {
            let (user, db, pw) = mariadb_creds(vars)?;
            Ok(DatabaseSanitySpec {
                cmd: vec![
                    "sh".into(),
                    "-c".into(),
                    "exec \"$(command -v mariadb || command -v mysql)\" -h 127.0.0.1 -u\"$1\" -NBe \"SHOW TABLES\" \"$2\"".into(),
                    "_".into(),
                    user.to_owned(),
                    db.to_owned(),
                ],
                env: vec![format!("MYSQL_PWD={pw}")],
                scrub_secrets,
            })
        }
        BackupDbKind::Redis => {
            let pw = find_var(vars, "REDIS_PASSWORD").unwrap_or("");
            Ok(DatabaseSanitySpec {
                cmd: vec![
                    "sh".into(),
                    "-c".into(),
                    "test \"$(redis-cli --no-auth-warning PING)\" = \"PONG\" && redis-cli --no-auth-warning DBSIZE".into(),
                ],
                env: if pw.is_empty() {
                    Vec::new()
                } else {
                    vec![format!("REDISCLI_AUTH={pw}")]
                },
                scrub_secrets,
            })
        }
        BackupDbKind::Mongo => {
            let (user, pw) = mongo_creds(vars)?;
            Ok(DatabaseSanitySpec {
                cmd: vec![
                    "sh".into(),
                    "-c".into(),
                    "exec \"$(command -v mongosh || command -v mongo)\" --quiet --authenticationDatabase=admin --host=127.0.0.1 --port=27017 --username=\"$OPSBOX_MONGO_USER\" --password=\"$OPSBOX_MONGO_PASS\" --eval \"db.getSiblingDB('admin').runCommand({ ping: 1 }).ok\"".into(),
                ],
                env: vec![
                    format!("OPSBOX_MONGO_USER={user}"),
                    format!("OPSBOX_MONGO_PASS={pw}"),
                ],
                scrub_secrets,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_dump_specs_without_secrets_on_argv() {
        let pg = build_dump_spec(
            BackupDbKind::Postgres,
            "pg-main",
            &[
                ("POSTGRES_USER".into(), "app".into()),
                ("POSTGRES_DB".into(), "shop".into()),
                ("POSTGRES_PASSWORD".into(), "s3cr3t!".into()),
            ],
        )
        .unwrap();
        assert_eq!(pg.cmd, ["pg_dump", "-Fc", "-U", "app", "-d", "shop"]);
        assert_eq!(pg.env, ["PGPASSWORD=s3cr3t!"]);
        assert_eq!(pg.stdin_filename, "pg-main.pg_dump.fc");
        assert!(!pg.cmd.join(" ").contains("s3cr3t!"));

        let mysql = build_dump_spec(
            BackupDbKind::Mysql,
            "my-db",
            &[
                ("MYSQL_DATABASE".into(), "app".into()),
                ("MYSQL_ROOT_PASSWORD".into(), "rootpw".into()),
            ],
        )
        .unwrap();
        assert!(mysql.cmd.contains(&"--single-transaction".to_owned()));
        assert_eq!(mysql.env, ["MYSQL_PWD=rootpw"]);
        assert!(!mysql.cmd.join(" ").contains("rootpw"));

        let maria = build_dump_spec(
            BackupDbKind::Mariadb,
            "maria",
            &[
                ("MARIADB_DATABASE".into(), "app".into()),
                ("MARIADB_ROOT_PASSWORD".into(), "mariapw".into()),
            ],
        )
        .unwrap();
        assert!(maria.cmd[2].contains("mariadb-dump"));
        assert!(maria.cmd[2].contains("--single-transaction"));
        assert_eq!(maria.env, ["MYSQL_PWD=mariapw"]);
        assert!(!maria.cmd.join(" ").contains("mariapw"));

        let redis = build_dump_spec(
            BackupDbKind::Redis,
            "cache",
            &[("REDIS_PASSWORD".into(), "redispw".into())],
        )
        .unwrap();
        assert_eq!(redis.cmd, ["redis-cli", "--no-auth-warning", "--rdb", "-"]);
        assert_eq!(redis.env, ["REDISCLI_AUTH=redispw"]);

        let mongo = build_dump_spec(
            BackupDbKind::Mongo,
            "docdb",
            &[
                ("MONGO_INITDB_ROOT_USERNAME".into(), "app".into()),
                ("MONGO_INITDB_ROOT_PASSWORD".into(), "mongopw".into()),
            ],
        )
        .unwrap();
        assert!(mongo.cmd[2].contains("mongodump --archive"));
        assert!(!mongo.cmd.join(" ").contains("mongopw"));
        assert!(mongo.env.contains(&"OPSBOX_MONGO_PASS=mongopw".to_owned()));
    }

    #[test]
    fn rejects_invalid_db_identifiers() {
        assert!(
            build_dump_spec(
                BackupDbKind::Postgres,
                "pg-main",
                &[("POSTGRES_DB".into(), "bad;drop".into())],
            )
            .is_err()
        );
    }

    #[test]
    fn builds_restore_and_sanity_specs_without_secrets_on_argv() {
        let pg_vars = vec![
            ("POSTGRES_USER".into(), "app".into()),
            ("POSTGRES_DB".into(), "shop".into()),
            ("POSTGRES_PASSWORD".into(), "s3cr3t!".into()),
        ];
        let pg_rst = build_restore_spec(BackupDbKind::Postgres, "pg-main", &pg_vars).unwrap();
        match pg_rst.method {
            DatabaseRestoreMethod::StdinExec { cmd, env } => {
                assert!(cmd.contains(&"pg_restore".to_owned()));
                assert!(cmd.contains(&"--clean".to_owned()));
                assert_eq!(env, ["PGPASSWORD=s3cr3t!"]);
                assert!(!cmd.join(" ").contains("s3cr3t!"));
            }
            DatabaseRestoreMethod::RedisRdbVolume => panic!("expected StdinExec"),
        }
        let pg_check = build_sanity_check_spec(BackupDbKind::Postgres, &pg_vars).unwrap();
        assert!(pg_check.cmd.contains(&"psql".to_owned()));
        assert_eq!(pg_check.env, ["PGPASSWORD=s3cr3t!"]);
        assert!(!pg_check.cmd.join(" ").contains("s3cr3t!"));

        let redis_rst = build_restore_spec(
            BackupDbKind::Redis,
            "cache",
            &[("REDIS_PASSWORD".into(), "rpw".into())],
        )
        .unwrap();
        assert_eq!(redis_rst.method, DatabaseRestoreMethod::RedisRdbVolume);

        let mongo_vars = vec![
            ("MONGO_INITDB_ROOT_USERNAME".into(), "app".into()),
            ("MONGO_INITDB_ROOT_PASSWORD".into(), "mpw".into()),
        ];
        let mongo_rst = build_restore_spec(BackupDbKind::Mongo, "docdb", &mongo_vars).unwrap();
        match mongo_rst.method {
            DatabaseRestoreMethod::StdinExec { cmd, env } => {
                assert!(cmd[2].contains("mongorestore --archive --drop"));
                assert!(!cmd.join(" ").contains("mpw"));
                assert!(env.contains(&"OPSBOX_MONGO_PASS=mpw".to_owned()));
            }
            DatabaseRestoreMethod::RedisRdbVolume => panic!("expected StdinExec"),
        }
        let mongo_chk = build_sanity_check_spec(BackupDbKind::Mongo, &mongo_vars).unwrap();
        assert!(!mongo_chk.cmd.join(" ").contains("mpw"));
        assert!(mongo_chk.env.contains(&"OPSBOX_MONGO_PASS=mpw".to_owned()));
    }
}
