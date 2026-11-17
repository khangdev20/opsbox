//! `opsbox web local-only on|off` and `opsbox web domain <name>`.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use opsbox_priv::web_ops::replace_listen_in_toml;
use opsbox_proxy::{ProxyClient, Route, UpstreamTls};
use opsbox_store::{ActorKind, AuditResult, NewAudit, Store};

use crate::cli::Toggle;
use crate::config::{Config, DEFAULT_CONFIG_PATH};

pub fn local_only(config_path: Option<&Path>, mode: Toggle) -> Result<()> {
    let path = config_path.unwrap_or_else(|| Path::new(DEFAULT_CONFIG_PATH));
    let target_listen = match mode {
        Toggle::On => "127.0.0.1:8443",
        Toggle::Off => "0.0.0.0:8443",
    };

    let existing = fs::read_to_string(path).unwrap_or_default();
    let updated = replace_listen_in_toml(&existing, target_listen);
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
        .args(["try-restart", "opsbox.service", "opsbox-watchdog.service"])
        .status();

    match mode {
        Toggle::On => println!("opsbox now listens on 127.0.0.1:8443 (local-only)."),
        Toggle::Off => println!("opsbox now listens on 0.0.0.0:8443 (public HTTPS)."),
    }
    Ok(())
}

pub fn domain(config: &Config, name: &str, skip_dns_check: bool) -> Result<()> {
    let domain = opsbox_proxy::normalize_domain(name).map_err(|e| anyhow::anyhow!("{e}"))?;
    super::admin::drop_to_service_user()?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start the async runtime")?;

    let server_ips = opsbox_proxy::detect_public_ips(&config.public_ips);
    let dns = rt
        .block_on(opsbox_proxy::check_dns(&domain, &server_ips))
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("{}", dns.message);
    if !dns.points_here {
        for rec in &dns.records {
            println!(
                "  add record: {}  {}  ->  {}",
                rec.record_type, rec.name, rec.value
            );
        }
        if !skip_dns_check {
            bail!(
                "DNS is not pointing to this server yet. Add the record above and retry (or pass --skip-dns-check)."
            );
        }
    }

    let store = Store::open(&config.db_path).with_context(|| {
        format!(
            "cannot open the database at {} (run as root/opsbox or pass --db)",
            config.db_path.display()
        )
    })?;
    if store
        .get_service_domain(&domain)
        .context("cannot query service domains")?
        .is_some()
    {
        bail!(
            "Domain '{domain}' is already attached to a service. Remove it from the service first."
        );
    }

    let cert_sha256 = {
        let sys_cert = Path::new(opsbox_api::tls::DEFAULT_CERT_PATH);
        let fallback_cert = config
            .db_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("/var/lib/opsbox"))
            .join("panel-selfsigned.crt");
        opsbox_api::tls::cert_sha256_hex(sys_cert)
            .or_else(|| opsbox_api::tls::cert_sha256_hex(&fallback_cert))
    };

    let upstream = opsbox_api::panel::upstream_addr(config.listen).to_string();
    let route = Route {
        host: domain.clone(),
        upstream,
        upstream_tls: UpstreamTls::Https { cert_sha256 },
        redirect_www: false,
        redirect_to: None,
        redirect_https: true,
        hsts: true,
        max_body_bytes: config.max_upload_mb.saturating_mul(1 << 20),
        is_panel: true,
    };

    let routes = opsbox_api::panel::build_routes_with_panel(&store, Some(route));

    let proxy = ProxyClient::new(&config.proxy_socket);
    rt.block_on(async {
        proxy
            .clone()
            .with_timeout(Duration::from_secs(5))
            .ping("cli-web-domain")
            .await
            .context("opsbox-proxy is not reachable (start opsbox-proxy.service first)")?;

        proxy
            .clone()
            .with_timeout(Duration::from_secs(5))
            .set_routes("cli-web-domain", routes)
            .await
            .context("could not update proxy routing table")?;

        println!("Issuing Let's Encrypt certificate for {domain}...");
        let cert = proxy
            .clone()
            .with_timeout(Duration::from_secs(120))
            .issue_cert("cli-web-domain", &domain)
            .await
            .context("ACME certificate issuance failed")?;
        Ok::<_, anyhow::Error>(cert)
    })?;

    store.set_setting(
        opsbox_api::panel::KEY_PANEL_DOMAIN,
        &serde_json::json!(domain),
    )?;
    let _ = store.delete_setting(opsbox_api::panel::KEY_PANEL_DRAFT);
    store.append_audit(NewAudit {
        actor: super::admin::actor(),
        actor_kind: ActorKind::System,
        ip: None,
        action: "panel.cert.issue".into(),
        target: Some(domain.clone()),
        result: AuditResult::Ok,
        detail: serde_json::json!({ "via": "cli" }),
    })?;

    println!("Panel is live at https://{domain}");
    if !config.listen.ip().is_loopback() {
        println!(
            "Raw port {} is still open with a self-signed certificate. To close it: sudo opsbox web local-only on",
            config.listen.port()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_or_adds_listen_in_toml() {
        let empty = "";
        assert_eq!(
            replace_listen_in_toml(empty, "127.0.0.1:8443"),
            "[api]\nlisten = \"127.0.0.1:8443\"\n"
        );

        let with_api = "[api]\nlisten = \"0.0.0.0:8443\"\n\n[store]\n";
        assert_eq!(
            replace_listen_in_toml(with_api, "127.0.0.1:8443"),
            "[api]\nlisten = \"127.0.0.1:8443\"\n\n[store]\n"
        );
    }
}
