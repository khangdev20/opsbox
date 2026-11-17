//! Docker Compose YAML parsing, pre-validation, safety policy checks, and graph metadata
//! extraction (docs/BACKEND_SPEC.md 7.6).

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use opsbox_proto::{ComposeServiceInfo, valid_compose_service_name, valid_service_name};
use serde_json::{Map, Value};
use yaml_rust2::{Yaml, YamlLoader};

use crate::DeployError;

pub const MAX_COMPOSE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposePrecheck {
    pub doc: Value,
    pub inferred_stack_name: Option<String>,
    pub services: Vec<ComposeServiceInfo>,
    pub depends_edges: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComposeRiskReport {
    pub reasons: Vec<String>,
}

impl ComposeRiskReport {
    pub fn is_safe(&self) -> bool {
        self.reasons.is_empty()
    }

    pub fn summary_message(&self) -> String {
        format!(
            "This Compose file requests elevated access that needs your confirmation: {}. Continue only if you trust this Compose file.",
            self.reasons.join("; ")
        )
    }
}

fn val_err(field: &'static str, msg: impl Into<String>) -> DeployError {
    DeployError::Validation {
        field: Some(field),
        message: msg.into(),
    }
}

fn yaml_to_json(node: &Yaml, depth: usize) -> Result<Value, DeployError> {
    if depth > 48 {
        return Err(val_err(
            "yaml",
            "Compose YAML is nested too deeply (maximum depth 48).",
        ));
    }
    Ok(match node {
        Yaml::Null | Yaml::BadValue => Value::Null,
        Yaml::Boolean(b) => Value::Bool(*b),
        Yaml::Integer(i) => Value::Number((*i).into()),
        Yaml::Real(s) => s
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map_or_else(|| Value::String(s.clone()), Value::Number),
        Yaml::String(s) => Value::String(s.clone()),
        Yaml::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(yaml_to_json(item, depth + 1)?);
            }
            Value::Array(out)
        }
        Yaml::Hash(map) => {
            let mut out = Map::new();
            for (k, v) in map {
                let key = match k {
                    Yaml::String(s) => s.clone(),
                    Yaml::Integer(i) => i.to_string(),
                    Yaml::Boolean(b) => b.to_string(),
                    Yaml::Real(r) => r.clone(),
                    _ => {
                        return Err(val_err(
                            "yaml",
                            "Compose YAML mappings must use string keys.",
                        ));
                    }
                };
                out.insert(key, yaml_to_json(v, depth + 1)?);
            }
            Value::Object(out)
        }
        Yaml::Alias(_) => {
            return Err(val_err(
                "yaml",
                "Unresolved YAML alias found in Compose file.",
            ));
        }
    })
}

/// Parse raw Compose YAML text into a JSON [`Value`], enforcing size and document limits.
pub fn parse_compose_yaml(yaml_text: &str) -> Result<Value, DeployError> {
    if yaml_text.trim().is_empty() {
        return Err(val_err("yaml", "Paste a Docker Compose YAML file."));
    }
    if yaml_text.len() > MAX_COMPOSE_BYTES {
        return Err(val_err(
            "yaml",
            "Compose file is too large (maximum 256 KB).",
        ));
    }
    if yaml_text.contains('\0') {
        return Err(val_err("yaml", "Compose file contains invalid NUL bytes."));
    }
    let docs = YamlLoader::load_from_str(yaml_text).map_err(|e| {
        val_err(
            "yaml",
            format!("This does not look like valid Compose YAML: {e}"),
        )
    })?;
    if docs.len() != 1 {
        return Err(val_err(
            "yaml",
            "Compose file must contain a single YAML document.",
        ));
    }
    yaml_to_json(&docs[0], 0)
}

/// Check that a file reference (`env_file`, `extends.file`, `build.context`, `secrets.*.file`)
/// stays strictly inside the Compose stack directory before `docker compose config` reads disk.
fn check_contained_ref_path(raw: &str, kind: &str, service_dir: &Path) -> Result<(), DeployError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(val_err(
            "yaml",
            format!("Empty {kind} path in Compose file."),
        ));
    }
    if trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("git://")
        || trimmed.starts_with("ssh://")
    {
        return Ok(());
    }
    if trimmed.starts_with('~') {
        return Err(val_err(
            "yaml",
            format!("{kind} path '{trimmed}' cannot reference home directories."),
        ));
    }
    if is_path_inside_service_dir(trimmed, service_dir).is_none() {
        return Err(val_err(
            "yaml",
            format!("{kind} path '{trimmed}' escapes the service directory and was refused."),
        ));
    }
    Ok(())
}

