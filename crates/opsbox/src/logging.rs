use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

use crate::config::{Config, LogFormat};

/// Log to stderr, which systemd sends to the journal. Plain text has no colour codes
/// so journal output stays clean.
pub fn init(config: &Config) -> Result<()> {
    let filter = EnvFilter::try_new(&config.log_level)
        .with_context(|| format!("invalid log filter {:?}", config.log_level))?;
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false);
    match config.log_format {
        LogFormat::Text => builder.try_init(),
        LogFormat::Json => builder.json().try_init(),
    }
    .map_err(|e| anyhow::anyhow!("cannot start logging: {e}"))
}
