//! Caller policy at the authenticated helper connection boundary.

use opsbox_proto::{PrivErrorCode, PrivOp, PrivRequest, PrivResponse};

/// Derived from the accepting listener after kernel peer UID verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Caller {
    Serve,
    Proxy,
}

/// Every new PrivOp needs an explicit caller decision here.
pub(crate) fn allowed(caller: Caller, op: &PrivOp) -> bool {
    let proxy_allowed = match op {
        PrivOp::PortHolders { .. } => true,
        PrivOp::SecretSeal { context, .. } | PrivOp::SecretOpen { context, .. } => {
            context.starts_with("proxy:")
        }
        PrivOp::Ping
        | PrivOp::Capacity
        | PrivOp::CapacityImageSize { .. }
        | PrivOp::WorkloadMemory { .. }
        | PrivOp::CapacityAddSwap { .. }
        | PrivOp::ProxyNetwork
        | PrivOp::WebDiscover { .. }
        | PrivOp::WebMove { .. }
        | PrivOp::WebUndo { .. }
        | PrivOp::WebMoves
        | PrivOp::WebTls { .. }
        | PrivOp::ProxyTakeover { .. }
        | PrivOp::ProxyUndo { .. }
        | PrivOp::ProxyKeepPorts { .. }
        | PrivOp::HostInfo
        | PrivOp::ConsoleNotice { .. }
        | PrivOp::Doctor
        | PrivOp::TotpNewSecret { .. }
        | PrivOp::TotpVerify { .. }
        | PrivOp::WebLocalOnly { .. }
        | PrivOp::MetricsSample { .. }
        | PrivOp::SystemFacts
        | PrivOp::PkgList
        | PrivOp::PkgInstall { .. }
        | PrivOp::PkgInstallSelected { .. }
        | PrivOp::AutoUpdatesSet { .. }
        | PrivOp::UnitList
        | PrivOp::UnitAction { .. }
        | PrivOp::JournalQuery { .. }
        | PrivOp::JobList
        | PrivOp::JobAdd { .. }
        | PrivOp::JobRun { .. }
        | PrivOp::JobRemove { .. }
        | PrivOp::FirewallList
        | PrivOp::FirewallAdd { .. }
        | PrivOp::FirewallRemove { .. }
        | PrivOp::FirewallConfirm
        | PrivOp::SshEmergency { .. }
        | PrivOp::Reboot { .. }
        | PrivOp::RebootCancel
        | PrivOp::SwapSet { .. }
        | PrivOp::DiskState
        | PrivOp::DiskGrow { .. }
        | PrivOp::HostnameSet { .. }
        | PrivOp::TimezoneSet { .. }
        | PrivOp::FileList { .. }
        | PrivOp::FileRead { .. }
        | PrivOp::FileWrite { .. }
        | PrivOp::FileChmod { .. }
        | PrivOp::FileMkdir { .. }
        | PrivOp::FileRemove { .. }
        | PrivOp::FileStageDownload { .. }
        | PrivOp::FileInstallUpload { .. }
        | PrivOp::CleanupScan
        | PrivOp::CleanupScanFresh
        | PrivOp::CleanupVolumeSizes
        | PrivOp::CleanupRun { .. }
        | PrivOp::CleanupAutoSet { .. }
        | PrivOp::SettingsGet
        | PrivOp::SelfUpdateInstall { .. }
        | PrivOp::SelfUpdateRollback { .. }
        | PrivOp::SecurityScan { .. }
        | PrivOp::SecurityFix { .. }
        | PrivOp::AccessList
        | PrivOp::SshKeyAdd { .. }
        | PrivOp::SshKeyRemove { .. }
        | PrivOp::UserLock { .. }
        | PrivOp::UserList
        | PrivOp::PtyOpen { .. }
        | PrivOp::WatchdogStatus
        | PrivOp::DockerStatus
        | PrivOp::DockerInstall
        | PrivOp::DockerListContainers
        | PrivOp::DockerInspect { .. }
        | PrivOp::DockerContainerAction { .. }
        | PrivOp::DockerLogs { .. }
        | PrivOp::DockerLogsFollow { .. }
        | PrivOp::DockerStats
        | PrivOp::DockerExec { .. }
        | PrivOp::DockerListImages
        | PrivOp::DockerListVolumes
        | PrivOp::DockerListNetworks
        | PrivOp::DeployEnsureDir { .. }
        | PrivOp::DeployBuild { .. }
        | PrivOp::DeployPullImage { .. }
        | PrivOp::DeployStartAndCheck { .. }
        | PrivOp::DeployStopContainer { .. }
        | PrivOp::DeployImageExists { .. }
        | PrivOp::DeployPruneImages { .. }
        | PrivOp::DeployDeleteService { .. }
        | PrivOp::TemplateStart(_)
        | PrivOp::DatabaseRotate(_)
        | PrivOp::DatabaseInspect(_)
        | PrivOp::DatabaseManage(_)
        | PrivOp::DatabasePooler(_)
        | PrivOp::DatabaseUpgrade(_)
        | PrivOp::ComposeValidate(_)
        | PrivOp::ComposeDeploy(_)
        | PrivOp::ComposeDown { .. }
        | PrivOp::ComposeRemoveService { .. }
        | PrivOp::BackupLocalInfo { .. }
        | PrivOp::BackupRun(_)
        | PrivOp::BackupSnapshots(_)
        | PrivOp::BackupForget(_)
        | PrivOp::BackupCheck(_)
        | PrivOp::BackupTestDestination(_)
        | PrivOp::BackupGenerateSftpKey(_)
        | PrivOp::BackupRecoveryKey
        | PrivOp::BackupRestore(_)
        | PrivOp::BackupRestoreTest(_)
        | PrivOp::UninstallPlan { .. }
        | PrivOp::Uninstall { .. } => false,
    };
    match caller {
        Caller::Serve => true,
        Caller::Proxy => proxy_allowed,
    }
}

