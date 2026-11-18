//! One module per privileged operation. Adding an operation means a new
//! `PrivOp` variant in `opsbox-proto`, a module here, and one line in [`dispatch`];
//! the compiler enforces the last two because the `match` is exhaustive.

mod access;
mod adoption;
pub(crate) mod backup;
pub(crate) mod capacity;
pub mod cleanup;
pub(crate) mod cron;
pub(crate) mod database;
pub(crate) mod deploy;
pub mod docker;
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
mod network;
mod ping;
mod pkg;
mod sealed;
mod security;
pub(crate) mod templates_compose;
pub(crate) mod terminal;
mod totp;
mod units;
mod update;
mod watchdog;
pub mod web;

pub use firewall::{janitor_tick, restore_on_start};

pub use totp::{base32_encode, code_at, verify_at};

use opsbox_proto::{PrivError, PrivOp, PrivReply};

use crate::secrets::Secrets;

/// What operations need besides their arguments.
pub(crate) struct Env {
    pub capacity: capacity::State,
    pub capacity_image_size_mb: Option<u64>,
    pub capacity_profile: Option<opsbox_capacity::Profile>,
    pub secrets: Secrets,
    pub master_key: std::path::PathBuf,
    pub config_path: std::path::PathBuf,
    pub files: files::Dirs,
    pub apps_dir: std::path::PathBuf,
    pub db_path: std::path::PathBuf,
    pub backup_dir: std::path::PathBuf,
    pub backup_cache_dir: std::path::PathBuf,
    pub restic_bin: Option<std::path::PathBuf>,
    pub docker: opsbox_docker::DockerClient,
    pub docker_bin: Option<std::path::PathBuf>,
    pub allow_test_git: bool,
    pub network_proc_root: std::path::PathBuf,
    pub network_systemctl: Option<std::path::PathBuf>,
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
        PrivOp::PortHolders { .. }
        | PrivOp::WebDiscover { .. }
        | PrivOp::WebMove { .. }
        | PrivOp::WebUndo { .. }
        | PrivOp::WebMoves
        | PrivOp::WebTls { .. }
        | PrivOp::ProxyNetwork
        | PrivOp::ProxyTakeover { .. }
        | PrivOp::ProxyUndo { .. }
        | PrivOp::ProxyKeepPorts { .. } => network::dispatch(op, env),
        PrivOp::WorkloadMemory {
            container,
            service,
            memory_mb,
        } => docker::block_on_fut(
            env.docker
                .set_workload_memory(container, service, *memory_mb),
        )?
        .map_err(|e| docker::map_docker_err(e, env.docker.socket_path()))
        .map(|()| PrivReply::Done),
        PrivOp::CapacityImageSize { image } => {
            capacity::image_size(env, image).map(|disk_mb| PrivReply::CapacityImageSize { disk_mb })
        }
        PrivOp::Capacity => capacity::collect(env).map(|s| PrivReply::Capacity(Box::new(s))),
        PrivOp::CapacityAddSwap { size_mb } => {
            capacity::add_swap(env, *size_mb).map(|()| PrivReply::Done)
        }
        PrivOp::Ping => Ok(ping::run()),
        PrivOp::HostInfo => host_info::run().map(PrivReply::HostInfo),
        PrivOp::Doctor => Ok(PrivReply::Doctor(doctor::collect_with(
            &env.master_key,
            &env.docker,
        ))),
        PrivOp::MetricsSample { procs } => metrics::run(*procs),
        PrivOp::SystemFacts => Ok(PrivReply::SystemFacts(facts::collect_with(&env.docker))),
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
        PrivOp::CleanupScan => Ok(cleanup::scan_with(false, &env.docker)),
        PrivOp::CleanupScanFresh => Ok(cleanup::scan_with(true, &env.docker)),
        PrivOp::CleanupVolumeSizes => cleanup::volume_sizes_with(&env.docker)
            .map(|(bytes, count)| PrivReply::CleanupVolumeSize { bytes, count }),
        PrivOp::CleanupRun { ids } => cleanup::run_ids_with(ids, &env.docker),
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
        PrivOp::WatchdogStatus => Ok(PrivReply::WatchdogStatus(Box::new(watchdog::status()))),
        PrivOp::TotpNewSecret { context } => totp::new_secret(secrets, context),
        PrivOp::TotpVerify {
            sealed,
            context,
            code,
            after_step,
        } => totp::verify(secrets, sealed, context, code.expose(), *after_step),
        PrivOp::SecretSeal { context, plaintext } => sealed::seal(secrets, context, plaintext),
        PrivOp::SecretOpen { context, sealed } => sealed::open(secrets, context, sealed),
        PrivOp::WebLocalOnly { local_only } => {
            web::set_local_only(&env.config_path, *local_only).map(|()| PrivReply::Done)
        }
        PrivOp::DockerStatus => Ok(PrivReply::DockerStatus(docker::status_with(&env.docker))),
        PrivOp::DockerInstall => docker::install().map(PrivReply::Outcome),
        PrivOp::DockerListContainers => {
            docker::list_containers_with(&env.docker).map(PrivReply::DockerContainers)
        }
        PrivOp::DockerInspect { id } => {
            docker::inspect_with(&env.docker, id).map(|i| PrivReply::DockerInspect(Box::new(i)))
        }
        PrivOp::DockerContainerAction { id, action } => {
            docker::action_with(&env.docker, id, *action).map(|()| PrivReply::Done)
        }
        PrivOp::DockerLogs { id, tail } => {
            docker::logs_with(&env.docker, id, *tail).map(PrivReply::DockerLogs)
        }
        PrivOp::DockerLogsFollow { .. } => Err(PrivError {
            code: opsbox_proto::PrivErrorCode::Internal,
            message: "Container log streaming can only be opened on its own connection.".into(),
        }),
        PrivOp::DockerStats => docker::stats_with(&env.docker).map(PrivReply::DockerStats),
        PrivOp::DockerExec(req) => docker::exec_with(&env.docker, req).map(PrivReply::DockerExec),
        PrivOp::DockerListImages => {
            docker::list_images_with(&env.docker).map(PrivReply::DockerImages)
        }
        PrivOp::DockerListVolumes => {
            docker::list_volumes_with(&env.docker).map(PrivReply::DockerVolumes)
        }
        PrivOp::DockerListNetworks => {
            docker::list_networks_with(&env.docker).map(PrivReply::DockerNetworks)
        }
        PrivOp::DeployEnsureDir { service } => {
            deploy::ensure_service_dir(&env.apps_dir, service).map(|_| PrivReply::Done)
        }
        PrivOp::DeployBuild(_) => Err(PrivError {
            code: opsbox_proto::PrivErrorCode::Internal,
            message: "Deploy builds stream on their own connection.".into(),
        }),
        PrivOp::DeployPullImage(req) => deploy::pull_image_with(env, req).map(|()| PrivReply::Done),
        PrivOp::DeployStartAndCheck(req) => {
            deploy::start_and_check_with(env, req).map(PrivReply::DeployStarted)
        }
        PrivOp::DeployStopContainer { container_id } => {
            deploy::stop_container_with(env, container_id).map(|()| PrivReply::Done)
        }
        PrivOp::DeployImageExists { image_ref } => {
            deploy::image_exists_with(env, image_ref).map(PrivReply::Bool)
        }
        PrivOp::DeployPruneImages {
            service,
            image_refs,
        } => deploy::prune_images_with(env, service, image_refs).map(PrivReply::PrunedImages),
        PrivOp::DeployDeleteService {
            service,
            container_ids,
            image_refs,
        } => deploy::delete_service_with(env, service, container_ids, image_refs)
            .map(|()| PrivReply::Done),
        PrivOp::TemplateStart(req) => {
            templates_compose::start_template_with(env, req).map(PrivReply::DeployStarted)
        }
        PrivOp::DatabaseRotate(req) => {
            templates_compose::rotate_database_with(env, req).map(PrivReply::DatabaseRotated)
        }
        PrivOp::DatabaseInspect(req) => database::inspect_database_with(env, req)
            .map(|r| PrivReply::DatabaseInspected(Box::new(r))),
        PrivOp::DatabaseManage(req) => {
            database::manage_database_with(env, req).map(PrivReply::DatabaseManaged)
        }
        PrivOp::DatabasePooler(req) => {
            database::pooler_database_with(env, req).map(PrivReply::DatabasePooler)
        }
        PrivOp::DatabaseUpgrade(req) => database::upgrade_database_with(env, req)
            .map(|r| PrivReply::DatabaseUpgraded(Box::new(r))),
        PrivOp::ComposeValidate(req) => {
            templates_compose::validate_compose_with(env, req).map(PrivReply::ComposeValidated)
        }
        PrivOp::ComposeDeploy(req) => {
            templates_compose::deploy_compose_with(env, req).map(PrivReply::ComposeDeployed)
        }
        PrivOp::ComposeDown {
            stack,
            remove_files,
        } => templates_compose::down_compose_with(env, stack, *remove_files)
            .map(|()| PrivReply::Done),
        PrivOp::ComposeRemoveService { stack, service } => {
            templates_compose::remove_compose_service_with(env, stack, service)
                .map(|()| PrivReply::Done)
        }
        PrivOp::BackupLocalInfo { check_paths } => backup::local_info_with(env, check_paths)
            .map(|v| PrivReply::BackupLocalInfo(Box::new(v))),
        PrivOp::BackupRun(req) => {
            backup::run_backup_with(env, req.clone()).map(|v| PrivReply::BackupRun(Box::new(v)))
        }
        PrivOp::BackupSnapshots(req) => {
            backup::list_snapshots_with(env, req.clone()).map(PrivReply::BackupSnapshots)
        }
        PrivOp::BackupForget(req) => {
            backup::forget_with(env, req.clone()).map(|v| PrivReply::BackupForget(Box::new(v)))
        }
        PrivOp::BackupCheck(req) => {
            backup::check_with(env, req.clone()).map(|v| PrivReply::BackupCheck(Box::new(v)))
        }
        PrivOp::BackupTestDestination(req) => {
            backup::test_destination_with(env, req.clone()).map(PrivReply::BackupTestDestination)
        }
        PrivOp::BackupGenerateSftpKey(req) => {
            backup::generate_sftp_key_with(env, req).map(PrivReply::BackupSftpKey)
        }
        PrivOp::BackupRecoveryKey => {
            backup::recovery_key_with(env).map(PrivReply::BackupRecoveryKey)
        }
        PrivOp::BackupRestore(req) => {
            backup::restore_with(env, req.clone()).map(|v| PrivReply::BackupRestore(Box::new(v)))
        }
        PrivOp::BackupRestoreTest(req) => backup::restore_test_with(env, req.clone())
            .map(|v| PrivReply::BackupRestoreTest(Box::new(v))),
    }
}