/// Resolve `raw` relative to `service_dir`, returning `Some(normalized)` iff it stays strictly
/// inside `service_dir` (and has no `~` or symlink-outside escape on existing ancestors).
pub fn is_path_inside_service_dir(raw: &str, service_dir: &Path) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with('~') || trimmed.contains('\0') {
        return None;
    }
    let candidate = Path::new(trimmed);
    let base = if candidate.is_absolute() {
        PathBuf::new()
    } else {
        normalize_lexical(service_dir)?
    };
    let mut out = base;
    for comp in candidate.components() {
        match comp {
            Component::RootDir => out.push("/"),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::Normal(seg) => out.push(seg),
            Component::Prefix(_) => return None,
        }
    }

    let norm_root = normalize_lexical(service_dir)?;
    if !(out == norm_root || out.starts_with(&norm_root)) {
        return None;
    }

    // If the service directory already exists on disk, also guard against symlinks escaping it.
    if let Ok(canon_root) = std::fs::canonicalize(service_dir) {
        let mut probe = out.clone();
        while !probe.exists() {
            if !probe.pop() {
                break;
            }
        }
        if probe.exists()
            && let Ok(canon_existing) = std::fs::canonicalize(&probe)
            && !(canon_existing == canon_root || canon_existing.starts_with(&canon_root))
        {
            return None;
        }
    }

    Some(out)
}