/// Refuse before any blocking or streaming operation starts. Do not log payloads
/// or the untrusted actor string; credentials come from the kernel.
pub(crate) fn authorize(
    caller: Caller,
    request: &PrivRequest,
    uid: u32,
    pid: Option<i32>,
) -> Result<(), Box<PrivResponse>> {
    if allowed(caller, &request.op) {
        return Ok(());
    }
    tracing::warn!(
        event = "priv_op_forbidden",
        ?caller,
        uid,
        ?pid,
        request_id = %request.id,
        op = request.op.name(),
        code = ?PrivErrorCode::Forbidden,
        "privileged operation denied"
    );
    Err(Box::new(PrivResponse::failure(
        request.id.clone(),
        PrivErrorCode::Forbidden,
        "This caller may not perform this privileged operation.",
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use opsbox_proto::{PrivErrorCode, PrivOp, PrivRequest, PrivResult, Secret};

    fn restricted_ops() -> Vec<PrivOp> {
        vec![
            PrivOp::Ping,
            PrivOp::HostInfo,
            PrivOp::JobAdd {
                name: "test".into(),
                cron: "0 2 * * *".into(),
                command: "true".into(),
            },
            PrivOp::PtyOpen {
                user: "root".into(),
                cols: 80,
                rows: 24,
            },
            PrivOp::Reboot { delay_min: 0 },
            PrivOp::FirewallRemove {
                panel_port: None,
                id: "test".into(),
                caller_ip: None,
                confirm: true,
            },
            PrivOp::FirewallConfirm,
            PrivOp::FileWrite {
                path: "/etc/opsbox/config.toml".into(),
                content: "secret-payload".into(),
                expect_mtime: None,
                jail: None,
            },
            PrivOp::SelfUpdateInstall {
                staging: "/tmp/update".into(),
                version: "test".into(),
                target_schema: 1,
            },
            PrivOp::DockerLogsFollow {
                id: "test".into(),
                tail: 1,
            },
            PrivOp::DeployBuild(opsbox_proto::deploy::DeployBuildRequest {
                service: "test".into(),
                deploy_id: "test".into(),
                source_kind: opsbox_proto::deploy::DeployGitSource::GitUrl,
                repo_url: "invalid".into(),
                branch: "main".into(),
                commit_sha: None,
                build_method: "dockerfile".into(),
                dockerfile_path: None,
                port: 8080,
                github_token: None,
                deploy_key_enc: None,
                deploy_key_context: None,
                image_tag: "test".into(),
                cache_from: None,
                vars: vec![],
                ref_sources: vec![],
                timeout_secs: 1,
                memory_limit_mb: None,
            }),
            PrivOp::DockerInstall,
            PrivOp::ProxyNetwork,
            PrivOp::ProxyKeepPorts { confirm: true },
            PrivOp::WebMoves,
            PrivOp::BackupRecoveryKey,
            PrivOp::Uninstall { purge: true },
            PrivOp::TotpNewSecret {
                context: "totp:admin".into(),
            },
        ]
    }

    #[test]
    fn proxy_only_gets_port_holders_and_proxy_secrets() {
        assert!(allowed(
            Caller::Proxy,
            &PrivOp::PortHolders {
                ports: vec![80, 443]
            }
        ));
        for context in ["proxy:cert:panel.example.com", "proxy:acme_account:prod"] {
            for op in [
                PrivOp::SecretSeal {
                    context: context.into(),
                    plaintext: Secret::new("key"),
                },
                PrivOp::SecretOpen {
                    context: context.into(),
                    sealed: "enc:v1:test".into(),
                },
            ] {
                assert!(allowed(Caller::Proxy, &op), "{}", op.name());
                assert!(allowed(Caller::Serve, &op));
            }
        }
        for op in restricted_ops() {
            assert!(!allowed(Caller::Proxy, &op), "{}", op.name());
            assert!(allowed(Caller::Serve, &op), "{}", op.name());
        }
    }

    #[test]
    fn proxy_cannot_seal_or_open_other_namespaces() {
        for context in [
            "",
            "proxy",
            "xproxy:cert:a",
            "deploy:key",
            "backup:dest:a",
            "alerts:channel:a",
            "totp:admin",
        ] {
            for op in [
                PrivOp::SecretSeal {
                    context: context.into(),
                    plaintext: Secret::new("secret-payload"),
                },
                PrivOp::SecretOpen {
                    context: context.into(),
                    sealed: "secret-payload".into(),
                },
            ] {
                assert!(!allowed(Caller::Proxy, &op), "{context}");
                assert!(allowed(Caller::Serve, &op));
                let request = PrivRequest {
                    id: "namespace".into(),
                    actor: None,
                    op,
                };
                let response = authorize(Caller::Proxy, &request, 1234, None).unwrap_err();
                assert!(
                    matches!(response.result, PrivResult::Failure { error } if error.code == PrivErrorCode::Forbidden)
                );
            }
        }
    }

    #[test]
    fn denied_proxy_requests_return_typed_forbidden_with_request_id() {
        for op in restricted_ops() {
            let request = PrivRequest {
                id: "fix-27".into(),
                actor: Some("root".into()),
                op,
            };
            let response = authorize(Caller::Proxy, &request, 1234, Some(5678)).unwrap_err();
            assert_eq!(response.id, "fix-27");
            let PrivResult::Failure { error } = response.result else {
                panic!("expected failure")
            };
            assert_eq!(
                error.code,
                PrivErrorCode::Forbidden,
                "{}",
                request.op.name()
            );
            assert!(!error.message.contains("secret-payload"));
            assert!(authorize(Caller::Serve, &request, 1234, Some(5678)).is_ok());
        }
    }

    #[test]
    fn denied_operations_log_kernel_identity_and_no_payload_or_actor() {
        use std::sync::{Arc, Mutex};
        use tracing::{
            Event, Metadata, Subscriber,
            field::{Field, Visit},
            span::{Attributes, Id, Record},
        };

        struct Capture(Arc<Mutex<String>>);
        struct Fields(String);
        impl Visit for Fields {
            fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                use std::fmt::Write;
                write!(self.0, " {}={value:?}", field.name()).unwrap();
            }
        }
        impl Subscriber for Capture {
            fn enabled(&self, _: &Metadata<'_>) -> bool {
                true
            }
            fn new_span(&self, _: &Attributes<'_>) -> Id {
                Id::from_u64(1)
            }
            fn record(&self, _: &Id, _: &Record<'_>) {}
            fn record_follows_from(&self, _: &Id, _: &Id) {}
            fn enter(&self, _: &Id) {}
            fn exit(&self, _: &Id) {}
            fn event(&self, event: &Event<'_>) {
                let mut fields = Fields(event.metadata().level().to_string());
                event.record(&mut fields);
                self.0.lock().unwrap().push_str(&fields.0);
            }
        }
        let captured = Arc::new(Mutex::new(String::new()));
        tracing::subscriber::with_default(Capture(Arc::clone(&captured)), || {
            let request = PrivRequest {
                id: "audit-27".into(),
                actor: Some("forged-admin".into()),
                op: PrivOp::SecretOpen {
                    context: "deploy:key".into(),
                    sealed: "secret-payload".into(),
                },
            };
            assert!(authorize(Caller::Proxy, &request, 1234, Some(5678)).is_err());
        });
        let logged = captured.lock().unwrap();
        for field in [
            "WARN",
            "priv_op_forbidden",
            "caller=Proxy",
            "uid=1234",
            "pid=Some(5678)",
            "request_id=audit-27",
            "secret_open",
            "Forbidden",
        ] {
            assert!(logged.contains(field), "missing {field}: {logged}");
        }
        for secret in ["forged-admin", "secret-payload", "deploy:key"] {
            assert!(!logged.contains(secret), "leaked {secret}: {logged}");
        }
    }
}
