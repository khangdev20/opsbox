use std::sync::Arc;

use anyhow::{Context, Result};
use opsbox_api::{AppState, SecureCookies};
use opsbox_auth::AuthConfig;
use opsbox_priv::PrivClient;
use opsbox_store::{ActorKind, AuditResult, NewAudit, Store};
use tokio::net::TcpListener;
use tracing::{info, warn};

use crate::VERSION;
use crate::config::{Config, SecureCookiesSetting};

pub fn run(config: Config) -> Result<()> {
    // Two workers are plenty for a loopback control plane and keep idle memory low.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("cannot start the async runtime")?;
    runtime.block_on(serve(config))
}

async fn serve(config: Config) -> Result<()> {
    let store = Arc::new(Store::open(&config.db_path).with_context(|| {
        format!(
            "cannot open the database at {} (pass --db to use another path)",
            config.db_path.display()
        )
    })?);
    let schema = store.schema_version()?;
    store.append_audit(NewAudit {
        actor: "opsbox".into(),
        actor_kind: ActorKind::System,
        ip: None,
        action: "system.start".into(),
        target: Some("serve".into()),
        result: AuditResult::Ok,
        detail: serde_json::json!({ "version": VERSION, "schema_version": schema }),
    })?;

    let listener = TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("cannot listen on {}", config.listen))?;
    let addr = listener.local_addr()?;

    let (cert_path, key_path) = {
        let sys_cert = std::path::Path::new(opsbox_api::tls::DEFAULT_CERT_PATH);
        let sys_key = std::path::Path::new(opsbox_api::tls::DEFAULT_KEY_PATH);
        if sys_cert.exists() && sys_key.exists() {
            (sys_cert.to_path_buf(), sys_key.to_path_buf())
        } else if let Ok(pair) = opsbox_api::tls::ensure_self_signed_cert(sys_cert, sys_key) {
            pair
        } else {
            let db_dir = config
                .db_path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("/var/lib/opsbox"));
            opsbox_api::tls::ensure_self_signed_cert(
                &db_dir.join("panel-selfsigned.crt"),
                &db_dir.join("panel-selfsigned.key"),
            )
            .context("cannot generate self-signed TLS certificate")?
        }
    };
    let acceptor = opsbox_api::tls::load_tls_acceptor(&cert_path, &key_path)
        .context("cannot load TLS certificate")?;

    info!(
        version = VERSION,
        %addr,
        tls = true,
        cert = %cert_path.display(),
        db = %config.db_path.display(),
        schema_version = schema,
        priv_socket = %config.priv_socket.display(),
        config_file = ?config.loaded_from,
        "opsbox serve started"
    );

    let mut auth_cfg = AuthConfig::default();
    auth_cfg.hash_params.memory_kib = config.argon2_memory_kib;
    auth_cfg.hash_params.iterations = config.argon2_iterations;
    let secure = match config.secure_cookies {
        SecureCookiesSetting::Auto => SecureCookies::Auto,
        SecureCookiesSetting::Always => SecureCookies::Always,
        SecureCookiesSetting::Never => SecureCookies::Never,
    };
    let state = AppState::new(
        store,
        PrivClient::new(&config.priv_socket),
        VERSION,
        secure,
        auth_cfg,
    )
    .with_files(opsbox_api::FilesSettings {
        apps_dir: config.apps_dir.clone(),
        xfer_dir: config.xfer_dir.clone(),
        max_upload: config.max_upload_mb.saturating_mul(1 << 20),
    })
    .with_listen(config.listen)
    .with_tls(true)
    .with_terminal_idle(std::time::Duration::from_secs(
        config.terminal_idle_min * 60,
    ));

    // First run: make sure there is a setup token and show it where the person who
    // installed opsbox will look: the console and the journal.
    if let Some(token) = state
        .auth
        .ensure_setup_token()
        .context("cannot create the setup token")?
    {
        announce_setup_token(&config, &token).await;
    }

    opsbox_api::tls::serve_tls(listener, acceptor, state, super::shutdown_signal())
        .await
        .context("the HTTPS server failed")?;
    info!("opsbox serve stopped");
    Ok(())
}

/// Write the setup banner to standard error (the journal) and, if possible, to the
/// system console, as docs/BACKEND_SPEC.md 6.3 asks.
async fn announce_setup_token(config: &Config, token: &str) {
    let banner = super::admin::setup_banner(config, token);
    eprint!("{banner}");
    if let Err(e) = PrivClient::new(&config.priv_socket)
        .call(
            "setup-console",
            None,
            opsbox_proto::PrivOp::ConsoleNotice {
                text: opsbox_proto::Secret::new(banner),
            },
        )
        .await
    {
        warn!(error = %e, "setup notice remains available in the service journal");
    }
}
