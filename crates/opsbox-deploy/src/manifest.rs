//! GitHub App manifest construction and input validation (docs/BACKEND_SPEC.md 7.5 step 1).

use serde::Serialize;

use crate::DeployError;

const APP_NAME_MAX: usize = 34;

/// Normalize and validate an optional GitHub organization login.
pub fn validate_org(raw: Option<&str>) -> Result<Option<String>, DeployError> {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if s.len() > 39
        || s.starts_with('-')
        || s.ends_with('-')
        || s.contains("--")
        || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(DeployError::Validation {
            field: Some("org"),
            message: "Organization name may contain only letters, numbers, and single dashes."
                .into(),
        });
    }
    Ok(Some(s.to_owned()))
}

/// Generate a URL-safe 256-bit random hex string (for manifest state or fallback webhook secret).
pub fn random_hex(num_bytes: usize) -> Result<String, DeployError> {
    let mut buf = vec![0u8; num_bytes];
    getrandom::fill(&mut buf).map_err(|_| DeployError::Internal("RNG failure".into()))?;
    let mut out = String::with_capacity(num_bytes * 2);
    for b in buf {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    Ok(out)
}

/// Sanitize the host name into a valid GitHub App name (`<= 34` chars).
pub fn app_name_for_host(hostname: &str) -> String {
    let mut suffix = String::new();
    let mut prev_dash = false;
    for ch in hostname.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            suffix.push(lower);
            prev_dash = false;
        } else if !prev_dash && !suffix.is_empty() {
            suffix.push('-');
            prev_dash = true;
        }
    }
    let suffix = suffix.trim_matches('-');
    let raw = if suffix.is_empty() {
        "opsbox-panel".to_owned()
    } else {
        format!("opsbox-{suffix}")
    };
    raw.chars()
        .take(APP_NAME_MAX)
        .collect::<String>()
        .trim_end_matches('-')
        .to_owned()
}

#[derive(Debug, Clone, Serialize)]
pub struct HookAttributes {
    pub url: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppPermissions {
    pub contents: &'static str,
    pub metadata: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitHubManifest {
    pub name: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_attributes: Option<HookAttributes>,
    pub redirect_url: String,
    pub callback_urls: Vec<String>,
    pub setup_url: String,
    pub setup_on_update: bool,
    pub public: bool,
    pub default_permissions: AppPermissions,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub default_events: Vec<&'static str>,
}

#[derive(Debug, Clone)]
pub struct ManifestStart {
    pub action_url: String,
    pub manifest_json: String,
    pub state: String,
    pub webhook_configured: bool,
    pub webhook_url: Option<String>,
    pub panel_domain_required: bool,
}

/// Build the GitHub App manifest form submission target + JSON payload.
pub fn build_manifest(
    web_base: &str,
    panel_origin: &str,
    panel_domain: Option<&str>,
    hostname: &str,
    org: Option<&str>,
    state: &str,
) -> Result<ManifestStart, DeployError> {
    let web = web_base.trim_end_matches('/');
    let origin = panel_origin.trim_end_matches('/');
    let redirect_url = format!("{origin}/api/v1/github/callback");
    let setup_url = format!("{origin}/api/v1/github/setup");

    let (hook_attributes, default_events, webhook_url, webhook_configured, panel_domain_required) =
        if let Some(domain) = panel_domain.filter(|d| !d.is_empty()) {
            let hook_url = format!("https://{domain}/api/v1/github/webhook");
            (
                Some(HookAttributes {
                    url: hook_url.clone(),
                    active: true,
                }),
                vec!["push"],
                Some(hook_url),
                true,
                false,
            )
        } else {
            (None, Vec::new(), None, false, true)
        };

    // GitHub requires `url` to be an HTTP/HTTPS URL; prefer the panel domain when set.
    let homepage_url = if let Some(domain) = panel_domain.filter(|d| !d.is_empty()) {
        format!("https://{domain}")
    } else if origin.starts_with("https://") || origin.starts_with("http://") {
        origin.to_owned()
    } else {
        "https://github.com/khangdev20/opsbox".to_owned()
    };

    let manifest = GitHubManifest {
        name: app_name_for_host(hostname),
        url: homepage_url,
        hook_attributes,
        redirect_url: redirect_url.clone(),
        callback_urls: vec![redirect_url],
        setup_url,
        setup_on_update: true,
        public: false,
        default_permissions: AppPermissions {
            contents: "read",
            metadata: "read",
        },
        default_events,
    };
    let manifest_json = serde_json::to_string(&manifest)
        .map_err(|e| DeployError::Internal(format!("manifest serialization failed: {e}")))?;

    let action_url = match org {
        Some(o) => format!("{web}/organizations/{o}/settings/apps/new?state={state}"),
        None => format!("{web}/settings/apps/new?state={state}"),
    };

    Ok(ManifestStart {
        action_url,
        manifest_json,
        state: state.to_owned(),
        webhook_configured,
        webhook_url,
        panel_domain_required,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn org_validation_accepts_valid_and_rejects_bad() {
        assert_eq!(validate_org(None).unwrap(), None);
        assert_eq!(validate_org(Some("  ")).unwrap(), None);
        assert_eq!(
            validate_org(Some("acme-co")).unwrap(),
            Some("acme-co".into())
        );
        for bad in [
            "-acme",
            "acme-",
            "acme--co",
            "acme/evil",
            "a".repeat(40).as_str(),
        ] {
            assert!(validate_org(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn app_name_stays_within_github_limit() {
        assert_eq!(app_name_for_host("srv-01"), "opsbox-srv-01");
        let long = app_name_for_host("very-long-production-host-name-in-ap-southeast");
        assert!(long.len() <= 34);
        assert!(!long.ends_with('-'));
        assert_eq!(app_name_for_host("---"), "opsbox-panel");
    }

    #[test]
    fn manifest_includes_webhook_only_with_panel_domain() {
        let with_dom = build_manifest(
            "https://github.com",
            "https://panel.example.com",
            Some("panel.example.com"),
            "srv-01",
            Some("acme-co"),
            "state123",
        )
        .unwrap();
        assert_eq!(
            with_dom.action_url,
            "https://github.com/organizations/acme-co/settings/apps/new?state=state123"
        );
        assert!(with_dom.webhook_configured);
        assert!(!with_dom.panel_domain_required);
        let v: serde_json::Value = serde_json::from_str(&with_dom.manifest_json).unwrap();
        assert_eq!(
            v["hook_attributes"]["url"],
            "https://panel.example.com/api/v1/github/webhook"
        );
        assert_eq!(v["default_events"], serde_json::json!(["push"]));
        assert_eq!(
            v["redirect_url"],
            "https://panel.example.com/api/v1/github/callback"
        );
        assert_eq!(v["public"], false);

        let no_dom = build_manifest(
            "https://github.com",
            "https://127.0.0.1:8443",
            None,
            "srv-01",
            None,
            "state456",
        )
        .unwrap();
        assert_eq!(
            no_dom.action_url,
            "https://github.com/settings/apps/new?state=state456"
        );
        assert!(!no_dom.webhook_configured);
        assert!(no_dom.panel_domain_required);
        let v2: serde_json::Value = serde_json::from_str(&no_dom.manifest_json).unwrap();
        assert!(v2.get("hook_attributes").is_none());
        assert!(v2.get("default_events").is_none());
    }
}
