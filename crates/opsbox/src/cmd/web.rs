//! `opsbox web local-only on|off`

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use crate::cli::Toggle;
use crate::config::DEFAULT_CONFIG_PATH;

pub fn local_only(config_path: Option<&Path>, mode: Toggle) -> Result<()> {
    let path = config_path.unwrap_or_else(|| Path::new(DEFAULT_CONFIG_PATH));
    let target_listen = match mode {
        Toggle::On => "127.0.0.1:8443",
        Toggle::Off => "0.0.0.0:8443",
    };

    let existing = fs::read_to_string(path).unwrap_or_default();
    let updated = set_listen_in_toml(&existing, target_listen);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, updated).with_context(|| format!("cannot write {}", path.display()))?;

    if matches!(mode, Toggle::Off) {
        let _ = opsbox_api::tls::ensure_self_signed_cert(
            Path::new(opsbox_api::tls::DEFAULT_CERT_PATH),
            Path::new(opsbox_api::tls::DEFAULT_KEY_PATH),
        );
    }

    let _ = Command::new("systemctl")
        .args(["try-restart", "opsbox.service"])
        .status();

    match mode {
        Toggle::On => println!("opsbox now listens on 127.0.0.1:8443 (local-only)."),
        Toggle::Off => println!("opsbox now listens on 0.0.0.0:8443 (public HTTPS)."),
    }
    Ok(())
}

fn set_listen_in_toml(raw: &str, listen: &str) -> String {
    let mut out = Vec::new();
    let mut in_api = false;
    let mut saw_api = false;
    let mut replaced = false;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_api && !replaced {
                out.push(format!("listen = \"{listen}\""));
                replaced = true;
            }
            in_api = trimmed == "[api]";
            if in_api {
                saw_api = true;
            }
            out.push(line.to_owned());
            continue;
        }
        if in_api
            && !trimmed.starts_with('#')
            && trimmed.starts_with("listen")
            && let Some((k, _)) = trimmed.split_once('=')
            && k.trim() == "listen"
        {
            out.push(format!("listen = \"{listen}\""));
            replaced = true;
            continue;
        }
        out.push(line.to_owned());
    }

    if in_api && !replaced {
        out.push(format!("listen = \"{listen}\""));
        replaced = true;
    }
    if !saw_api && !replaced {
        if !out.is_empty() && !out.last().is_some_and(String::is_empty) {
            out.push(String::new());
        }
        out.push("[api]".to_owned());
        out.push(format!("listen = \"{listen}\""));
    }

    let mut result = out.join("\n");
    result.push('\n');
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_or_adds_listen_in_toml() {
        let empty = "";
        assert_eq!(
            set_listen_in_toml(empty, "127.0.0.1:8443"),
            "[api]\nlisten = \"127.0.0.1:8443\"\n"
        );

        let with_api = "[api]\nlisten = \"0.0.0.0:8443\"\n\n[store]\n";
        assert_eq!(
            set_listen_in_toml(with_api, "127.0.0.1:8443"),
            "[api]\nlisten = \"127.0.0.1:8443\"\n\n[store]\n"
        );
    }
}
