pub mod admin;
pub mod install;
pub mod privileged;
pub mod selftest;
pub mod serve;
pub mod status;

use tokio::signal::unix::{SignalKind, signal};
use tracing::info;

/// Completes on SIGINT or SIGTERM (what `systemctl stop` sends).
pub async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    let term = async {
        match signal(SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            // Could not install the handler: rely on Ctrl-C alone.
            Err(_) => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        () = ctrl_c => {},
        () = term => {},
    }
    info!("shutdown signal received");
}
