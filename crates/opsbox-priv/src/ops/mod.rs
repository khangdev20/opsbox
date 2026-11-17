//! One module per privileged operation. Adding an operation means a new
//! `PrivOp` variant in `opsbox-proto`, a module here, and one line in [`dispatch`];
//! the compiler enforces the last two because the `match` is exhaustive.

mod access;
pub mod cleanup;
pub(crate) mod cron;
mod doctor;
pub(crate) mod exec;
mod facts;
pub(crate) mod files;
mod firewall;
mod host_info;
mod hostset;
mod jobs;
mod journal;
mod metrics;
mod ping;
mod pkg;
mod security;
pub(crate) mod terminal;
mod totp;
mod units;
mod update;

pub use firewall::{janitor_tick, restore_on_start};

pub use totp::{base32_encode, code_at, verify_at};

use opsbox_proto::{PrivError, PrivOp, PrivReply};

use crate::secrets::Secrets;

/// What operations need besides their arguments.
pub(crate) struct Env {
    pub secrets: Secrets,
    pub master_key: std::path::PathBuf,
    pub files: files::Dirs,
}

/// Blocking; the server runs it on a blocking thread.
pub(crate) fn dispatch(op: &PrivOp, env: &Env) -> Result<PrivReply, PrivError> {
    let secrets = &env.secrets;
    match op {
        PrivOp::ConsoleNotice { text } => {
            use std::io::Write;
            if text.expose().len() > 4096
                || !text.expose().starts_with("opsbox first-run setup")
                || text
                    .expose()
                    .bytes()
                    .any(|b| b < 32 && b != b'\n' && b != b'\t')
            {
                return Err(PrivError {
                    code: opsbox_proto::PrivErrorCode::Invalid,
                    message: "Invalid setup console notice.".into(),
                });
            }
            let mut console = std::fs::OpenOptions::new()
                .write(true)
                .open("/dev/console")
                .map_err(|_| PrivError {
                    code: opsbox_proto::PrivErrorCode::Internal,
                    message: "System console is unavailable.".into(),
                })?;
            console
                .write_all(text.expose().as_bytes())
                .map_err(|_| PrivError {
                    code: opsbox_proto::PrivErrorCode::Internal,
                    message: "Cannot write system console.".into(),
                })?;
            Ok(PrivReply::Done)
        }
        PrivOp::Ping => Ok(ping::run()),
        PrivOp::HostInfo => host_info::run().map(PrivReply::HostInfo),
        PrivOp::Doctor => Ok(PrivReply::Doctor(doctor::collect(&env.master_key))),
        PrivOp::MetricsSample { procs } => metrics::run(*procs),
        PrivOp::SystemFacts => Ok(PrivReply::SystemFacts(facts::collect())),
        PrivOp::PkgList => pkg::list().map(PrivReply::Pkgs),
        PrivOp::PkgInstall { which } => pkg::install(*which).map(PrivReply::Outcome),
        PrivOp::PkgInstallSelected { names } => {
            pkg::install_selected(names).map(PrivReply::Outcome)
        }
        PrivOp::AutoUpdatesSet { on } => pkg::set_auto(*on).map(|()| PrivReply::Done),
        PrivOp::UnitList => units::list().map(PrivReply::Units),
        PrivOp::UnitAction { name, action } => {
            units::action(name, *action).map(|()| PrivReply::Done)
        }
        PrivOp::JournalQuery {
            unit,
            max_priority,
            text,
            limit,
            after,
        } => journal::query(
            unit.as_deref(),
            *max_priority,
            text.as_deref(),
            *limit,
            after.as_deref(),
        )
        .map(PrivReply::Journal),
        PrivOp::JobList => jobs::list().map(PrivReply::Jobs),
        PrivOp::JobAdd {
            name,
            cron,
            command,
        } => jobs::add(name, cron, command).map(PrivReply::JobId),
        PrivOp::JobRun { id } => jobs::run_now(id).map(|()| PrivReply::Done),
        PrivOp::JobRemove { id } => jobs::remove(id).map(|()| PrivReply::Done),
        PrivOp::FirewallList => firewall::list().map(PrivReply::Firewall),
        PrivOp::FirewallAdd {
            rule,
            caller_ip,
            confirm,
        } => firewall::add(rule, caller_ip.as_deref(), *confirm).map(PrivReply::FirewallChange),
        PrivOp::FirewallRemove {
            id,
            caller_ip,
            confirm,
        } => firewall::remove(id, caller_ip.as_deref(), *confirm).map(PrivReply::FirewallChange),
        PrivOp::FirewallConfirm => firewall::confirm().map(|()| PrivReply::Done),
        PrivOp::SshEmergency { caller_ip } => firewall::ssh_emergency(caller_ip).map(|until| {
            PrivReply::FirewallChange(opsbox_proto::FirewallChange {
                revert_at: Some(until),
            })
        }),
        PrivOp::Reboot { delay_min } => hostset::reboot(*delay_min).map(|()| PrivReply::Done),
        PrivOp::RebootCancel => hostset::reboot_cancel().map(|()| PrivReply::Done),
        PrivOp::SwapSet { size_mb } => hostset::set_swap(*size_mb).map(|()| PrivReply::Done),
        PrivOp::HostnameSet { name } => hostset::set_hostname(name).map(|()| PrivReply::Done),
        PrivOp::TimezoneSet { zone } => hostset::set_timezone(zone).map(|()| PrivReply::Done),
        PrivOp::FileList { path, jail } => {
            files::list(path, jail.as_deref()).map(PrivReply::Listing)
        }
        PrivOp::FileRead { path, jail } => {
            files::read(path, jail.as_deref()).map(PrivReply::FileContent)
        }
        PrivOp::FileWrite {
            path,
            content,
            expect_mtime,
            jail,
        } => files::write(path, content, *expect_mtime, jail.as_deref())
            .map(|mtime| PrivReply::Written { mtime }),
        PrivOp::FileChmod { path, mode, jail } => {
            files::chmod(path, mode, jail.as_deref()).map(|()| PrivReply::Done)
        }
        PrivOp::FileMkdir { path, jail } => {
            files::mkdir(path, jail.as_deref()).map(|()| PrivReply::Done)
        }
        PrivOp::FileRemove {
            paths,
            permanent,
            jail,
        } => files::remove(paths, *permanent, jail.as_deref(), &env.files).map(PrivReply::Removed),
        PrivOp::FileStageDownload { path, jail } => {
            files::stage_download(path, jail.as_deref(), &env.files).map(PrivReply::Staged)
        }
        PrivOp::FileInstallUpload {
            staging,
            dir,
            name,
            overwrite,
            jail,
        } => files::install_upload(staging, dir, name, *overwrite, jail.as_deref(), &env.files)
            .map(|()| PrivReply::Done),
        PrivOp::CleanupScan => Ok(cleanup::scan()),
        PrivOp::CleanupRun { ids } => cleanup::run_ids(ids),
        PrivOp::CleanupAutoSet { on } => cleanup::set_auto(*on).map(|()| PrivReply::Done),
        PrivOp::SettingsGet => Ok(PrivReply::Settings(hostset::settings())),
        PrivOp::SelfUpdateInstall {
            staging,
            version,
            target_schema,
        } => {
            update::install(staging, version, *target_schema, &env.files).map(|()| PrivReply::Done)
        }
        PrivOp::SelfUpdateRollback { to_version } => {
            update::rollback(to_version.as_deref()).map(|()| PrivReply::Done)
        }
        PrivOp::SecurityScan { apps_dir } => Ok(security::scan(apps_dir, &env.master_key)),
        PrivOp::SecurityFix { id, apps_dir } => {
            security::fix(id, apps_dir).map(|()| PrivReply::Done)
        }
        PrivOp::AccessList => access::list(),
        PrivOp::SshKeyAdd { user, key } => {
            access::add_key(user, key).map(|fingerprint| PrivReply::KeyAdded { fingerprint })
        }
        PrivOp::SshKeyRemove { user, fingerprint } => {
            access::remove_key(user, fingerprint).map(PrivReply::KeysRemoved)
        }
        PrivOp::UserLock { name, lock } => access::set_lock(name, *lock).map(|()| PrivReply::Done),
        PrivOp::UserList => terminal::list(),
        // A PTY turns the whole connection into a stream, which only the server can do.
        PrivOp::PtyOpen { .. } => Err(PrivError {
            code: opsbox_proto::PrivErrorCode::Internal,
            message: "A terminal can only be opened on its own connection.".into(),
        }),
        PrivOp::TotpNewSecret { context } => totp::new_secret(secrets, context),
        PrivOp::TotpVerify {
            sealed,
            context,
            code,
            after_step,
        } => totp::verify(secrets, sealed, context, code.expose(), *after_step),
    }
}
