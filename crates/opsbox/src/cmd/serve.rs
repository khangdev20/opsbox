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
    if !addr.ip().is_loopback() {
        // Phase 0 has no authentication.
        warn!(%addr, "listening on a non-loopback address: there is no authentication yet, anyone who can reach this port can read host facts");
    }

    info!(
        version = VERSION,
        %addr,
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
    );

    // First run: make sure there is a setup token and show it where the person who
    // installed opsbox will look: the console and the journal.
    if let Some(token) = state
        .auth
        .ensure_setup_token()
        .context("cannot create the setup token")?
    {
        announce_setup_token(&config, &token);
    }

    opsbox_api::serve(listener, state, super::shutdown_signal())
        .await
        .context("the HTTP server failed")?;
    info!("opsbox serve stopped");
    Ok(())
}

/// Write the setup banner to standard error (the journal) and, if possible, to the
/// system console, as docs/BACKEND_SPEC.md 6.3 asks.
fn announce_setup_token(config: &Config, token: &str) {
    let banner = super::admin::setup_banner(config, token);
    eprint!("{banner}");
    if let Ok(mut console) = std::fs::OpenOptions::new().write(true).open("/dev/console") {
        use std::io::Write;
        let _ = console.write_all(banner.as_bytes());
    }
}