fn normalize_lexical(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::RootDir => out.push("/"),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::Normal(s) => out.push(s),
            Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

/// Pre-validate raw Compose YAML before handing it to `docker compose config`.
///
/// Refuses invalid schemas, empty `services:`, bad service names, and directives that could
/// make `docker compose config` read host files outside `service_dir`.
pub fn precheck_compose_yaml(
    yaml_text: &str,
    service_dir: &Path,
) -> Result<ComposePrecheck, DeployError> {
    let doc = parse_compose_yaml(yaml_text)?;
    let Some(root) = doc.as_object() else {
        return Err(val_err(
            "yaml",
            "This does not look like a Compose file: top level must be a mapping.",
        ));
    };

    if root.contains_key("include") {
        return Err(val_err(
            "yaml",
            "Top-level 'include' directives are not allowed in pasted Compose files.",
        ));
    }

    for section in ["secrets", "configs"] {
        if let Some(map) = root.get(section).and_then(Value::as_object) {
            for (_, item) in map {
                if let Some(file_path) = item.get("file").and_then(Value::as_str) {
                    check_contained_ref_path(file_path, section, service_dir)?;
                }
            }
        }
    }

    let Some(services_map) = root.get("services").and_then(Value::as_object) else {
        return Err(val_err(
            "yaml",
            "This does not look like a Compose file: it needs a 'services:' section.",
        ));
    };
    if services_map.is_empty() {
        return Err(val_err(
            "yaml",
            "Compose file must define at least one service under 'services:'.",
        ));
    }

    for (svc_name, svc_val) in services_map {
        if !valid_compose_service_name(svc_name) {
            return Err(val_err(
                "yaml",
                format!(
                    "Compose service name '{svc_name}' must use lowercase letters, digits, dashes, or underscores."
                ),
            ));
        }
        let Some(svc_obj) = svc_val.as_object() else {
            return Err(val_err(
                "yaml",
                format!("Compose service '{svc_name}' must be a mapping."),
            ));
        };

        // Guard pre-config file reads (`env_file`, `extends`, `build.context`/`dockerfile`).
        if let Some(env_file) = svc_obj.get("env_file") {
            match env_file {
                Value::String(p) => check_contained_ref_path(p, "env_file", service_dir)?,
                Value::Array(arr) => {
                    for entry in arr {
                        if let Some(p) = entry.as_str() {
                            check_contained_ref_path(p, "env_file", service_dir)?;
                        } else if let Some(p) = entry.get("path").and_then(Value::as_str) {
                            check_contained_ref_path(p, "env_file", service_dir)?;
                        }
                    }
                }
                _ => {}
            }
        }

        if let Some(extends) = svc_obj.get("extends")
            && let Some(file_p) = extends.get("file").and_then(Value::as_str)
        {
            check_contained_ref_path(file_p, "extends.file", service_dir)?;
        }

        if let Some(build) = svc_obj.get("build") {
            match build {
                Value::String(ctx) => check_contained_ref_path(ctx, "build", service_dir)?,
                Value::Object(bobj) => {
                    if let Some(ctx) = bobj.get("context").and_then(Value::as_str) {
                        check_contained_ref_path(ctx, "build.context", service_dir)?;
                    }
                    if let Some(df) = bobj.get("dockerfile").and_then(Value::as_str) {
                        check_contained_ref_path(df, "build.dockerfile", service_dir)?;
                    }
                    if let Some(add) = bobj.get("additional_contexts").and_then(Value::as_object) {
                        for (_, v) in add {
                            if let Some(p) = v.as_str() {
                                check_contained_ref_path(
                                    p,
                                    "build.additional_contexts",
                                    service_dir,
                                )?;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let inferred_stack_name = root
        .get("name")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| valid_service_name(s));

    let (services, depends_edges) = extract_compose_services(&doc)?;
    Ok(ComposePrecheck {
        doc,
        inferred_stack_name,
        services,
        depends_edges,
    })
}

fn parse_port_spec(val: &Value) -> Option<u16> {
    match val {
        Value::Number(n) => n.as_u64().and_then(|u| u16::try_from(u).ok()),
        Value::String(s) => {
            let clean = s.split('/').next().unwrap_or(s).trim();
            // e.g. "127.0.0.1:3000:80", "3000:80", "3000"
            let target = clean.rsplit(':').next()?.trim();
            target.split('-').next()?.trim().parse::<u16>().ok()
        }
        Value::Object(obj) => obj
            .get("target")
            .and_then(|v| match v {
                Value::Number(n) => n.as_u64().and_then(|u| u16::try_from(u).ok()),
                Value::String(s) => s.trim().parse::<u16>().ok(),
                _ => None,
            })
            .filter(|p| *p > 0),
        _ => None,
    }
}

fn extract_depends_on(svc_obj: &Map<String, Value>) -> Vec<String> {
    let mut deps = Vec::new();
    let Some(raw) = svc_obj.get("depends_on") else {
        return deps;
    };
    match raw {
        Value::Array(arr) => {
            for item in arr {
                if let Some(s) = item.as_str() {
                    deps.push(s.to_owned());
                }
            }
        }
        Value::Object(map) => {
            for k in map.keys() {
                deps.push(k.clone());
            }
        }
        _ => {}
    }
    deps
}

/// `(services, depends_on_edges)` extracted from a Compose document.
pub type ExtractedComposeServices = (Vec<ComposeServiceInfo>, Vec<(String, String)>);

/// Extract service metadata and dependency edges from a Compose JSON/YAML value.
pub fn extract_compose_services(doc: &Value) -> Result<ExtractedComposeServices, DeployError> {
    let Some(services_map) = doc.get("services").and_then(Value::as_object) else {
        return Err(val_err(
            "yaml",
            "Compose configuration contains no 'services' mapping.",
        ));
    };
    if services_map.is_empty() {
        return Err(val_err(
            "yaml",
            "Compose configuration defines zero services.",
        ));
    }

    let mut services = Vec::with_capacity(services_map.len());
    let mut edges = Vec::new();
    for (name, val) in services_map {
        let Some(obj) = val.as_object() else {
            continue;
        };
        let image = obj.get("image").and_then(Value::as_str).map(str::to_owned);
        let port = obj
            .get("ports")
            .and_then(Value::as_array)
            .and_then(|arr| arr.iter().find_map(parse_port_spec))
            .unwrap_or(0);
        let depends_on = extract_depends_on(obj);
        for dep in &depends_on {
            edges.push((name.clone(), dep.clone()));
        }
        let host_network = obj
            .get("network_mode")
            .and_then(Value::as_str)
            .is_some_and(|m| m.trim().eq_ignore_ascii_case("host"));
        services.push(ComposeServiceInfo {
            name: name.clone(),
            image,
            port,
            depends_on,
            host_network,
        });
    }
    Ok((services, edges))
}

fn is_truthy(val: &Value) -> bool {
    match val {
        Value::Bool(b) => *b,
        Value::String(s) => matches!(s.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes"),
        _ => false,
    }
}

fn looks_like_host_bind_source(src: &str) -> bool {
    src.starts_with('/')
        || src.starts_with('.')
        || src.starts_with('~')
        || src.contains('/')
        || src.contains('\\')
}

fn split_short_volume_spec(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Single segment without ':' is an anonymous container volume (e.g. "/app/cache").
    let first_colon = trimmed.find(':')?;
    let source = trimmed[..first_colon].trim();
    if source.is_empty() {
        return None;
    }
    Some(source)
}

/// Inspect a Compose YAML/JSON document for elevated privileges or host access:
/// - `privileged: true`
/// - `network_mode: host` (or top-level network `driver: host`)
/// - bind mounts outside `service_dir`
pub fn inspect_compose_risks(doc: &Value, service_dir: &Path) -> ComposeRiskReport {
    let mut reasons = BTreeSet::new();

    if let Some(networks) = doc.get("networks").and_then(Value::as_object) {
        for (net_name, net_val) in networks {
            if net_val
                .get("driver")
                .and_then(Value::as_str)
                .is_some_and(|d| d.trim().eq_ignore_ascii_case("host"))
            {
                reasons.insert(format!("network '{net_name}' uses host network driver"));
            }
        }
    }

    if let Some(volumes) = doc.get("volumes").and_then(Value::as_object) {
        for (vol_name, vol_val) in volumes {
            if let Some(opts) = vol_val.get("driver_opts").and_then(Value::as_object) {
                let is_bind_opt = opts
                    .get("o")
                    .and_then(Value::as_str)
                    .is_some_and(|o| o.split(',').any(|p| p.trim().eq_ignore_ascii_case("bind")));
                if is_bind_opt
                    && let Some(device) = opts.get("device").and_then(Value::as_str)
                    && is_path_inside_service_dir(device, service_dir).is_none()
                {
                    reasons.insert(format!(
                        "volume '{vol_name}' binds host path '{device}' outside the service directory"
                    ));
                }
            }
        }
    }

    if let Some(services) = doc.get("services").and_then(Value::as_object) {
        for (svc_name, svc_val) in services {
            let Some(obj) = svc_val.as_object() else {
                continue;
            };
            if obj.get("privileged").is_some_and(is_truthy) {
                reasons.insert(format!("service '{svc_name}' runs in privileged mode"));
            }
            if obj
                .get("network_mode")
                .and_then(Value::as_str)
                .is_some_and(|m| m.trim().eq_ignore_ascii_case("host"))
            {
                reasons.insert(format!("service '{svc_name}' uses host networking"));
            }
            if let Some(vols) = obj.get("volumes").and_then(Value::as_array) {
                for v in vols {
                    match v {
                        Value::String(spec) => {
                            if let Some(source) = split_short_volume_spec(spec)
                                && looks_like_host_bind_source(source)
                                && is_path_inside_service_dir(source, service_dir).is_none()
                            {
                                reasons.insert(format!(
                                    "service '{svc_name}' mounts host path '{source}' outside the service directory"
                                ));
                            }
                        }
                        Value::Object(vobj) => {
                            let typ = vobj
                                .get("type")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_ascii_lowercase();
                            let source = vobj.get("source").and_then(Value::as_str).unwrap_or("");
                            if (typ == "bind"
                                || (!source.is_empty() && looks_like_host_bind_source(source)))
                                && is_path_inside_service_dir(source, service_dir).is_none()
                            {
                                reasons.insert(format!(
                                    "service '{svc_name}' mounts host path '{source}' outside the service directory"
                                ));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    ComposeRiskReport {
        reasons: reasons.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_safe_compose_and_contained_relative_binds() {
        let svc_dir = Path::new("/var/lib/opsbox/apps/shop");
        let yaml = r#"
services:
  web:
    image: nginx:1.27-alpine
    ports: ["127.0.0.1:3000:80"]
    volumes:
      - ./data:/usr/share/nginx/html
      - /var/lib/opsbox/apps/shop/uploads:/uploads:ro
      - cachedata:/cache
    depends_on:
      - cache
  cache:
    image: redis:7-alpine
    volumes:
      - cachedata:/data
volumes:
  cachedata:
"#;
        let pre = precheck_compose_yaml(yaml, svc_dir).unwrap();
        assert_eq!(pre.services.len(), 2);
        assert_eq!(pre.depends_edges, vec![("web".into(), "cache".into())]);
        let web = pre.services.iter().find(|s| s.name == "web").unwrap();
        assert_eq!(web.port, 80);
        assert!(inspect_compose_risks(&pre.doc, svc_dir).is_safe());
    }

    #[test]
    fn flags_privileged_host_network_and_outside_bind_mounts() {
        let svc_dir = Path::new("/var/lib/opsbox/apps/shop");
        let yaml = r#"
services:
  web:
    image: nginx:1.27-alpine
    privileged: true
    network_mode: host
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
      - ../other:/mnt/other
      - type: bind
        source: /etc
        target: /host-etc
"#;
        let pre = precheck_compose_yaml(yaml, svc_dir).unwrap();
        let report = inspect_compose_risks(&pre.doc, svc_dir);
        assert!(!report.is_safe());
        let msg = report.summary_message();
        assert!(msg.contains("privileged mode"), "{msg}");
        assert!(msg.contains("host networking"), "{msg}");
        assert!(msg.contains("/var/run/docker.sock"), "{msg}");
        assert!(msg.contains("../other"), "{msg}");
        assert!(msg.contains("/etc"), "{msg}");
    }

    #[test]
    fn refuses_include_and_escaping_env_file_before_compose_config() {
        let svc_dir = Path::new("/var/lib/opsbox/apps/shop");
        assert!(
            precheck_compose_yaml(
                "include:\n  - /etc/passwd\nservices:\n  a:\n    image: alpine\n",
                svc_dir
            )
            .is_err()
        );
        assert!(
            precheck_compose_yaml(
                "services:\n  a:\n    image: alpine\n    env_file: /etc/shadow\n",
                svc_dir
            )
            .is_err()
        );
        assert!(
            precheck_compose_yaml(
                "services:\n  a:\n    image: alpine\n    env_file: ../../secret.env\n",
                svc_dir
            )
            .is_err()
        );
    }
}
